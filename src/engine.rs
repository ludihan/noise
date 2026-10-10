//! The audio engine: runs the sequencer and the module graph.
//!
//! The engine lives on the audio thread. The UI talks to it through a
//! command channel and reads playback state back from `Shared`.

use crate::dsp::{self, Ctx, Dsp, Frame, Playhead};
use crate::project::{
    Cell, FX_AUTOPAN, FX_BREAK, FX_MAYBE, FX_PHRASE, FX_REVERSE, FX_SLICE, FX_TRACK_VOLUME, FX_TREMOR, FX_WAIT, MACROS,
    MAX_COLUMNS, MAX_FX_COLUMNS, MAX_TRACKS, ModuleKind, Note, OUTPUT_ID, PhraseMode, Project,
};
use crate::sample::Sample;
use std::f32::consts::TAU;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

const BLOCK: usize = dsp::MAX_BLOCK;
/// Frames of each track's scope.
pub const TRACK_SCOPE_LEN: usize = 512;
/// Beats in a bar, for the metronome's accent.
const BEATS_PER_BAR: usize = 4;
/// Frames of output kept for the scope and the spectrum analyzer.
pub const SCOPE_LEN: usize = 4096;
/// Playheads published at most, so the audio thread never allocates for them.
const MAX_PLAYHEADS: usize = 256;
/// Keys at or above this are live keyboard notes rather than tracks.
pub const LIVE_KEY: u32 = 1000;
/// How long the mixer meters take to fall to a third.
const METER_FALL: f32 = 0.3;
/// Automated values published at most.
const MAX_AUTOMATED: usize = 256;
/// Notes a MultiSynth keeps track of at once.
const MAX_HELD: usize = 128;
/// How deep MultiSynths may feed each other.
const MAX_NOTE_DEPTH: u8 = 8;

/// A note event on its way to a module.
#[derive(Clone, Copy, Debug, PartialEq)]
enum NoteEv {
    On(f32, f32),
    Off,
    Pitch(f32),
    Vel(f32),
    Pan(f32),
    Offset(f32),
    /// Play the sample backwards, or forwards again.
    Reverse(bool),
    /// Play this slice of the sample instead.
    Slice(u8),
    Seek(f64),
}

/// A phrase playing on a key of an instrument: which of its phrases, how
/// far the note played moves the phrase and its velocity, the next line,
/// its next tick and how long until it, and the phrase's notes and
/// effects, played as a track plays them.
#[derive(Clone, Copy)]
struct PhrasePlayer {
    module: u8,
    phrase: usize,
    key: u32,
    transpose: f32,
    vel: f32,
    /// The line playing, and the next.
    playing: usize,
    line: usize,
    tick: u32,
    wait: f64,
    track: Track,
}

impl PhrasePlayer {
    /// Passes an event of the phrase's track to the instrument's `dsp`,
    /// moved by the note played and scaled by its velocity.
    fn send(&self, dsp: &mut dyn Dsp, ev: NoteEv) {
        let (key, t, v) = (self.key, self.transpose, self.vel);
        match ev {
            NoteEv::On(note, vel) => dsp.note_on(key, note + t, vel * v),
            NoteEv::Off => dsp.note_off(key),
            NoteEv::Pitch(note) => dsp.set_pitch(key, note + t),
            NoteEv::Vel(vel) => dsp.set_velocity(key, vel * v),
            NoteEv::Pan(pan) => dsp.set_pan(key, pan),
            NoteEv::Offset(pos) => dsp.sample_offset(key, pos),
            NoteEv::Reverse(on) => dsp.reverse(key, on),
            NoteEv::Slice(k) => dsp.play_slice(key, k as usize),
            NoteEv::Seek(frames) => dsp.seek(key, frames),
        }
    }
}

/// Phrases playing at once, at most.
const MAX_PHRASES: usize = 64;

/// A note a MultiSynth passed on: its key, the pitch and velocity changes
/// picked for it, and the one target it went to (`None` for all).
#[derive(Clone, Copy)]
struct Held {
    key: u32,
    detune: f32,
    vel: f32,
    target: Option<usize>,
}

/// A note a Glide passes on: its key, the pitch it is at and the one it
/// slides to, in semitones a second, whether it sounds, and a note-off
/// held back until the tick's events are in, in case a note follows it.
#[derive(Clone, Copy)]
struct GlideVoice {
    key: u32,
    at: f32,
    to: f32,
    rate: f32,
    sounding: bool,
    off_pending: bool,
}

/// The longest stretch rendered while a Glide slides, so its pitch moves
/// smoothly.
const GLIDE_STEP: usize = 64;

pub enum Cmd {
    Project(Arc<Project>),
    /// Start playing at `order` / `line`. With `loop_pattern` the current
    /// pattern repeats instead of following the order list.
    Play {
        order: usize,
        line: usize,
        loop_pattern: bool,
    },
    Stop,
    NoteOn {
        module: u8,
        key: u32,
        note: u8,
        vel: f32,
    },
    NoteOff {
        module: u8,
        key: u32,
    },
    Panic,
    /// Play a sample straight to the output, as the disk browser does when
    /// a file is clicked. `None` stops the preview.
    Preview(Option<Arc<Sample>>),
    /// How loud previews play, and whether they loop until stopped.
    PreviewSettings {
        volume: f32,
        looping: bool,
    },
    /// Click on every beat while playing, higher on the first of a bar.
    Metronome(bool),
    /// Loop lines `from..=to` while the song plays position `order`, a
    /// block loop; `None` turns it off.
    BlockLoop(Option<(usize, usize, usize)>),
}

/// Things the audio thread hands back to the UI thread so they are freed
/// there rather than in the audio callback. The contents are never read.
#[allow(dead_code)]
pub enum Garbage {
    Project(Arc<Project>),
    Sample(Arc<Sample>),
}

/// Frames handed from an audio thread to the UI for recording, without the
/// audio thread waiting or allocating: it adds to a buffer made with room
/// for a few seconds, and the UI empties it every frame, keeping the room.
pub struct Tape {
    frames: Mutex<Vec<Frame>>,
    on: AtomicBool,
    /// Frames were lost: the buffer was full or busy.
    dropped: AtomicBool,
    /// The rate of the sound on it, when the one writing says.
    rate: AtomicU32,
}

impl Default for Tape {
    fn default() -> Self {
        Self {
            frames: Mutex::new(Vec::new()),
            on: AtomicBool::new(false),
            dropped: AtomicBool::new(false),
            rate: AtomicU32::new(0),
        }
    }
}

impl Tape {
    /// Starts taking frames, with room for `room` of them between reads.
    pub fn start(&self, room: usize) {
        *self.frames.lock().unwrap() = Vec::with_capacity(room);
        self.dropped.store(false, Ordering::Relaxed);
        self.on.store(true, Ordering::Release);
    }

    pub fn stop(&self) {
        self.on.store(false, Ordering::Release);
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }

    /// Adds `frames` if the tape is on, as many as there is room for.
    pub fn push(&self, frames: &[Frame]) {
        if !self.is_on() {
            return;
        }
        let Ok(mut buf) = self.frames.try_lock() else {
            self.dropped.store(true, Ordering::Relaxed);
            return;
        };
        let room = buf.capacity() - buf.len();
        if frames.len() > room {
            self.dropped.store(true, Ordering::Relaxed);
        }
        buf.extend_from_slice(&frames[..frames.len().min(room)]);
    }

    pub fn set_rate(&self, rate: u32) {
        self.rate.store(rate, Ordering::Relaxed);
    }

    /// The sound's rate, or 0 if not known.
    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    /// For a reader on another audio thread: moves the oldest frames into
    /// `out`, as many as there are, and returns how many. With more than
    /// `most` waiting, the oldest are dropped first, so the reader stays
    /// close behind the writer. Never waits.
    pub fn read(&self, out: &mut [Frame], most: usize) -> usize {
        let Ok(mut buf) = self.frames.try_lock() else { return 0 };
        if buf.len() > most {
            let late = buf.len() - most;
            buf.drain(..late);
        }
        let n = buf.len().min(out.len());
        out[..n].copy_from_slice(&buf[..n]);
        buf.drain(..n);
        n
    }

    /// Moves the frames taken so far onto `into`, and says whether any
    /// were lost since the tape started.
    pub fn take(&self, into: &mut Vec<Frame>) -> bool {
        into.append(&mut self.frames.lock().unwrap());
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Playback state published for the UI.
pub struct Shared {
    pub playing: AtomicBool,
    pub order: AtomicUsize,
    pub line: AtomicUsize,
    /// How far into that line playback is, 0..1, as f32 bits.
    pub line_frac: AtomicU32,
    pub bpm: AtomicU32,
    pub peak: [AtomicU32; 2],
    pub scope: Mutex<Vec<Frame>>,
    /// Where each sounding sampler voice is.
    pub playheads: Mutex<Vec<Playhead>>,
    /// Position of the disk browser preview in frames, or -1 when silent.
    pub preview_pos: AtomicU32,
    /// Seconds since playback started.
    pub time: AtomicU32,
    /// Share of the real time that rendering takes, 0..1.
    pub cpu: AtomicU32,
    /// Peak level of each module's output after its mixer settings, left
    /// and right, indexed by `2 * id + channel`.
    pub levels: Vec<AtomicU32>,
    /// The values envelopes give parameters while the song plays, as
    /// (module, automatable parameter, value).
    pub automated: Mutex<Vec<(u8, usize, f32)>>,
    /// What each track's notes played lately, before effects, mono:
    /// `TRACK_SCOPE_LEN` frames per track, oldest first.
    pub track_scopes: Mutex<Vec<f32>>,
    /// The phrases playing, as (module, phrase, line playing).
    pub phrases: Mutex<Vec<(u8, usize, usize)>>,
    /// The output, while the sample recorder records the song.
    pub resample: Tape,
    /// The sound card's input, while the song has an Input module.
    pub input: Arc<Tape>,
}

impl Shared {
    /// The peak levels of module `id`.
    pub fn level(&self, id: u8) -> [f32; 2] {
        let at = |ch: usize| f32::from_bits(self.levels[2 * id as usize + ch].load(Ordering::Relaxed));
        [at(0), at(1)]
    }
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            playing: AtomicBool::new(false),
            order: AtomicUsize::new(0),
            line: AtomicUsize::new(0),
            line_frac: AtomicU32::new(0),
            bpm: AtomicU32::new(0),
            peak: [AtomicU32::new(0), AtomicU32::new(0)],
            scope: Mutex::new(vec![[0.0; 2]; SCOPE_LEN]),
            playheads: Mutex::new(Vec::with_capacity(4 * MAX_PLAYHEADS)),
            preview_pos: AtomicU32::new((-1f32).to_bits()),
            time: AtomicU32::new(0),
            cpu: AtomicU32::new(0),
            levels: (0..512).map(|_| AtomicU32::new(0)).collect(),
            automated: Mutex::new(Vec::with_capacity(MAX_AUTOMATED)),
            track_scopes: Mutex::new(vec![0.0; MAX_TRACKS * TRACK_SCOPE_LEN]),
            phrases: Mutex::new(Vec::with_capacity(MAX_PHRASES)),
            resample: Tape::default(),
            input: Arc::default(),
        }
    }
}

struct Node {
    id: u8,
    /// For a copy made for a track with effects of its own, that track.
    track: Option<usize>,
    kind: ModuleKind,
    /// Index into `project.modules`.
    module: usize,
    dsp: Box<dyn Dsp>,
    inputs: Vec<usize>,
    /// For an effect with a key input, the nodes whose sound it listens to:
    /// the module keying it and its copies for tracks.
    key: Vec<usize>,
    buf: Vec<Frame>,
    /// Off while another module is soloed and this one neither feeds it
    /// nor is fed by it.
    audible: bool,
    /// Output peak since the meters were last published.
    peak: [f32; 2],
    /// The module's parameters with the playing pattern's automation
    /// applied, used instead of the song's while `automated`.
    params: Vec<f32>,
    automated: bool,
    /// The mixer fader and pan envelopes set, used instead of the song's.
    mix_gain: Option<f32>,
    mix_pan: Option<f32>,
    /// For an instrument, where envelopes and Modulators turn its macros,
    /// used instead of the song's, and what the macros move: macro, node,
    /// automatable parameter and the range it moves it over.
    macros: [Option<f32>; MACROS],
    macro_targets: Vec<(usize, usize, usize, f32, f32)>,
    /// For a MultiSynth: the nodes it passes notes to, the notes it holds
    /// and the next round-robin target.
    targets: Vec<usize>,
    held: Vec<Held>,
    next_target: usize,
    /// For a Glide: the last note on each key, sliding.
    glides: Vec<GlideVoice>,
    /// For a Modulator: the nodes and automatable parameters it moves,
    /// its LFO's phase and the level it follows.
    controls: Vec<(usize, usize)>,
    phase: f32,
    follow: f32,
    /// The last note an instrument was played, how hard (0..1), and when
    /// (by `Engine::notes_played`), for Modulators tracking it. A Modulator
    /// in Envelope mode keeps the when of the note that started it here.
    last_note: (f32, f32, u64),
}

#[derive(Clone, Copy, Default)]
struct Track {
    module: Option<u8>,
    /// Module currently holding a note on this track.
    sounding: Option<u8>,
    /// The note's pitch and velocity without this line's arpeggio,
    /// vibrato and tremolo, and the pitch and velocity last sent.
    base: f32,
    pitch: f32,
    vel: f32,
    sent_vel: f32,
    /// Panning set with 8xx, -1..1; it stays until changed.
    pan: f32,
    porta_target: Option<f32>,
    porta_speed: f32,
    // Per-line effect state.
    arp: Option<(u8, u8)>,
    slide: f32,
    cut_at: Option<u32>,
    delayed: Option<(u32, Cell, Effects)>,
    /// Vibrato and tremolo speed (cycles per tick) and depth.
    vibrato: Option<(f32, f32)>,
    tremolo: Option<(f32, f32)>,
    vibrato_phase: f32,
    tremolo_phase: f32,
    /// The last speed and depth given, for parameters left at zero.
    vibrato_memory: (u8, u8),
    tremolo_memory: (u8, u8),
    /// Auto-pan: speed (cycles per tick) and depth, its phase, its last
    /// parameters, and the panning last sent.
    autopan: Option<(f32, f32)>,
    autopan_phase: f32,
    autopan_memory: (u8, u8),
    sent_pan: f32,
    /// Velocity added each tick.
    vol_slide: f32,
    /// Play the note again every this many ticks.
    retrigger: Option<u32>,
    /// Txy: ticks on, then off.
    tremor: Option<(u32, u32)>,
}

/// An effect's two nibbles, where a zero picks up the last value given,
/// as in ProTracker.
fn remember(memory: &mut (u8, u8), arg: u8) -> (u8, u8) {
    if arg >> 4 != 0 {
        memory.0 = arg >> 4;
    }
    if arg & 0xF != 0 {
        memory.1 = arg & 0xF;
    }
    *memory
}

/// A line's effect commands for one note column: its volume column's,
/// its own, then those of its track's effect columns, which act on every
/// note column. Later ones win.
#[derive(Clone, Copy, Default)]
struct Effects([Option<(u8, u8)>; 2 + MAX_FX_COLUMNS]);

impl Effects {
    /// The commands `cell` holds, and `more`.
    fn new(cell: &Cell, more: impl Iterator<Item = (u8, u8)>) -> Self {
        let mut e = Effects::default();
        let own = cell.vol.and_then(crate::project::vol_effect).into_iter().chain(cell.fx);
        for (slot, fx) in e.0.iter_mut().zip(own.chain(more)) {
            *slot = Some(fx);
        }
        e
    }

    fn iter(&self) -> impl Iterator<Item = (u8, u8)> + '_ {
        self.0.iter().flatten().copied()
    }

    /// The argument of the last `cmd`.
    fn find(&self, cmd: u8) -> Option<u8> {
        self.iter().filter(|e| e.0 == cmd).last().map(|e| e.1)
    }
}

impl Track {
    /// Ends the line's effects, putting back the pitch and velocity that
    /// arpeggio, vibrato and tremolo moved. `send` gets the events for the
    /// module sounding.
    fn end_line(&mut self, send: &mut impl FnMut(u8, NoteEv)) {
        self.arp = None;
        self.slide = 0.0;
        self.cut_at = None;
        self.delayed = None;
        self.vibrato = None;
        self.tremolo = None;
        self.vol_slide = 0.0;
        self.retrigger = None;
        self.tremor = None;
        self.autopan = None;
        let pan = (self.sent_pan != self.pan).then_some(self.pan);
        self.sent_pan = self.pan;
        if let (Some(m), Some(p)) = (self.sounding, pan) {
            send(m, NoteEv::Pan(p));
        }
        let (pitch, vel) =
            ((self.pitch != self.base).then_some(self.base), (self.sent_vel != self.vel).then_some(self.vel));
        self.pitch = self.base;
        self.sent_vel = self.vel;
        if let Some(m) = self.sounding {
            if let Some(p) = pitch {
                send(m, NoteEv::Pitch(p));
            }
            if let Some(v) = vel {
                send(m, NoteEv::Vel(v));
            }
        }
    }

    /// Plays a line's cell: its note, volume, panning and `effects`, those
    /// that act on the note. Effects on the song (Bxx, Fxx) are left to
    /// the caller. `plays` says whether the module the note goes to takes
    /// notes.
    fn trigger(&mut self, cell: Cell, effects: &Effects, plays: bool, send: &mut impl FnMut(u8, NoteEv)) {
        if let Some(m) = cell.module {
            self.module = Some(m);
        }
        // Above 80 the volume column holds a command, which `effects` has.
        let vel = cell.vol.filter(|&v| v <= 0x80).map(|v| v as f32 / 128.0);
        let pan_fx = effects.find(0x8);
        if let Some(p) = pan_fx {
            // 00 is left, 80 the middle and FF right.
            self.pan = ((p as f32 - 128.0) / 127.0).clamp(-1.0, 1.0);
        }
        // The panning column: 00 left, 40 the middle, 80 right.
        let pan_set = pan_fx.is_some() || cell.pan.is_some();
        if let Some(p) = cell.pan {
            self.pan = ((p.min(0x80) as f32 - 64.0) / 64.0).clamp(-1.0, 1.0);
        }

        match cell.note {
            Some(Note::On(n)) => {
                let porta = effects.find(0x3).filter(|_| self.sounding.is_some() && self.sounding == self.module);
                if let Some(speed) = porta {
                    self.porta_target = Some(n as f32);
                    self.porta_speed = speed.max(1) as f32 / 16.0;
                } else {
                    if let Some(m) = self.sounding.take() {
                        send(m, NoteEv::Off);
                    }
                    self.vel = vel.unwrap_or(1.0);
                    if let Some(m) = self.module.filter(|_| plays) {
                        send(m, NoteEv::On(n as f32, self.vel));
                        if self.pan != 0.0 {
                            send(m, NoteEv::Pan(self.pan));
                        }
                        if let Some(k) = effects.find(FX_SLICE) {
                            send(m, NoteEv::Slice(k));
                        }
                        if let Some(offset) = effects.find(0x9) {
                            send(m, NoteEv::Offset(offset as f32 / 256.0));
                        }
                        self.sounding = Some(m);
                    }
                    self.base = n as f32;
                    self.pitch = n as f32;
                    self.sent_vel = self.vel;
                    self.porta_target = None;
                    self.vibrato_phase = 0.0;
                    self.tremolo_phase = 0.0;
                }
            }
            Some(Note::Off) => {
                if let Some(m) = self.sounding.take() {
                    send(m, NoteEv::Off);
                }
            }
            None => {
                if let Some(v) = vel {
                    self.vel = v;
                    self.sent_vel = v;
                    if let Some(m) = self.sounding {
                        send(m, NoteEv::Vel(v));
                    }
                }
            }
        }
        // Panning moves a note that is already playing too.
        if pan_set && let Some(m) = self.sounding {
            send(m, NoteEv::Pan(self.pan));
        }
        // So does Rxx, from the end for a note just started.
        if let (Some(r), Some(m)) = (effects.find(FX_REVERSE), self.sounding) {
            send(m, NoteEv::Reverse(r != 0));
        }
        self.sent_pan = self.pan;

        for fx in effects.iter() {
            self.effect(fx);
        }
        if self.cut_at == Some(0) {
            self.cut(send);
        }
    }

    /// Sets up the per-line effect `fx` for the ticks to come.
    fn effect(&mut self, fx: (u8, u8)) {
        match fx {
            (0x0, a) if a != 0 => self.arp = Some((a >> 4, a & 0xF)),
            (0x1, a) => self.slide = a as f32 / 16.0,
            (0x2, a) => self.slide = -(a as f32) / 16.0,
            (0x4, a) => {
                let (speed, depth) = remember(&mut self.vibrato_memory, a);
                self.vibrato = Some((speed as f32 / 32.0, depth as f32 / 8.0));
            }
            (0x7, a) => {
                let (speed, depth) = remember(&mut self.tremolo_memory, a);
                self.tremolo = Some((speed as f32 / 32.0, depth as f32 / 16.0));
            }
            (0xA, a) => {
                let (up, down) = (a >> 4, a & 0xF);
                self.vol_slide = if up != 0 { up as f32 } else { -(down as f32) } / 128.0;
            }
            (FX_AUTOPAN, a) => {
                let (speed, depth) = remember(&mut self.autopan_memory, a);
                self.autopan = Some((speed as f32 / 32.0, depth as f32 / 15.0));
            }
            (0xC, a) => self.cut_at = Some(a as u32),
            (0xE, a) if a > 0 => self.retrigger = Some(a as u32),
            (FX_TREMOR, a) => self.tremor = Some(((a >> 4).max(1) as u32, (a & 0xF) as u32)),
            _ => {}
        }
    }

    fn cut(&mut self, send: &mut impl FnMut(u8, NoteEv)) {
        if let Some(m) = self.sounding.take() {
            send(m, NoteEv::Off);
        }
        self.cut_at = None;
    }

    /// Runs the line's effects on tick `tick` after its first.
    fn tick_effects(&mut self, tick: u32, send: &mut impl FnMut(u8, NoteEv)) {
        let Some(m) = self.sounding else { return };
        if self.cut_at == Some(tick) {
            self.cut(send);
            return;
        }
        // Slides and glides move the note for good; arpeggio and vibrato
        // only for this line.
        self.base += self.slide;
        if let Some(target) = self.porta_target {
            let step = self.porta_speed;
            self.base =
                if self.base < target { (self.base + step).min(target) } else { (self.base - step).max(target) };
            if self.base == target {
                self.porta_target = None;
            }
        }
        let mut pitch = self.base;
        if let Some((x, y)) = self.arp {
            pitch += [0, x, y][(tick % 3) as usize] as f32;
        }
        if let Some((speed, depth)) = self.vibrato {
            self.vibrato_phase = (self.vibrato_phase + speed).fract();
            pitch += (self.vibrato_phase * TAU).sin() * depth;
        }
        // Likewise a volume slide changes the velocity for good, tremolo
        // only for this line.
        self.vel = (self.vel + self.vol_slide).clamp(0.0, 1.0);
        let mut vel = self.vel;
        if let Some((speed, depth)) = self.tremolo {
            self.tremolo_phase = (self.tremolo_phase + speed).fract();
            vel *= 1.0 - depth * (0.5 - 0.5 * (self.tremolo_phase * TAU).cos());
        }
        if let Some((on, off)) = self.tremor
            && tick % (on + off) >= on
        {
            vel = 0.0;
        }
        if let Some((speed, depth)) = self.autopan {
            self.autopan_phase = (self.autopan_phase + speed).fract();
            let pan = (self.pan + (self.autopan_phase * TAU).sin() * depth).clamp(-1.0, 1.0);
            if pan != self.sent_pan {
                self.sent_pan = pan;
                send(m, NoteEv::Pan(pan));
            }
        }
        let retrigger = self.retrigger.is_some_and(|r| tick.is_multiple_of(r));
        let (moved, louder) = (pitch != self.pitch, vel != self.sent_vel);
        self.pitch = pitch;
        self.sent_vel = vel;
        if retrigger {
            send(m, NoteEv::Off);
            send(m, NoteEv::On(pitch, vel));
            if self.pan != 0.0 {
                send(m, NoteEv::Pan(self.pan));
            }
            return;
        }
        if moved {
            send(m, NoteEv::Pitch(pitch));
        }
        if louder {
            send(m, NoteEv::Vel(vel));
        }
    }
}

pub struct Engine {
    sr: f32,
    project: Arc<Project>,
    nodes: Vec<Node>,
    /// Node indices in processing order (sources first).
    order: Vec<usize>,
    output: Option<usize>,
    /// For each module's node, by index, its copies for tracks with
    /// effects, so a note finds its track's copy at once.
    copies: Vec<[Option<usize>; MAX_TRACKS]>,
    /// The last effect of each track that has effects, whose sound its
    /// scope shows.
    track_ends: Vec<(usize, usize)>,
    scratch: Vec<Frame>,
    /// What the key input of the effect being processed adds up to.
    key_scratch: Vec<Frame>,

    playing: bool,
    loop_pattern: bool,
    pos_order: usize,
    pos_line: usize,
    tick: u32,
    samples_to_tick: f64,
    bpm: f32,
    /// Ticks per line, which Fxx can change while playing.
    tpl: u32,
    /// Order position a Bxx effect jumps to after this line.
    jump: Option<usize>,
    /// Line a Jxx breaks to, in the next slot, after this line.
    break_to: Option<usize>,
    /// Lines a Wxx holds this line for, after its own.
    wait: u32,
    /// Each track's volume, as Lxx sets it, scaling its notes' velocity.
    track_volume: [f32; MAX_TRACKS],
    /// The block loop: order position and first and last line.
    block_loop: Option<(usize, usize, usize)>,
    /// The state of each note column, `column * MAX_TRACKS + track`, which
    /// is also the key its notes are sent with.
    tracks: Vec<Track>,
    /// Set when the song wraps around to the start; used by offline render.
    pub song_ended: bool,
    /// An F00 played on this line: playing stops when it ends.
    stop_after_line: bool,
    /// The order position and line currently sounding, and the tick.
    shown: (usize, usize),
    shown_tick: u32,
    /// Frames rendered since playback started.
    played: u64,
    /// Notes played on instruments so far, to tell which came last.
    notes_played: u64,

    /// Disk browser preview: the sample and the read position.
    preview: Option<(Arc<Sample>, f64)>,
    preview_volume: f32,
    preview_loop: bool,
    metronome: bool,
    /// The metronome click sounding: seconds since it started and its pitch.
    click: Option<(f32, f32)>,

    rx: Option<Receiver<Cmd>>,
    garbage: Option<Sender<Garbage>>,
    shared: Arc<Shared>,
    scope: Vec<Frame>,
    scope_pos: usize,
    playheads: Vec<Playhead>,
    /// State of the random numbers MultiSynths use.
    rng: u32,
    /// The automated values of the last block, for `Shared::automated`.
    live: Vec<(u8, usize, f32)>,
    phrases: Vec<PhrasePlayer>,
    /// The phrase a `Zxx` picks for the note being sent, 0 for none.
    picked_phrase: Option<u8>,
    /// What each track's notes add up to in the block being rendered, and
    /// the rings of the track scopes, written at `track_pos`.
    track_block: Vec<[f32; BLOCK]>,
    track_rings: Vec<f32>,
    track_pos: usize,
}

impl Engine {
    pub fn new(
        sr: f32,
        project: Arc<Project>,
        rx: Option<Receiver<Cmd>>,
        garbage: Option<Sender<Garbage>>,
        shared: Arc<Shared>,
    ) -> Self {
        let mut e = Self {
            sr,
            bpm: project.bpm,
            tpl: project.tpl,
            jump: None,
            break_to: None,
            wait: 0,
            track_volume: [1.0; MAX_TRACKS],
            block_loop: None,
            project: project.clone(),
            nodes: Vec::new(),
            order: Vec::new(),
            output: None,
            copies: Vec::new(),
            track_ends: Vec::new(),
            scratch: vec![[0.0; 2]; BLOCK],
            key_scratch: vec![[0.0; 2]; BLOCK],
            playing: false,
            loop_pattern: false,
            pos_order: 0,
            pos_line: 0,
            tick: 0,
            samples_to_tick: 0.0,
            tracks: vec![Track::default(); MAX_TRACKS * MAX_COLUMNS],
            song_ended: false,
            stop_after_line: false,
            shown: (0, 0),
            shown_tick: 0,
            played: 0,
            notes_played: 0,
            preview: None,
            preview_volume: 0.8,
            preview_loop: false,
            metronome: false,
            click: None,
            rx,
            garbage,
            shared,
            scope: vec![[0.0; 2]; SCOPE_LEN],
            scope_pos: 0,
            playheads: Vec::with_capacity(4 * MAX_PLAYHEADS),
            rng: 0x2545_F491,
            live: Vec::with_capacity(MAX_AUTOMATED),
            phrases: Vec::with_capacity(MAX_PHRASES),
            picked_phrase: None,
            track_block: vec![[0.0; BLOCK]; MAX_TRACKS],
            track_rings: vec![0.0; MAX_TRACKS * TRACK_SCOPE_LEN],
            track_pos: 0,
        };
        e.set_project(project);
        e
    }

    fn set_project(&mut self, project: Arc<Project>) {
        if project.bpm != self.project.bpm {
            self.bpm = project.bpm;
        }
        if project.tpl != self.project.tpl {
            self.tpl = project.tpl;
        }
        // Keep DSP state for modules that still exist so tails and voices
        // survive edits; create the rest.
        let mut old: Vec<Node> = std::mem::take(&mut self.nodes);
        let (instances, links) = project.signal_graph();
        let mut module_of = [None; 256];
        for (mi, m) in project.modules.iter().enumerate() {
            module_of[m.id as usize] = Some(mi);
        }
        for &(id, track) in &instances {
            let Some(mi) = module_of[id as usize] else { continue };
            let m = &project.modules[mi];
            let node = match old.iter().position(|n| n.id == m.id && n.kind == m.kind && n.track == track) {
                Some(i) => {
                    let mut n = old.swap_remove(i);
                    n.module = mi;
                    n.inputs.clear();
                    n.key.clear();
                    n.targets.clear();
                    n.controls.clear();
                    n.macro_targets.clear();
                    n
                }
                None => Node {
                    id: m.id,
                    track,
                    kind: m.kind,
                    module: mi,
                    dsp: match m.kind {
                        ModuleKind::Input => Box::new(dsp::LiveInput::new(self.shared.input.clone())),
                        kind => dsp::create(kind, self.sr),
                    },
                    inputs: Vec::new(),
                    key: Vec::new(),
                    buf: vec![[0.0; 2]; BLOCK],
                    audible: true,
                    peak: [0.0; 2],
                    params: Vec::with_capacity(m.params.len()),
                    automated: false,
                    mix_gain: None,
                    mix_pan: None,
                    macros: [None; MACROS],
                    macro_targets: Vec::new(),
                    targets: Vec::new(),
                    held: Vec::with_capacity(if m.kind.notes_only() { MAX_HELD } else { 0 }),
                    next_target: 0,
                    glides: Vec::with_capacity(if m.kind == ModuleKind::Glide { MAX_HELD } else { 0 }),
                    controls: Vec::new(),
                    phase: 0.0,
                    follow: 0.0,
                    last_note: (0.0, 0.0, 0),
                },
            };
            let mut node = node;
            node.dsp.set_samples(&m.samples);
            node.dsp.set_modulation(&m.modulation);
            self.nodes.push(node);
        }
        // Meters of deleted modules drop to zero.
        for n in old {
            for ch in 0..2 {
                self.shared.levels[2 * n.id as usize + ch].store(0, Ordering::Relaxed);
            }
        }
        // Where each module's node is, and its copies.
        let mut base = [None; 256];
        for (i, n) in self.nodes.iter().enumerate().filter(|(_, n)| n.track.is_none()) {
            base[n.id as usize] = Some(i);
        }
        self.copies = vec![[None; MAX_TRACKS]; self.nodes.len()];
        for (i, n) in self.nodes.iter().enumerate() {
            if let (Some(t), Some(b)) = (n.track, base[n.id as usize]) {
                self.copies[b][t] = Some(i);
            }
        }
        let index = |&(id, track): &(u8, Option<usize>), copies: &[[Option<usize>; MAX_TRACKS]]| {
            let b = base[id as usize]?;
            match track {
                None => Some(b),
                Some(t) => copies[b][t],
            }
        };
        for (from, to) in &links {
            let (Some(a), Some(b)) = (index(from, &self.copies), index(to, &self.copies)) else { continue };
            if self.nodes[a].kind.notes_only() {
                self.nodes[a].targets.push(b);
            } else if self.nodes[a].kind.controls() {
                // A Modulator moves its target's copies too.
                let param = project.control_param(from.0, to.0);
                self.nodes[a].controls.push((b, param));
                for c in self.copies[b].into_iter().flatten() {
                    self.nodes[a].controls.push((c, param));
                }
            } else {
                self.nodes[b].inputs.push(a);
            }
        }
        // An effect listening to a key input hears every copy of it.
        for j in 0..self.nodes.len() {
            let m = &project.modules[self.nodes[j].module];
            let Some(src) = m.key.filter(|&k| project.can_key(m.id, k)) else { continue };
            let Some(b) = base[src as usize] else { continue };
            self.nodes[j].key.push(b);
            self.nodes[j].key.extend(self.copies[b].into_iter().flatten());
        }
        // What each instrument's macros move: the node of the module named,
        // its copy for the same track when there is one.
        for j in 0..self.nodes.len() {
            let m = &project.modules[self.nodes[j].module];
            for (k, mac) in m.macros.iter().enumerate() {
                for t in &mac.targets {
                    let track = self.nodes[j].track;
                    let found = (self.nodes.iter().position(|n| n.id == t.module && n.track == track))
                        .or_else(|| base[t.module as usize]);
                    if let Some(at) = found {
                        self.nodes[j].macro_targets.push((k, at, t.param, t.from, t.to));
                    }
                }
            }
        }
        self.output = base[OUTPUT_ID as usize];
        self.track_ends = (project.tracks.iter().enumerate())
            .filter_map(|(t, track)| Some((t, base[*track.effects.last()? as usize]?)))
            .collect();
        self.apply_solo(&project);

        // Kahn's algorithm. The project refuses cyclic links, but stay safe
        // and simply drop anything left over.
        // A Modulator goes before the modules it moves.
        let n = self.nodes.len();
        let mut edges: Vec<(usize, usize)> = Vec::new();
        for (j, node) in self.nodes.iter().enumerate() {
            edges.extend(node.inputs.iter().map(|&i| (i, j)));
            edges.extend(node.key.iter().map(|&i| (i, j)));
            edges.extend(node.controls.iter().map(|&(t, _)| (j, t)));
        }
        let mut indeg = vec![0; n];
        for &(_, j) in &edges {
            indeg[j] += 1;
        }
        let mut queue: Vec<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
        self.order.clear();
        while let Some(i) = queue.pop() {
            self.order.push(i);
            for &(_, j) in edges.iter().filter(|e| e.0 == i) {
                indeg[j] -= 1;
                if indeg[j] == 0 {
                    queue.push(j);
                }
            }
        }

        let old_project = std::mem::replace(&mut self.project, project);
        self.discard(Garbage::Project(old_project));
        if self.pos_order >= self.project.order.len() {
            self.pos_order = 0;
        }
    }

    /// Soloing a module keeps it, everything that feeds it and everything
    /// it feeds audible, and silences the rest.
    fn apply_solo(&mut self, project: &Project) {
        let n = self.nodes.len();
        let soloed: Vec<usize> = (0..n).filter(|&i| project.modules[self.nodes[i].module].solo).collect();
        let (mut up, mut down) = (vec![false; n], vec![false; n]);
        for &s in &soloed {
            let mut stack = vec![s];
            while let Some(i) = stack.pop() {
                if !std::mem::replace(&mut up[i], true) {
                    stack.extend(&self.nodes[i].inputs);
                }
            }
            let mut stack = vec![s];
            while let Some(i) = stack.pop() {
                if !std::mem::replace(&mut down[i], true) {
                    stack.extend((0..n).filter(|&j| self.nodes[j].inputs.contains(&i)));
                    stack.extend(&self.nodes[i].targets);
                    stack.extend(self.nodes[i].controls.iter().map(|c| c.0));
                }
            }
        }
        for (i, node) in self.nodes.iter_mut().enumerate() {
            node.audible = soloed.is_empty() || up[i] || down[i];
        }
    }

    fn discard(&self, g: Garbage) {
        if let Some(tx) = &self.garbage {
            let _ = tx.send(g);
        }
    }

    /// Whether module `id` exists and takes notes.
    fn plays_notes(&self, id: u8) -> bool {
        self.nodes.iter().any(|n| n.id == id && n.kind.is_instrument())
    }

    /// Sends a note event to module `id`.
    fn note(&mut self, id: u8, key: u32, ev: NoteEv) {
        if let Some(i) = self.nodes.iter().position(|n| n.id == id && n.track.is_none()) {
            self.note_to(self.copy_for(i, key), key, ev, 0);
        }
    }

    /// Node `i`'s copy for the track playing `key`, if that track has
    /// effects and so a copy; otherwise `i`.
    fn copy_for(&self, i: usize, key: u32) -> usize {
        let key = key as usize;
        if key >= MAX_TRACKS * MAX_COLUMNS {
            return i;
        }
        self.copies.get(i).and_then(|c| c[key % MAX_TRACKS]).unwrap_or(i)
    }

    fn random(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        self.rng as f32 / u32::MAX as f32
    }

    /// Sends a note event to node `i`; a MultiSynth changes it and passes
    /// it on to the instruments it is connected to.
    fn note_to(&mut self, i: usize, key: u32, ev: NoteEv, depth: u8) {
        // An instrument with phrases may play one instead.
        let project = self.project.clone();
        let module = &project.modules[self.nodes[i].module];
        if !module.phrases.is_empty() && module.kind.makes_sound() && self.phrase_event(i, module, key, ev) {
            return;
        }
        let node = &mut self.nodes[i];
        if !node.kind.notes_only() {
            match ev {
                NoteEv::On(note, vel) => {
                    self.notes_played += 1;
                    node.last_note = (note, vel, self.notes_played);
                }
                NoteEv::Pitch(note) => node.last_note.0 = note,
                _ => {}
            }
            let dsp = &mut node.dsp;
            match ev {
                NoteEv::On(note, vel) => dsp.note_on(key, note, vel),
                NoteEv::Off => dsp.note_off(key),
                NoteEv::Pitch(note) => dsp.set_pitch(key, note),
                NoteEv::Vel(vel) => dsp.set_velocity(key, vel),
                NoteEv::Pan(pan) => dsp.set_pan(key, pan),
                NoteEv::Offset(pos) => dsp.sample_offset(key, pos),
                NoteEv::Reverse(on) => dsp.reverse(key, on),
                NoteEv::Slice(k) => dsp.play_slice(key, k as usize),
                NoteEv::Seek(frames) => dsp.seek(key, frames),
            }
            return;
        }
        if node.kind == ModuleKind::Glide {
            self.glide_event(i, key, ev, depth);
            return;
        }
        if depth >= MAX_NOTE_DEPTH || node.targets.is_empty() {
            return;
        }
        let project = &self.project;
        let p = if node.automated { &node.params } else { &project.modules[node.module].params };
        let (mode, transpose, finetune, spread, vel_scale, vel_spread, low, high) =
            (p[0].round() as u32, p[1], p[2], p[3], p[4], p[5], p[6], p[7]);
        let held = node.held.iter().position(|h| h.key == key);
        let held = match ev {
            NoteEv::On(note, _) => {
                if let Some(h) = held {
                    node.held.swap_remove(h);
                }
                if note < low.round() || note > high.round() || node.held.len() >= MAX_HELD {
                    return;
                }
                let n = self.nodes[i].targets.len();
                let target = match mode {
                    0 => None,
                    1 => {
                        let t = self.nodes[i].next_target % n;
                        self.nodes[i].next_target = (t + 1) % n;
                        Some(t)
                    }
                    _ => Some(((self.random() * n as f32) as usize).min(n - 1)),
                };
                let detune = transpose + (finetune + spread * (2.0 * self.random() - 1.0)) / 100.0;
                let vel = vel_scale * (1.0 - vel_spread * self.random());
                let h = Held { key, detune, vel, target };
                self.nodes[i].held.push(h);
                h
            }
            _ => match held {
                Some(h) => node.held[h],
                None => return,
            },
        };
        if ev == NoteEv::Off {
            self.nodes[i].held.retain(|h| h.key != key);
        }
        let ev = match ev {
            NoteEv::On(note, vel) => NoteEv::On(note + held.detune, (vel * held.vel).min(1.0)),
            NoteEv::Pitch(note) => NoteEv::Pitch(note + held.detune),
            NoteEv::Vel(vel) => NoteEv::Vel((vel * held.vel).min(1.0)),
            ev => ev,
        };
        let n = self.nodes[i].targets.len();
        for t in 0..n {
            if held.target.is_none_or(|h| h == t) {
                let j = self.copy_for(self.nodes[i].targets[t], key);
                self.note_to(j, key, ev, depth + 1);
            }
        }
    }

    /// Passes `ev` from note module `i` on to every instrument it is
    /// connected to.
    fn pass_on(&mut self, i: usize, key: u32, ev: NoteEv, depth: u8) {
        if depth >= MAX_NOTE_DEPTH {
            return;
        }
        for t in 0..self.nodes[i].targets.len() {
            let j = self.copy_for(self.nodes[i].targets[t], key);
            self.note_to(j, key, ev, depth + 1);
        }
    }

    /// A note event for Glide node `i`: a new note starts where the last
    /// one on its key was and slides to its own pitch, in Time whatever
    /// the distance. In Legato mode only a note played over the last one
    /// slides, and goes on sounding rather than starting again.
    fn glide_event(&mut self, i: usize, key: u32, ev: NoteEv, depth: u8) {
        let node = &self.nodes[i];
        let p = if node.automated { &node.params } else { &self.project.modules[node.module].params };
        let (legato, time) = (p[0] >= 0.5, p[1].max(0.001));
        let at = node.glides.iter().position(|g| g.key == key);
        let glides = &mut self.nodes[i].glides;
        match ev {
            NoteEv::On(note, vel) => {
                let last = at.map(|a| glides[a]);
                let tied = last.is_some_and(|g| g.sounding);
                let from = match last {
                    Some(g) if tied || !legato => g.at,
                    _ => note,
                };
                let g = GlideVoice {
                    key,
                    at: from,
                    to: note,
                    rate: (note - from).abs() / time,
                    sounding: true,
                    off_pending: false,
                };
                match at {
                    Some(a) => glides[a] = g,
                    None if glides.len() < MAX_HELD => glides.push(g),
                    None => {}
                }
                if legato && tied {
                    self.pass_on(i, key, NoteEv::Vel(vel), depth);
                } else {
                    // Starting again, with the last note let go first if it
                    // was held back.
                    if last.is_some_and(|g| g.off_pending) {
                        self.pass_on(i, key, NoteEv::Off, depth);
                    }
                    self.pass_on(i, key, NoteEv::On(from, vel), depth);
                }
            }
            NoteEv::Off => match at {
                Some(a) if legato => glides[a].off_pending = glides[a].sounding,
                _ => {
                    if let Some(a) = at {
                        glides[a].sounding = false;
                    }
                    self.pass_on(i, key, ev, depth);
                }
            },
            NoteEv::Pitch(note) => {
                // Slides and vibrato move the note it goes to; while it
                // still glides there, it keeps gliding.
                match at.map(|a| &mut glides[a]) {
                    Some(g) if g.at != g.to => g.to = note,
                    Some(g) => {
                        (g.at, g.to) = (note, note);
                        self.pass_on(i, key, ev, depth);
                    }
                    None => self.pass_on(i, key, ev, depth),
                }
            }
            ev => self.pass_on(i, key, ev, depth),
        }
    }

    /// Moves every Glide's notes on by `frames`, and lets go of the notes
    /// it held back that no new note followed.
    fn run_glides(&mut self, frames: usize) {
        let dt = frames as f32 / self.sr;
        for i in 0..self.nodes.len() {
            if self.nodes[i].glides.is_empty() {
                continue;
            }
            for k in 0..self.nodes[i].glides.len() {
                let g = &mut self.nodes[i].glides[k];
                let key = g.key;
                if std::mem::take(&mut g.off_pending) {
                    g.sounding = false;
                    self.pass_on(i, key, NoteEv::Off, 0);
                    continue;
                }
                let g = &mut self.nodes[i].glides[k];
                if g.at != g.to {
                    let step = g.rate.max(1e-3) * dt;
                    g.at = if g.at < g.to { (g.at + step).min(g.to) } else { (g.at - step).max(g.to) };
                    let at = g.at;
                    self.pass_on(i, key, NoteEv::Pitch(at), 0);
                }
            }
        }
    }

    /// Whether a Glide is sliding a note, so rendering goes in small steps.
    fn gliding(&self) -> bool {
        self.nodes.iter().any(|n| n.glides.iter().any(|g| g.at != g.to || g.off_pending))
    }

    /// Forgets the notes MultiSynths and Glides hold, and the phrases
    /// playing.
    fn forget_held(&mut self) {
        for n in &mut self.nodes {
            n.held.clear();
            n.glides.clear();
        }
        self.phrases.clear();
    }

    /// Samples a line of `phrase` takes at the song's tempo.
    fn phrase_line(&self, lpb: u32) -> f64 {
        self.samples_per_tick() * self.tpl.max(1) as f64 * self.project.lpb.max(1) as f64 / lpb.max(1) as f64
    }

    /// The phrase of `module` that note `note` plays: the one a `Zxx`
    /// picked, or else the one its phrase mode picks.
    fn pick_phrase(&self, module: &crate::project::Module, note: f32) -> Option<usize> {
        let phrase = match self.picked_phrase {
            Some(0) => return None,
            Some(z) => z as usize - 1,
            None => match module.phrase_mode {
                PhraseMode::Off => return None,
                PhraseMode::Program => module.selected_phrase,
                PhraseMode::Keymap => module.phrases.iter().position(|p| p.has_key(note))?,
            },
        };
        (phrase < module.phrases.len()).then_some(phrase)
    }

    /// Handles a note event for node `i` (instrument `module`, which has
    /// phrases). Returns false for events that go to the instrument as
    /// they are: notes that play no phrase, and what follows them.
    fn phrase_event(&mut self, i: usize, module: &crate::project::Module, key: u32, ev: NoteEv) -> bool {
        let id = module.id;
        let at = self.phrases.iter().position(|p| p.key == key && p.module == id);
        match ev {
            NoteEv::On(note, vel) => {
                if let Some(at) = at {
                    let old = self.phrases.swap_remove(at);
                    if old.track.sounding.is_some() {
                        self.nodes[i].dsp.note_off(key);
                    }
                }
                let Some(phrase) = self.pick_phrase(module, note) else { return false };
                if self.phrases.len() < MAX_PHRASES {
                    let track = Track { module: Some(id), ..Track::default() };
                    let player = PhrasePlayer {
                        module: id,
                        phrase,
                        key,
                        transpose: note - 48.0,
                        vel,
                        playing: 0,
                        line: 0,
                        tick: 0,
                        wait: 0.0,
                        track,
                    };
                    self.phrases.push(player);
                    // The first line plays at once.
                    let last = self.phrases.len() - 1;
                    self.advance_phrase(last, 0.0);
                }
                true
            }
            NoteEv::Off => {
                // The phrase's last note may still sound after it ended, so
                // the instrument gets the note-off either way.
                if let Some(at) = at {
                    self.phrases.swap_remove(at);
                }
                false
            }
            NoteEv::Pitch(note) => {
                let Some(at) = at else { return false };
                let p = &mut self.phrases[at];
                p.transpose = note - 48.0;
                if p.track.sounding.is_some() {
                    let p = *p;
                    p.send(self.nodes[i].dsp.as_mut(), NoteEv::Pitch(p.track.pitch));
                }
                true
            }
            NoteEv::Vel(vel) => {
                let Some(at) = at else { return false };
                let p = &mut self.phrases[at];
                p.vel = vel;
                if p.track.sounding.is_some() {
                    let p = *p;
                    p.send(self.nodes[i].dsp.as_mut(), NoteEv::Vel(p.track.sent_vel));
                }
                true
            }
            NoteEv::Seek(_) => {
                // Phrases don't seek: one started before playback did
                // stays silent, as other instruments do.
                let Some(at) = at else { return false };
                self.phrases.swap_remove(at);
                self.nodes[i].dsp.note_off(key);
                true
            }
            NoteEv::Pan(_) | NoteEv::Offset(_) | NoteEv::Reverse(_) | NoteEv::Slice(_) => false,
        }
    }

    /// `cell`, without its note when a `Yxx` in `effects` decides it
    /// doesn't play this time: it plays with a chance of xx in FF.
    fn maybe(&mut self, mut cell: Cell, effects: &Effects) -> Cell {
        if let Some(chance) = effects.find(FX_MAYBE)
            && matches!(cell.note, Some(Note::On(_)))
            && self.random() * 255.0 >= chance as f32
        {
            cell.note = None;
        }
        cell
    }

    /// Moves phrase player `k` on by `frames`, playing the lines and
    /// ticks it reaches. Returns false once a phrase that doesn't loop is
    /// over.
    fn advance_phrase(&mut self, k: usize, frames: f64) -> bool {
        let project = self.project.clone();
        let mut p = self.phrases[k];
        let base = self.nodes.iter().position(|n| n.id == p.module && n.track.is_none());
        let (Some(i), Some(module)) = (base.map(|b| self.copy_for(b, p.key)), project.module(p.module)) else {
            return false;
        };
        let Some(phrase) = module.phrases.get(p.phrase) else { return false };
        let tpl = self.tpl.max(1);
        let tick_len = self.phrase_line(phrase.lpb) / tpl as f64;
        p.wait -= frames;
        while p.wait <= 0.0 {
            if p.tick == 0 && p.line >= phrase.lines {
                if !phrase.looping {
                    self.phrases[k] = p;
                    return false;
                }
                p.line = 0;
            }
            // The phrase's cells play on its instrument, and effects on the
            // song (Bxx, Fxx, Zxx) don't apply.
            let line = (p.tick == 0).then(|| {
                let cell = Cell { module: None, ..phrase.cells[p.line] };
                let effects = Effects::new(&cell, std::iter::empty());
                (self.maybe(cell, &effects), effects)
            });
            let dsp = self.nodes[i].dsp.as_mut();
            let mut track = p.track;
            let player = p;
            let mut send = |_, ev| player.send(dsp, ev);
            if let Some((cell, effects)) = line {
                p.playing = p.line;
                track.end_line(&mut send);
                match effects.find(0xD) {
                    Some(d) if d > 0 => track.delayed = Some((d as u32, cell, effects)),
                    _ => track.trigger(cell, &effects, true, &mut send),
                }
            } else {
                if let Some((at, cell, effects)) = track.delayed
                    && at == p.tick
                {
                    track.delayed = None;
                    track.trigger(cell, &effects, true, &mut send);
                }
                track.tick_effects(p.tick, &mut send);
            }
            p.track = track;
            p.tick += 1;
            if p.tick >= tpl {
                p.tick = 0;
                p.line += 1;
            }
            p.wait += tick_len;
        }
        self.phrases[k] = p;
        true
    }

    /// Plays the phrases on for `frames` more frames.
    fn run_phrases(&mut self, frames: usize) {
        let mut k = 0;
        while k < self.phrases.len() {
            if self.advance_phrase(k, frames as f64) {
                k += 1;
            } else {
                // Over: its last note sounds on until the key lets go.
                self.phrases.swap_remove(k);
            }
        }
    }

    pub fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Project(p) => self.set_project(p),
            Cmd::Play { order, line, loop_pattern } => {
                self.stop_notes();
                self.playing = true;
                self.loop_pattern = loop_pattern;
                self.pos_order = order.min(self.project.order.len().saturating_sub(1));
                self.pos_line = line;
                self.tick = 0;
                self.samples_to_tick = 0.0;
                self.bpm = self.project.bpm;
                self.tpl = self.project.tpl;
                self.jump = None;
                self.break_to = None;
                self.wait = 0;
                self.track_volume = [1.0; MAX_TRACKS];
                self.song_ended = false;
                self.stop_after_line = false;
                self.shown = (self.pos_order, line);
                self.played = 0;
                self.autoseek();
            }
            Cmd::Stop => {
                self.playing = false;
                self.stop_notes();
            }
            Cmd::NoteOn { module, key, note, vel } => self.note(module, key, NoteEv::On(note as f32, vel)),
            Cmd::NoteOff { module, key } => self.note(module, key, NoteEv::Off),
            Cmd::Panic => {
                self.playing = false;
                self.tracks.fill(Track::default());
                self.forget_held();
                for n in &mut self.nodes {
                    n.dsp.reset();
                }
                self.set_preview(None);
                self.click = None;
            }
            Cmd::Preview(sample) => self.set_preview(sample),
            Cmd::PreviewSettings { volume, looping } => (self.preview_volume, self.preview_loop) = (volume, looping),
            Cmd::Metronome(on) => self.metronome = on,
            Cmd::BlockLoop(range) => self.block_loop = range,
        }
    }

    fn set_preview(&mut self, sample: Option<Arc<Sample>>) {
        if let Some((old, _)) = self.preview.take() {
            self.discard(Garbage::Sample(old));
        }
        self.preview = sample.map(|s| (s, 0.0));
    }

    /// Mixes the disk browser preview into `out`.
    fn render_preview(&mut self, out: &mut [Frame]) {
        let Some((sample, pos)) = &mut self.preview else { return };
        let rate = (sample.sample_rate / self.sr) as f64;
        let len = sample.frames.len();
        let vol = self.preview_volume;
        for o in out.iter_mut() {
            if *pos as usize >= len {
                if !self.preview_loop || len == 0 {
                    self.set_preview(None);
                    return;
                }
                *pos = pos.rem_euclid(len as f64);
            }
            let x = sample.frames[*pos as usize];
            o[0] += x[0] * vol;
            o[1] += x[1] * vol;
            *pos += rate;
        }
    }

    /// Mixes the metronome click into `out`.
    fn render_click(&mut self, out: &mut [Frame]) {
        let Some((t, freq)) = &mut self.click else { return };
        for o in out.iter_mut() {
            let env = (-*t * 70.0).exp();
            if env < 0.001 {
                self.click = None;
                return;
            }
            let x = (*t * *freq * TAU).sin() * env * 0.4;
            o[0] += x;
            o[1] += x;
            *t += 1.0 / self.sr;
        }
    }

    fn stop_notes(&mut self) {
        for n in &mut self.nodes {
            n.dsp.release_all();
        }
        self.forget_held();
        self.tracks.fill(Track::default());
    }

    pub fn play_song(&mut self) {
        self.handle(Cmd::Play { order: 0, line: 0, loop_pattern: false });
    }

    /// How long until the next tick, after `run_tick` has moved on to it.
    /// With groove, an odd line starts late and its ticks are shorter, so
    /// the even line after it is on time.
    fn tick_wait(&self) -> f64 {
        let tick = self.samples_per_tick();
        let delay = self.project.groove.clamp(0.0, 1.0) as f64 * 0.5 * tick * self.tpl.max(1) as f64;
        if delay == 0.0 {
            return tick;
        }
        // The line just played, and the tick coming up.
        let played_odd = self.shown.1 % 2 == 1;
        let base = if played_odd { tick - delay / self.tpl.max(1) as f64 } else { tick };
        let late = self.tick == 0 && self.pos_line % 2 == 1;
        base + if late { delay } else { 0.0 }
    }

    /// Ticks this line lasts: TPL, and as many again for each line a
    /// Wxx holds it.
    fn line_ticks(&self) -> u32 {
        self.tpl.max(1) * (1 + self.wait)
    }

    fn samples_per_tick(&self) -> f64 {
        self.sr as f64 * 60.0 / (self.bpm.max(1.0) as f64 * self.project.lpb.max(1) as f64 * self.tpl.max(1) as f64)
    }

    // ------------------------------------------------------------ sequencer

    fn run_tick(&mut self) {
        let project = self.project.clone();
        let Some(&slot) = project.order.get(self.pos_order) else {
            self.playing = false;
            return;
        };
        let Some(pattern) = project.patterns.get(slot.pattern) else {
            self.playing = false;
            return;
        };
        if self.pos_line >= pattern.lines {
            self.pos_line = 0;
        }
        self.shown_tick = self.tick;
        if self.tick == 0 {
            self.shown = (self.pos_order, self.pos_line);
            self.wait = 0;
            let lpb = self.project.lpb.max(1) as usize;
            if self.metronome && self.pos_line.is_multiple_of(lpb) {
                let accent = self.pos_line.is_multiple_of(lpb * BEATS_PER_BAR);
                self.click = Some((0.0, if accent { 1760.0 } else { 880.0 }));
            }
        }

        for ch in 0..MAX_TRACKS * MAX_COLUMNS {
            let (t, col) = (ch % MAX_TRACKS, ch / MAX_TRACKS);
            let key = ch as u32;
            let shown = t < pattern.num_tracks() && col < pattern.columns(t);
            if !shown {
                // A column hidden while it played lets go of its note.
                if self.tick == 0
                    && let Some(m) = self.tracks[ch].sounding.take()
                {
                    self.send_off(m, key);
                }
                continue;
            }
            if self.tick == 0 {
                let cell = pattern.cell(t, col, self.pos_line);
                let effects = Effects::new(&cell, pattern.track_effects(t, self.pos_line));
                let cell = self.maybe(cell, &effects);
                self.end_line_effects(ch);
                if !project.track_audible(t) || slot.is_muted(t) {
                    if let Some(m) = self.tracks[ch].sounding.take() {
                        self.send_off(m, key);
                    }
                    continue;
                }
                // The delay column, in 256ths of a line, wins over Dxx.
                let delay = match (cell.delay, effects.find(0xD)) {
                    (Some(d), _) => d as u32 * self.tpl.max(1) / 256,
                    (None, Some(d)) => d as u32,
                    _ => 0,
                };
                if delay > 0 {
                    self.tracks[ch].delayed = Some((delay, cell, effects));
                } else {
                    self.trigger(ch, cell, &effects);
                }
            } else {
                let track = self.tracks[ch];
                if let Some((at, cell, effects)) = track.delayed
                    && at == self.tick
                {
                    self.tracks[ch].delayed = None;
                    self.trigger(ch, cell, &effects);
                }
                self.tick_effects(ch);
            }
        }

        self.tick += 1;
        let line_ticks = self.line_ticks();
        if self.tick >= line_ticks && std::mem::take(&mut self.stop_after_line) {
            // The song's end: back to its start for the next play, its last
            // notes let go to ring out.
            self.tick = 0;
            self.playing = false;
            self.song_ended = true;
            self.pos_order = 0;
            self.pos_line = 0;
            self.stop_notes();
            return;
        }
        if self.tick >= line_ticks {
            self.tick = 0;
            self.pos_line += 1;
            // Leaving the end of the block loop goes back to its start.
            let looped = self.block_loop.filter(|&(order, from, to)| {
                order == self.pos_order && from <= to && self.pos_line == to.min(pattern.lines - 1) + 1
            });
            let (jump, break_to) = (self.jump.take().filter(|_| !self.loop_pattern), self.break_to.take());
            if let Some((_, from, _)) = looped {
                self.pos_line = from;
            } else if jump.is_some() || break_to.is_some() {
                // Bxx picks the slot, Jxx the line; a break alone goes on
                // to the next slot, or stays in a looping pattern.
                let next = if self.loop_pattern { self.pos_order } else { self.pos_order + 1 };
                let mut to = jump.map_or(next, |to| to.min(project.order.len() - 1));
                if to >= project.order.len() {
                    to = 0;
                    self.song_ended = true;
                }
                // Jumping back to where the song has been ends it for rendering.
                self.song_ended |= jump.is_some() && to <= self.pos_order;
                self.pos_order = to;
                self.pos_line = break_to.unwrap_or(0);
            } else if self.pos_line >= pattern.lines {
                self.pos_line = 0;
                if !self.loop_pattern {
                    self.pos_order += 1;
                    if self.pos_order >= project.order.len() {
                        // The song starts over; rendering
                        // stops here.
                        self.pos_order = 0;
                        self.song_ended = true;
                    }
                }
            }
        }
    }

    fn send_off(&mut self, module: u8, key: u32) {
        self.note(module, key, NoteEv::Off);
    }

    /// Ends track `t`'s per-line effects.
    fn end_line_effects(&mut self, t: usize) {
        let mut track = self.tracks[t];
        track.end_line(&mut |m, ev| self.track_note(m, t, ev));
        self.tracks[t] = track;
    }

    /// Sends a note event of channel `ch` to module `m`, its velocity
    /// scaled by its track's volume.
    fn track_note(&mut self, m: u8, ch: usize, ev: NoteEv) {
        let v = self.track_volume[ch % MAX_TRACKS];
        let ev = match ev {
            NoteEv::On(note, vel) => NoteEv::On(note, vel * v),
            NoteEv::Vel(vel) => NoteEv::Vel(vel * v),
            ev => ev,
        };
        self.note(m, ch as u32, ev);
    }

    /// Sets track `t`'s volume from an Lxx, moving the notes it has
    /// sounding.
    fn set_track_volume(&mut self, t: usize, arg: u8) {
        let v = arg.min(0x80) as f32 / 128.0;
        if self.track_volume[t] == v {
            return;
        }
        self.track_volume[t] = v;
        for ch in (0..MAX_COLUMNS).map(|c| c * MAX_TRACKS + t) {
            if let Some(m) = self.tracks[ch].sounding {
                self.track_note(m, ch, NoteEv::Vel(self.tracks[ch].sent_vel));
            }
        }
    }

    /// Playback starting partway through the song: each column's last
    /// note before the start plays from where it would be by now, if it
    /// goes to a sample with autoseek.
    fn autoseek(&mut self) {
        let project = self.project.clone();
        let Some(start) = project.order.get(self.pos_order) else { return };
        let Some(pattern) = project.patterns.get(start.pattern) else { return };
        let samples_per_line = self.samples_per_tick() * self.tpl.max(1) as f64;
        let wants = |m: u8| {
            project.module(m).is_some_and(|m| m.kind == ModuleKind::Sampler && m.samples.iter().any(|s| s.autoseek))
        };
        if !project.modules.iter().any(|m| wants(m.id)) {
            return;
        }
        for t in 0..pattern.num_tracks() {
            if !project.track_audible(t) || start.is_muted(t) {
                continue;
            }
            for col in 0..pattern.columns(t) {
                let Some((cell, ago)) = self.last_note(t, col) else { continue };
                if !cell.module.is_some_and(wants) {
                    continue;
                }
                let ch = col * MAX_TRACKS + t;
                // The note alone, with its sample offset: its other effects
                // have had their time.
                let effects = Effects::new(&Cell { fx: cell.fx.filter(|f| f.0 == 0x9), ..cell }, std::iter::empty());
                self.trigger(ch, cell, &effects);
                if let Some(m) = self.tracks[ch].sounding {
                    self.note(m, ch as u32, NoteEv::Seek(ago as f64 * samples_per_line));
                }
            }
        }
    }

    /// The last note in column `col` of track `t` before the play position,
    /// with the instrument it plays (from an earlier line if it names none),
    /// and how many lines ago it was; `None` if a note-off came after it.
    fn last_note(&self, t: usize, col: usize) -> Option<(Cell, usize)> {
        let project = &self.project;
        let mut note: Option<(Cell, usize)> = None;
        let mut ago = 0;
        for order in (0..=self.pos_order).rev() {
            let pattern = project.patterns.get(project.order.get(order)?.pattern)?;
            let end = if order == self.pos_order { self.pos_line.min(pattern.lines) } else { pattern.lines };
            for line in (0..end).rev() {
                ago += 1;
                if t >= pattern.num_tracks() || col >= pattern.columns(t) {
                    continue;
                }
                let cell = pattern.cell(t, col, line);
                match (&mut note, cell.note) {
                    (None, Some(Note::Off)) => return None,
                    (None, Some(Note::On(_))) => note = Some((cell, ago)),
                    _ => {}
                }
                if let Some((found, _)) = &mut note {
                    found.module = found.module.or(cell.module);
                    if found.module.is_some() {
                        return note;
                    }
                }
            }
        }
        None
    }

    fn trigger(&mut self, t: usize, cell: Cell, effects: &Effects) {
        let key = t as u32;
        let mut track = self.tracks[t];
        // Notes sent to effects are ignored.
        let plays = cell.module.or(track.module).is_some_and(|m| self.plays_notes(m));
        let picked = effects.find(FX_PHRASE);
        if let Some(l) = effects.find(FX_TRACK_VOLUME) {
            self.set_track_volume(t % MAX_TRACKS, l);
        }
        track.trigger(cell, effects, plays, &mut |m, ev| {
            if let NoteEv::On(..) = ev {
                self.picked_phrase = picked;
            }
            self.track_note(m, key as usize, ev);
            self.picked_phrase = None;
        });
        self.tracks[t] = track;
        for fx in effects.iter() {
            match fx {
                (0xB, a) => self.jump = Some(a as usize),
                (FX_BREAK, a) => self.break_to = Some(a as usize),
                (FX_WAIT, a) => self.wait = self.wait.max(a as u32),
                // F00 ends the song after this line, as in ProTracker.
                (0xF, 0) => self.stop_after_line = true,
                (0xF, a) if a >= 0x20 => self.bpm = a as f32,
                (0xF, a) if a > 0 => self.tpl = a as u32,
                _ => {}
            }
        }
    }

    fn tick_effects(&mut self, t: usize) {
        let (tick, mut track) = (self.tick, self.tracks[t]);
        track.tick_effects(tick, &mut |m, ev| self.track_note(m, t, ev));
        self.tracks[t] = track;
    }

    // ------------------------------------------------------------ rendering

    /// Sets the parameters that the playing pattern's envelopes control,
    /// for the block about to be rendered.
    fn automate(&mut self) {
        for node in &mut self.nodes {
            node.automated = false;
            node.mix_gain = None;
            node.mix_pan = None;
            node.macros = [None; MACROS];
        }
        self.live.clear();
        if !self.playing {
            return;
        }
        let project = self.project.clone();
        let Some(pattern) = project.order.get(self.shown.0).and_then(|s| project.patterns.get(s.pattern)) else {
            return;
        };
        let pos = self.pattern_line() as f32;
        for env in &pattern.automation {
            let Some(module) = project.module(env.module) else { continue };
            let (Some(spec), Some(t)) = (module.kind.automatable(env.param), env.value_in(pos, pattern.lines)) else {
                continue;
            };
            let value = spec.value_at(t);
            // The module and its copies for tracks with effects.
            for node in self.nodes.iter_mut().filter(|n| n.id == env.module) {
                node.set_param(module, env.param, value);
            }
            if self.live.len() < MAX_AUTOMATED {
                self.live.push((env.module, env.param, value));
            }
        }
    }

    /// Where in the pattern playing this block starts, in lines.
    fn pattern_line(&self) -> f64 {
        let into_tick = (1.0 - self.samples_to_tick / self.samples_per_tick()).clamp(0.0, 1.0);
        self.shown.1 as f64 + (self.shown_tick as f64 + into_tick) / self.line_ticks() as f64
    }

    /// While the song plays, where this block starts in it, in lines from
    /// its start: the slots before the one playing in full, then the lines
    /// of its pattern played. LFOs synced to lines or beats follow it, so
    /// they keep to the beat wherever playback started.
    fn song_line(&self) -> Option<f64> {
        if !self.playing {
            return None;
        }
        let project = &self.project;
        let before = project.order.iter().take(self.shown.0).filter_map(|s| project.patterns.get(s.pattern));
        Some(before.map(|p| p.lines).sum::<usize>() as f64 + self.pattern_line())
    }

    /// Runs Modulator `i` for a block of `frames` and moves the parameters
    /// it controls, on top of the song's value or the envelope's.
    #[allow(clippy::too_many_arguments)]
    fn modulate(
        nodes: &mut [Node],
        live: &mut Vec<(u8, usize, f32)>,
        project: &Project,
        ctx: &Ctx,
        i: usize,
        frames: usize,
        input_peak: f32,
    ) {
        let node = &nodes[i];
        let module = &project.modules[node.module];
        let p = if node.automated { &node.params } else { &module.params };
        let (mode, shape, rate, sync, period, amount, attack, release) =
            (p[0].round() as u32, p[1].round() as u32, p[2], p[3].round() as u32, p[4], p[5], p[6], p[7]);
        let beats = crate::project::MODULATOR_BEAT_LENGTHS[(p[8].round() as usize).min(9)];
        // The last note played on what it follows, for Key and Velocity.
        let last = || {
            let notes = nodes[i].inputs.iter().map(|&j| nodes[j].last_note);
            notes.filter(|n| n.2 > 0).max_by_key(|n| n.2)
        };
        let followed = match mode {
            0 => None,
            1 => Some(input_peak.min(1.0)),
            // Up or down from C-4 by up to four octaves.
            2 => Some(last().map_or(0.0, |n| ((n.0 - 48.0) / 48.0).clamp(-1.0, 1.0))),
            3 => Some(last().map_or(0.0, |n| n.1)),
            // A new note starts the envelope: up over the attack, then down
            // over the release. `phase` counts the seconds since it began.
            4 => {
                let started = last().map_or(0, |n| n.2);
                let node = &mut nodes[i];
                if started != node.last_note.2 {
                    node.last_note.2 = started;
                    node.phase = 0.0;
                } else {
                    node.phase += frames as f32 / ctx.sr;
                }
                let up = (node.phase / attack.max(1e-4)).min(1.0);
                let down = ((node.phase - attack).max(0.0) / release.max(1e-4)).min(1.0);
                // The envelope itself moves; it skips the follower.
                node.follow = if started == 0 { 0.0 } else { up * (1.0 - down) };
                None
            }
            _ => Some(1.0),
        };
        let node = &mut nodes[i];
        let offset = if mode == 4 {
            amount * node.follow
        } else if let Some(target) = followed {
            // Followed through the attack and release, without jumps.
            let time = if target > node.follow { attack } else { release };
            let coef = 1.0 - (-(frames as f32) / (time.max(1e-4) * ctx.sr)).exp();
            node.follow += coef * (target - node.follow);
            amount * node.follow
        } else {
            let lines = match sync {
                0 => None,
                1 => Some(period),
                _ => Some(beats * project.lpb.max(1) as f32),
            };
            // Synced, its cycle follows the song while it plays.
            if let (Some(l), Some(at)) = (lines, ctx.song_line) {
                node.phase = (at / l.max(1e-3) as f64).rem_euclid(1.0) as f32;
            }
            let inc = match lines {
                Some(l) => frames as f32 / (l * ctx.samples_per_line).max(1.0),
                None => rate * frames as f32 / ctx.sr,
            };
            let w = if shape == crate::project::DRAWN_SHAPE {
                let points =
                    if module.shape.is_empty() { &crate::project::DEFAULT_SHAPE[..] } else { &module.shape[..] };
                crate::project::interpolate(points, node.phase, false, false).map_or(0.0, |v| 2.0 * v - 1.0)
            } else {
                dsp::lfo_shape(shape, node.phase)
            };
            node.phase = (node.phase + inc).fract();
            amount * w * 0.5
        };
        if module.mute {
            return;
        }
        for k in 0..nodes[i].controls.len() {
            let (j, param) = nodes[i].controls[k];
            let target = &mut nodes[j];
            let m = &project.modules[target.module];
            let Some(spec) = m.kind.automatable(param) else { continue };
            let value = spec.value_at(spec.position(target.param(m, param)) + offset);
            target.set_param(m, param, value);
            if live.len() < MAX_AUTOMATED {
                live.push((m.id, param, value));
            }
        }
    }

    fn render_block(&mut self, out: &mut [Frame]) {
        let n = out.len();
        let samples_per_line = (self.samples_per_tick() * self.tpl.max(1) as f64) as f32;
        let ctx = Ctx { sr: self.sr, samples_per_line, song_line: self.song_line() };
        let project = &self.project;
        for oi in 0..self.order.len() {
            let i = self.order[oi];
            let scratch = &mut self.scratch[..n];
            scratch.fill([0.0; 2]);
            for &j in &self.nodes[i].inputs {
                for (s, x) in scratch.iter_mut().zip(&self.nodes[j].buf[..n]) {
                    s[0] += x[0];
                    s[1] += x[1];
                }
            }
            if !self.nodes[i].key.is_empty() {
                let key = &mut self.key_scratch[..n];
                key.fill([0.0; 2]);
                for &j in &self.nodes[i].key {
                    for (s, x) in key.iter_mut().zip(&self.nodes[j].buf[..n]) {
                        s[0] += x[0];
                        s[1] += x[1];
                    }
                }
            }
            // An instrument's macros set what they move before it plays.
            for k in 0..self.nodes[i].macro_targets.len() {
                let (mac, j, param, from, to) = self.nodes[i].macro_targets[k];
                let m = &project.modules[self.nodes[i].module];
                let turned = self.nodes[i].param(m, m.params.len() + 2 + mac);
                let target = &project.modules[self.nodes[j].module];
                let Some(spec) = target.kind.automatable(param) else { continue };
                let value = spec.value_at(from + (to - from) * turned);
                self.nodes[j].set_param(target, param, value);
                if self.live.len() < MAX_AUTOMATED {
                    self.live.push((target.id, param, value));
                }
            }
            if self.nodes[i].kind.controls() {
                let input_peak = scratch.iter().fold(0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
                Self::modulate(&mut self.nodes, &mut self.live, project, &ctx, i, n, input_peak);
            }
            let node = &mut self.nodes[i];
            let module = &project.modules[node.module];
            let params = if node.automated { &node.params } else { &module.params };
            if module.bypass && module.kind.has_input() {
                // A switched-off effect lets its input through.
                node.buf[..n].copy_from_slice(scratch);
            } else if node.key.is_empty() {
                node.dsp.process(&ctx, params, scratch, &mut node.buf[..n]);
            } else {
                node.dsp.process_keyed(&ctx, params, scratch, &self.key_scratch[..n], &mut node.buf[..n]);
            }
            let silent = module.mute || !node.audible;
            if let Some(tap) = node.dsp.track_tap() {
                if !silent {
                    let mut used = tap.used;
                    while used != 0 {
                        let t = used.trailing_zeros() as usize;
                        for (sum, x) in self.track_block[t][..n].iter_mut().zip(&tap.sums[t][..n]) {
                            *sum += x;
                        }
                        used &= used - 1;
                    }
                }
                tap.clear();
            }
            let buf = &mut node.buf[..n];
            let (gain, pan) = (node.mix_gain.unwrap_or(module.gain), node.mix_pan.unwrap_or(module.pan));
            if module.mute || !node.audible {
                buf.fill([0.0; 2]);
            } else if gain != 1.0 || pan != 0.0 {
                let (l, r) = balance(pan);
                for f in buf.iter_mut() {
                    f[0] *= gain * l;
                    f[1] *= gain * r;
                }
            }
            for f in buf.iter() {
                node.peak[0] = node.peak[0].max(f[0].abs());
                node.peak[1] = node.peak[1].max(f[1].abs());
            }
        }
        match self.output {
            Some(o) => out.copy_from_slice(&self.nodes[o].buf[..n]),
            None => out.fill([0.0; 2]),
        }
        // A track with effects shows them in its scope: their sound.
        for &(t, j) in &self.track_ends {
            for (s, f) in self.track_block[t][..n].iter_mut().zip(&self.nodes[j].buf[..n]) {
                *s = (f[0] + f[1]) * 0.5;
            }
        }
        for k in 0..n {
            for (t, block) in self.track_block.iter().enumerate() {
                self.track_rings[t * TRACK_SCOPE_LEN + self.track_pos] = block[k];
            }
            self.track_pos = (self.track_pos + 1) % TRACK_SCOPE_LEN;
        }
        for block in &mut self.track_block {
            block[..n].fill(0.0);
        }
    }

    /// Renders `out.len()` frames, running the sequencer on tick boundaries.
    pub fn render(&mut self, out: &mut [Frame]) {
        let started = std::time::Instant::now();
        crate::dsp::flush_denormals();
        if let Some(rx) = self.rx.take() {
            while let Ok(cmd) = rx.try_recv() {
                self.handle(cmd);
            }
            self.rx = Some(rx);
        }

        let mut done = 0;
        while done < out.len() {
            if self.playing && self.samples_to_tick <= 0.0 {
                self.run_tick();
                self.samples_to_tick += self.tick_wait();
            }
            let mut n = (out.len() - done).min(BLOCK);
            if self.playing {
                n = n.min(self.samples_to_tick.ceil().max(1.0) as usize);
            }
            if self.gliding() {
                n = n.min(GLIDE_STEP);
            }
            self.automate();
            self.run_phrases(n);
            self.run_glides(n);
            self.render_block(&mut out[done..done + n]);
            self.render_preview(&mut out[done..done + n]);
            self.render_click(&mut out[done..done + n]);
            if self.playing {
                self.samples_to_tick -= n as f64;
                self.played += n as u64;
            }
            done += n;
        }

        let mut peak = [0f32; 2];
        for f in out.iter_mut() {
            for ch in 0..2 {
                // Guard against blowups from extreme parameter settings.
                if !f[ch].is_finite() {
                    f[ch] = 0.0;
                }
                peak[ch] = peak[ch].max(f[ch].abs());
            }
            self.scope[self.scope_pos] = *f;
            self.scope_pos = (self.scope_pos + 1) % SCOPE_LEN;
        }
        self.shared.resample.push(out);
        let load = started.elapsed().as_secs_f32() * self.sr / out.len().max(1) as f32;
        self.publish(peak, load, out.len());
    }

    fn publish(&mut self, peak: [f32; 2], load: f32, frames: usize) {
        let s = &self.shared;
        let fall = (-(frames as f32) / (self.sr * METER_FALL)).exp();
        // A module's meter shows the loudest of it and its copies.
        let mut levels = [[0f32; 2]; 256];
        for node in &mut self.nodes {
            for (l, peak) in levels[node.id as usize].iter_mut().zip(&mut node.peak) {
                let level = if peak.is_finite() { *peak } else { 0.0 };
                *l = l.max(level);
                *peak = level * fall;
            }
        }
        for node in self.nodes.iter().filter(|n| n.track.is_none()) {
            let at = 2 * node.id as usize;
            for (out, l) in s.levels[at..at + 2].iter().zip(levels[node.id as usize]) {
                out.store(l.to_bits(), Ordering::Relaxed);
            }
        }
        let time = (self.played as f64 / self.sr as f64) as f32;
        s.time.store(time.to_bits(), Ordering::Relaxed);
        let cpu = f32::from_bits(s.cpu.load(Ordering::Relaxed));
        s.cpu.store((cpu + (load - cpu) * 0.05).to_bits(), Ordering::Relaxed);
        s.playing.store(self.playing, Ordering::Relaxed);
        s.order.store(self.shown.0, Ordering::Relaxed);
        s.line.store(self.shown.1, Ordering::Relaxed);
        let into_tick = (1.0 - self.samples_to_tick / self.samples_per_tick()).clamp(0.0, 1.0) as f32;
        let frac = ((self.shown_tick as f32 + into_tick) / self.tpl.max(1) as f32).clamp(0.0, 0.999);
        s.line_frac.store(frac.to_bits(), Ordering::Relaxed);
        s.bpm.store(self.bpm.to_bits(), Ordering::Relaxed);
        for (p, out) in peak.iter().zip(&s.peak) {
            let old = f32::from_bits(out.load(Ordering::Relaxed));
            out.store(p.max(old * 0.9).to_bits(), Ordering::Relaxed);
        }
        if let Ok(mut scope) = s.scope.try_lock() {
            let (a, b) = self.scope.split_at(self.scope_pos);
            scope[..b.len()].copy_from_slice(b);
            scope[b.len()..].copy_from_slice(a);
        }
        let preview = self.preview.as_ref().map_or(-1.0, |p| p.1 as f32);
        s.preview_pos.store(preview.to_bits(), Ordering::Relaxed);

        self.playheads.clear();
        for n in self.nodes.iter().filter(|n| n.kind == ModuleKind::Sampler) {
            let from = self.playheads.len();
            n.dsp.playheads(&mut self.playheads);
            for p in &mut self.playheads[from..] {
                p.module = n.id;
            }
        }
        self.playheads.truncate(MAX_PLAYHEADS);
        if let Ok(mut out) = s.playheads.try_lock() {
            out.clear();
            out.extend_from_slice(&self.playheads);
        }
        if let Ok(mut out) = s.track_scopes.try_lock() {
            let p = self.track_pos;
            for (dst, src) in out.chunks_mut(TRACK_SCOPE_LEN).zip(self.track_rings.chunks(TRACK_SCOPE_LEN)) {
                dst[..TRACK_SCOPE_LEN - p].copy_from_slice(&src[p..]);
                dst[TRACK_SCOPE_LEN - p..].copy_from_slice(&src[..p]);
            }
        }
        if let Ok(mut out) = s.automated.try_lock() {
            out.clear();
            out.extend_from_slice(&self.live);
        }
        if let Ok(mut out) = s.phrases.try_lock() {
            out.clear();
            out.extend(self.phrases.iter().map(|p| (p.module, p.phrase, p.playing)));
        }
    }
}

/// Left and right gains that turn a stereo signal towards `pan` (-1..1)
/// without making the near side louder.
impl Node {
    /// Automatable parameter `param` of the node's module `m` (see
    /// `ModuleKind::automatable`) as it is this block: the song's value
    /// or what an envelope, Modulator or macro set.
    fn param(&self, m: &crate::project::Module, param: usize) -> f32 {
        let n = m.params.len();
        match param {
            p if p < n && self.automated => self.params[p],
            p if p < n => m.params[p],
            p if p == n => self.mix_gain.unwrap_or(m.gain),
            p if p == n + 1 => self.mix_pan.unwrap_or(m.pan),
            p => self.macros.get(p - n - 2).copied().flatten().unwrap_or_else(|| m.macro_value(p - n - 2)),
        }
    }

    /// Sets automatable parameter `param` for this block.
    fn set_param(&mut self, m: &crate::project::Module, param: usize, value: f32) {
        let n = m.params.len();
        match param {
            p if p < n => {
                if !self.automated {
                    self.params.clear();
                    self.params.extend_from_slice(&m.params);
                    self.automated = true;
                }
                self.params[p] = value;
            }
            p if p == n => self.mix_gain = Some(value),
            p if p == n + 1 => self.mix_pan = Some(value),
            p => {
                if let Some(v) = self.macros.get_mut(p - n - 2) {
                    *v = Some(value);
                }
            }
        }
    }
}

fn balance(pan: f32) -> (f32, f32) {
    ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{MacroTarget, Owner, Phrase};

    const SR: f32 = 8000.0;

    fn engine(project: Project) -> Engine {
        Engine::new(SR, Arc::new(project), None, None, Arc::new(Shared::default()))
    }

    #[test]
    fn previews_play_at_their_volume_and_loop_until_stopped() {
        let mut p = Project::empty();
        p.modules[0].params[0] = 1.0;
        let mut e = engine(p);
        let sample = Sample { name: "click".into(), sample_rate: SR, channels: 2, frames: vec![[1.0; 2], [0.0; 2]] };
        e.handle(Cmd::PreviewSettings { volume: 0.5, looping: true });
        e.handle(Cmd::Preview(Some(Arc::new(sample))));
        let mut out = vec![[0.0; 2]; 6];
        e.render(&mut out);
        let left: Vec<f32> = out.iter().map(|f| f[0]).collect();
        assert_eq!(left, [0.5, 0.0, 0.5, 0.0, 0.5, 0.0], "half as loud, over and over");
        e.handle(Cmd::PreviewSettings { volume: 0.5, looping: false });
        let mut out = vec![[0.0; 2]; 6];
        e.render(&mut out);
        assert_eq!(out.iter().filter(|f| f[0] != 0.0).count(), 0, "once it reaches the end, it stops");
        // A sample at a much higher rate than the output steps over its
        // whole length at once, and still loops.
        let fast = Sample { name: "fast".into(), sample_rate: SR * 5.0, channels: 2, frames: vec![[1.0; 2], [0.0; 2]] };
        e.handle(Cmd::PreviewSettings { volume: 1.0, looping: true });
        e.handle(Cmd::Preview(Some(Arc::new(fast))));
        e.render(&mut out);
    }

    #[test]
    fn the_output_goes_onto_the_resample_tape_while_it_is_on() {
        let shared = Arc::new(Shared::default());
        let mut e = Engine::new(SR, Arc::new(Project::demo()), None, None, shared.clone());
        e.play_song();
        let mut out = vec![[0.0; 2]; 256];
        e.render(&mut out);
        let mut got = Vec::new();
        shared.resample.take(&mut got);
        assert!(got.is_empty(), "nothing before it starts");
        shared.resample.start(1024);
        for _ in 0..3 {
            e.render(&mut out);
        }
        assert!(!shared.resample.take(&mut got));
        assert_eq!(got.len(), 768);
        assert_eq!(got[512..], out[..], "the frames played, in order");
    }

    /// Renders `frames` frames, in pieces the size of an audio callback,
    /// and returns the left channel.
    fn render(e: &mut Engine, frames: usize) -> Vec<f32> {
        let mut out = vec![[0.0; 2]; frames];
        for chunk in out.chunks_mut(256) {
            e.render(chunk);
        }
        out.iter().map(|f| f[0]).collect()
    }

    fn loudness(x: &[f32]) -> f32 {
        x.iter().fold(0.0, |m, v| m.max(v.abs()))
    }

    #[test]
    fn the_master_chain_processes_the_whole_mix() {
        let mut p = Project::empty();
        p.modules[0].params[0] = 1.0;
        let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
        p.connect(synth, OUTPUT_ID);
        let mut e = engine(p.clone());
        e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note: 60, vel: 1.0 });
        let dry = loudness(&render(&mut e, 4000)[2000..]);
        assert!(dry > 0.05);
        let amp = p.chain_insert(OUTPUT_ID, 0, ModuleKind::Amplifier).unwrap();
        p.module_mut(amp).unwrap().params[0] = 0.5;
        let mut e = engine(p);
        e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note: 60, vel: 1.0 });
        let through = loudness(&render(&mut e, 4000)[2000..]);
        assert!((through / dry - 0.5).abs() < 0.02, "half as loud through the master Amplifier: {dry} {through}");
    }

    #[test]
    fn songs_loop_unless_f00_ends_them() {
        let mut p = Project::empty();
        p.modules[0].params[0] = 1.0;
        let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
        p.connect(synth, OUTPUT_ID);
        p.patterns[0].lines = 4;
        p.patterns[0].tracks[0][0] = Cell { note: Some(Note::On(60)), module: Some(synth), ..Cell::default() };
        let line = 6 * TICK;
        for stop in [false, true] {
            let mut p = p.clone();
            if stop {
                assert!(p.end_with_stop());
                assert_eq!(p.patterns[0].tracks[0][3].fx, Some((0xF, 0)), "on the last line");
            }
            let mut e = engine(p);
            e.play_song();
            render(&mut e, 4 * line + 64);
            assert_eq!(e.playing, !stop, "F00 {stop}");
            assert!(e.song_ended, "a render stops after the song either way");
            if stop {
                assert_eq!((e.pos_order, e.pos_line), (0, 0), "back at the start for the next play");
                assert!(loudness(&render(&mut e, 64)) > 0.0, "the last note rings out as it is let go");
            }
        }
    }

    #[test]
    fn a_tracks_effects_change_only_what_it_plays() {
        let mut p = Project::empty();
        p.modules[0].params[0] = 1.0;
        let synth = p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap();
        p.connect(synth, OUTPUT_ID);
        // The instrument's own effect halves it, wherever it plays.
        let own = p.chain_insert(synth, 0, ModuleKind::Amplifier).unwrap();
        p.module_mut(own).unwrap().params[0] = 0.5;
        // Both tracks play it; track 1 also halves what it plays.
        let on = |m| Cell { note: Some(Note::On(60)), module: Some(m), ..Cell::default() };
        p.patterns[0].tracks[0][0] = on(synth);
        p.patterns[0].tracks[1][0] = on(synth);
        let level = |p: &Project, track: usize| {
            let mut p = p.clone();
            p.patterns[0].tracks[1 - track][0] = Cell::default();
            let mut e = engine(p);
            e.play_song();
            loudness(&render(&mut e, 4000)[2000..])
        };
        let (plain, before) = (level(&p, 0), level(&p, 1));
        assert!((plain / before - 1.0).abs() < 0.01, "the same without track effects");
        let fx = p.chain_insert(Owner::Track(1), 0, ModuleKind::Amplifier).unwrap();
        p.module_mut(fx).unwrap().params[0] = 0.5;
        assert!(!p.links.iter().any(|l| l.0 == fx || l.1 == fx), "placed, not linked");
        let (other, through) = (level(&p, 0), level(&p, 1));
        assert!((other / plain - 1.0).abs() < 0.01, "track 0 is as it was: {other} {plain}");
        assert!((through / plain - 0.5).abs() < 0.01, "track 1 goes through its effect too: {through} {plain}");
    }

    #[test]
    fn metronome_clicks_on_beats() {
        let mut p = Project::empty();
        p.bpm = 120.0;
        p.lpb = 4;
        let mut e = engine(p);
        e.handle(Cmd::Metronome(true));
        e.play_song();
        // At 120 BPM a beat is half a second.
        let out = render(&mut e, SR as usize);
        let beat = SR as usize / 2;
        assert!(loudness(&out[..200]) > 0.2, "click on the first beat");
        assert!(loudness(&out[beat / 2..beat]) < 0.001, "silent between beats");
        assert!(loudness(&out[beat..beat + 200]) > 0.2, "click on the second beat");

        // Off, an empty song is silent.
        let mut e = engine(Project::empty());
        e.play_song();
        assert_eq!(loudness(&render(&mut e, SR as usize)), 0.0);
    }

    /// Two generators into the output, each holding a note.
    fn two_synths() -> (Project, u8, u8) {
        let mut p = Project::empty();
        let a = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        let b = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        p.connect(a, OUTPUT_ID);
        p.connect(b, OUTPUT_ID);
        (p, a, b)
    }

    fn hold_notes(e: &mut Engine, ids: &[u8]) {
        for &id in ids {
            e.handle(Cmd::NoteOn { module: id, key: LIVE_KEY, note: 60, vel: 1.0 });
        }
    }

    #[test]
    fn a_key_input_lets_another_sound_work_the_compressor() {
        let (mut p, a, b) = two_synths();
        p.disconnect(a, OUTPUT_ID);
        // A heavy compressor on a, keyed by b.
        let comp = p.add_module(ModuleKind::Compressor, [0.0, 0.0]).unwrap();
        p.connect(a, comp);
        p.connect(comp, OUTPUT_ID);
        p.module_mut(comp).unwrap().params = vec![0.02, 20.0, 0.0001, 0.05, 1.0, 1.0];
        // b is only heard through the compressor's key input.
        p.disconnect(b, OUTPUT_ID);
        let level = |p: &Project, notes: &[u8]| {
            let mut e = engine(p.clone());
            hold_notes(&mut e, notes);
            render(&mut e, 4000);
            loudness(&render(&mut e, 4000))
        };
        // On its own input it squashes a's note.
        let own = level(&p, &[a]);
        p.module_mut(comp).unwrap().key = Some(b);
        assert!(p.can_key(comp, b));
        // Keyed by a silent b it lets a through untouched; b playing ducks it.
        let quiet_key = level(&p, &[a]);
        let ducked = level(&p, &[a, b]);
        assert!(quiet_key > 3.0 * own, "{quiet_key} {own}");
        assert!(ducked < 0.5 * quiet_key, "{ducked} {quiet_key}");
        // What the compressor feeds can't key it: that would loop.
        let after = p.add_module(ModuleKind::Filter, [0.0, 0.0]).unwrap();
        p.disconnect(comp, OUTPUT_ID);
        p.connect(comp, after);
        p.connect(after, OUTPUT_ID);
        assert!(!p.can_key(comp, after));
        assert!(!p.can_key(comp, comp));
    }

    #[test]
    fn macros_move_their_instruments_parameters() {
        let (mut p, a, b) = two_synths();
        p.disconnect(b, OUTPUT_ID);
        let level = |p: &Project| {
            let mut e = engine(p.clone());
            hold_notes(&mut e, &[a]);
            render(&mut e, 2000);
            loudness(&render(&mut e, 2000))
        };
        let full = level(&p);
        // Macro 1 turns the Generator's volume from nothing to a quarter.
        let m = p.module_mut(a).unwrap();
        m.macro_mut(0).targets.push(MacroTarget { module: a, param: 0, from: 0.0, to: 0.25 });
        assert_eq!(level(&p), 0.0, "turned down, it silences the synth");
        p.module_mut(a).unwrap().macro_mut(0).value = 1.0;
        let half = level(&p);
        assert!(half > 0.1 && half < full, "{half} {full}");
        // A Modulator can turn the macro, as it can any parameter.
        p.module_mut(a).unwrap().macro_mut(0).value = 0.0;
        let n = ModuleKind::Generator.params().len();
        assert_eq!(p.module(a).unwrap().automatable_name(n + 2), "Macro 1");
        let knob = p.add_module(ModuleKind::Modulator, [0.0, 0.0]).unwrap();
        p.module_mut(knob).unwrap().params[0] = 5.0;
        p.module_mut(knob).unwrap().params[5] = 1.0;
        p.connect(knob, a);
        p.set_control_param(knob, a, n + 2);
        assert!(level(&p) > 0.1, "the Modulator turns it up");
    }

    #[test]
    fn mixer_gain_pan_and_meters() {
        let (mut p, a, b) = two_synths();
        p.module_mut(b).unwrap().mute = true;
        let mut e = engine(p.clone());
        hold_notes(&mut e, &[a, b]);
        render(&mut e, 2000);
        let full = e.shared.level(a);
        assert!(full[0] > 0.1 && (full[0] - full[1]).abs() < 1e-3, "{full:?}");
        assert_eq!(e.shared.level(b), [0.0, 0.0], "muted modules read silent");

        // Half the fader, panned hard right.
        p.module_mut(a).unwrap().gain = 0.5;
        p.module_mut(a).unwrap().pan = 1.0;
        e.handle(Cmd::Project(Arc::new(p)));
        render(&mut e, (SR * METER_FALL * 10.0) as usize);
        let [l, r] = e.shared.level(a);
        assert!(l < 1e-3, "left is silent: {l}");
        assert!((r - full[1] * 0.5).abs() < 0.05, "{r} vs {}", full[1]);
    }

    #[test]
    fn solo_keeps_the_chain_audible() {
        let (mut p, a, b) = two_synths();
        // a -> filter -> output; soloing the filter keeps a and the output.
        p.disconnect(a, OUTPUT_ID);
        let f = p.add_module(ModuleKind::Filter, [0.0, 0.0]).unwrap();
        p.connect(a, f);
        p.connect(f, OUTPUT_ID);
        p.module_mut(f).unwrap().solo = true;
        let mut e = engine(p);
        hold_notes(&mut e, &[a, b]);
        render(&mut e, 2000);
        assert!(e.shared.level(a)[0] > 0.01);
        assert!(e.shared.level(f)[0] > 0.01);
        assert!(e.shared.level(OUTPUT_ID)[0] > 0.01);
        assert_eq!(e.shared.level(b), [0.0, 0.0]);
    }

    impl Engine {
        fn node_mut(&mut self, id: u8) -> Option<&mut Node> {
            self.nodes.iter_mut().find(|n| n.id == id)
        }
    }

    /// Records the events the sequencer sends a module.
    #[derive(Clone, Default)]
    struct Log(Arc<Mutex<Vec<(&'static str, f32)>>>);

    impl Log {
        fn take(&self) -> Vec<(&'static str, f32)> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    struct Recorder(Log);

    impl Dsp for Recorder {
        fn note_on(&mut self, _: u32, note: f32, _: f32) {
            self.0.0.lock().unwrap().push(("on", note));
        }
        fn note_off(&mut self, _: u32) {
            self.0.0.lock().unwrap().push(("off", 0.0));
        }
        fn set_pitch(&mut self, _: u32, note: f32) {
            self.0.0.lock().unwrap().push(("pitch", note));
        }
        fn set_velocity(&mut self, _: u32, vel: f32) {
            self.0.0.lock().unwrap().push(("vel", vel));
        }
        fn set_pan(&mut self, _: u32, pan: f32) {
            self.0.0.lock().unwrap().push(("pan", pan));
        }
        fn process(&mut self, _: &Ctx, _: &[f32], _: &[Frame], out: &mut [Frame]) {
            out.fill([0.0; 2]);
        }
    }

    /// At 125 BPM, 4 lines per beat and 6 ticks per line.
    const TICK: usize = 160;

    /// Plays `cells` on track 0, sending them to a recorder.
    fn sequencer(cells: &[(usize, Cell)]) -> (Engine, Log) {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        p.connect(id, OUTPUT_ID);
        for &(line, cell) in cells {
            p.patterns[0].tracks[0][line] = Cell { module: Some(id), ..cell };
        }
        let mut e = engine(p);
        let log = Log::default();
        e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
        e.play_song();
        (e, log)
    }

    fn note(n: u8, vol: Option<u8>, fx: Option<(u8, u8)>) -> Cell {
        Cell { note: Some(Note::On(n)), module: None, vol, fx, ..Cell::default() }
    }

    fn effect(cmd: u8, arg: u8) -> Cell {
        Cell { fx: Some((cmd, arg)), ..Cell::default() }
    }

    fn values(log: &[(&str, f32)], what: &str) -> Vec<f32> {
        log.iter().filter(|e| e.0 == what).map(|e| e.1).collect()
    }

    #[test]
    fn arpeggio_and_vibrato_end_with_their_line() {
        let (mut e, log) = sequencer(&[(0, note(48, None, Some((0x0, 0x37)))), (2, note(48, None, Some((0x4, 0x48))))]);
        render(&mut e, 7 * TICK);
        let events = log.take();
        assert_eq!(values(&events, "pitch"), [51.0, 55.0, 48.0, 51.0, 55.0, 48.0], "back to the note on the next line");
        render(&mut e, 6 * TICK + 6 * TICK);
        let pitches = values(&log.take(), "pitch");
        let widest = pitches.iter().fold(0f32, |m, p| m.max((p - 48.0).abs()));
        assert!(widest > 0.5 && widest <= 1.0, "a semitone of vibrato: {pitches:?}");
        assert_eq!(*pitches.last().unwrap(), 48.0);
    }

    #[test]
    fn volume_slide_stays_and_tremolo_does_not() {
        let (mut e, log) = sequencer(&[(0, note(48, Some(0x40), Some((0xA, 0x40)))), (2, effect(0x7, 0x4F))]);
        render(&mut e, 12 * TICK);
        let vels = values(&log.take(), "vel");
        assert_eq!(vels.len(), 5, "one change per tick after the first: {vels:?}");
        assert!((vels[4] - (0.5 + 5.0 * 4.0 / 128.0)).abs() < 1e-6);
        // A line of tremolo dips and comes back.
        render(&mut e, 7 * TICK);
        let vels = values(&log.take(), "vel");
        let low = vels.iter().fold(1f32, |m, &v| m.min(v));
        assert!(low < 0.3, "{vels:?}");
        assert!((vels.last().unwrap() - 0.65625).abs() < 1e-6, "{vels:?}");
    }

    #[test]
    fn panning_stays_with_the_track() {
        let (mut e, log) =
            sequencer(&[(0, note(48, None, Some((0x8, 0x00)))), (1, effect(0x8, 0xFF)), (2, note(50, None, None))]);
        render(&mut e, 13 * TICK);
        let events = log.take();
        let pans = values(&events, "pan");
        assert_eq!(pans, [-1.0, -1.0, 1.0, 1.0]);
        let second = events.iter().rposition(|e| *e == ("on", 50.0)).unwrap();
        assert_eq!(events[second + 1], ("pan", 1.0), "the next note starts on the right");

        // And the generator really plays on one side.
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        p.connect(id, OUTPUT_ID);
        p.patterns[0].tracks[0][0] = Cell { module: Some(id), ..note(60, None, Some((0x8, 0x00))) };
        let mut e = engine(p);
        e.play_song();
        let mut out = vec![[0.0; 2]; 2000];
        e.render(&mut out);
        let side = |ch: usize| out.iter().fold(0f32, |m, f| m.max(f[ch].abs()));
        assert!(side(0) > 0.1 && side(1) < 1e-6, "{} {}", side(0), side(1));
    }

    #[test]
    fn retrigger_plays_the_note_again() {
        let (mut e, log) = sequencer(&[(0, note(48, None, Some((0xE, 0x02))))]);
        render(&mut e, 6 * TICK);
        let events = log.take();
        assert_eq!(values(&events, "on"), [48.0, 48.0, 48.0], "on ticks 0, 2 and 4");
        assert_eq!(values(&events, "off").len(), 2);
    }

    #[test]
    fn slots_mute_tracks() {
        let (mut e, log) = sequencer(&[(0, note(48, None, None))]);
        let mut p = (*e.project).clone();
        let mut muted = p.order[0];
        muted.toggle_mute(0);
        p.order = vec![muted, p.order[0]];
        p.patterns[0].lines = 2;
        e.handle(Cmd::Project(Arc::new(p)));
        e.play_song();
        render(&mut e, 12 * TICK);
        assert!(values(&log.take(), "on").is_empty(), "muted in the first slot");
        render(&mut e, TICK);
        assert_eq!(values(&log.take(), "on"), [48.0]);
    }

    #[test]
    fn position_jump_and_ticks_per_line() {
        let (mut e, _) = sequencer(&[(0, effect(0xF, 0x03)), (1, effect(0xB, 0x00))]);
        render(&mut e, TICK);
        assert_eq!(e.tpl, 3);
        // The line still lasts as long, in three longer ticks.
        render(&mut e, 5 * TICK);
        assert_eq!(e.pos_line, 1);
        render(&mut e, 6 * TICK);
        assert_eq!((e.pos_order, e.pos_line), (0, 0));
        assert!(e.song_ended, "jumping back ends the song for rendering");
    }

    #[test]
    fn a_break_goes_on_at_a_line_of_the_next_slot() {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        p.connect(id, OUTPUT_ID);
        p.order.push(crate::project::Slot::new(0));
        p.patterns[0].tracks[0][0] = effect(FX_BREAK, 0x05);
        let mut e = engine(p);
        e.play_song();
        render(&mut e, 6 * TICK + 10);
        assert_eq!(e.shown, (1, 5));
        assert!(!e.song_ended);
        // The rest of that slot plays, then the song starts over.
        render(&mut e, (64 - 5) * 6 * TICK);
        assert_eq!(e.shown, (0, 0));
        assert!(e.song_ended);
    }

    #[test]
    fn a_wait_holds_the_line_while_its_effects_go_on() {
        let (mut e, log) = sequencer(&[(0, note(48, None, Some((FX_WAIT, 0x02)))), (1, note(50, None, None))]);
        render(&mut e, 6 * TICK + 10);
        assert_eq!(e.shown, (0, 0));
        render(&mut e, 12 * TICK);
        assert_eq!(e.shown, (0, 1), "three lines' time in all");
        render(&mut e, 6 * TICK);
        assert_eq!(e.shown, (0, 2), "the next line is as long as ever");
        assert_eq!(values(&log.take(), "on"), [48.0, 50.0]);
    }

    #[test]
    fn volume_column_commands_act_as_effects() {
        let vc = |c, x| crate::project::vol_command_value(c, x);
        // A fade out starts at full volume, not at the command's value.
        let (mut e, log) = sequencer(&[(0, note(48, vc('O', 4), None)), (1, Cell::default())]);
        render(&mut e, 6 * TICK + 10);
        let vels = values(&log.take(), "vel");
        assert_eq!(vels.len(), 5);
        assert!((vels[0] - (1.0 - 4.0 / 128.0)).abs() < 1e-6 && vels.windows(2).all(|w| w[1] < w[0]), "{vels:?}");
        // G4 glides a semitone a tick.
        let (mut e, log) = sequencer(&[(0, note(48, None, None)), (1, note(52, vc('G', 4), None))]);
        render(&mut e, 12 * TICK + 10);
        assert_eq!(values(&log.take(), "pitch"), [49.0, 50.0, 51.0, 52.0]);
    }

    #[test]
    fn track_volume_scales_the_track_s_notes() {
        let (mut e, log) = sequencer(&[
            (0, note(48, Some(0x40), None)),
            (1, effect(FX_TRACK_VOLUME, 0x40)),
            (2, Cell { vol: Some(0x80), ..Cell::default() }),
        ]);
        render(&mut e, 18 * TICK + 10);
        // Half the track's volume halves the note playing, and a new
        // volume on the line after.
        assert_eq!(values(&log.take(), "vel"), [0.25, 0.5]);
    }

    #[test]
    fn tremor_switches_the_note_on_and_off_by_ticks() {
        let (mut e, log) = sequencer(&[(0, note(48, None, Some((FX_TREMOR, 0x21)))), (1, Cell::default())]);
        render(&mut e, 6 * TICK + 10);
        // On for ticks 0 and 1, off for 2, on for 3 and 4, off for 5, then
        // back on as the line ends.
        assert_eq!(values(&log.take(), "vel"), [0.0, 1.0, 0.0, 1.0]);
    }

    /// Keeps the last value of parameter 1 that a module processed with.
    struct Probe(Arc<Mutex<f32>>);

    impl Dsp for Probe {
        fn process(&mut self, _: &Ctx, params: &[f32], _: &[Frame], out: &mut [Frame]) {
            *self.0.lock().unwrap() = params[1];
            out.fill([0.0; 2]);
        }
    }

    #[test]
    fn envelopes_move_parameters_while_playing() {
        let mut p = Project::empty();
        let f = p.add_module(ModuleKind::Filter, [0.0, 0.0]).unwrap();
        p.connect(f, OUTPUT_ID);
        // Cutoff from 20 Hz at line 0 to 20 kHz at line 4.
        let points = vec![(0.0, 0.0), (4.0, 1.0)];
        p.patterns[0].automation.push(crate::project::Envelope::new(f, 1, points));
        let mut e = engine(p);
        let cutoff = Arc::new(Mutex::new(0.0));
        e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
        let at = |e: &mut Engine, frames: usize| {
            render(e, frames);
            *cutoff.lock().unwrap()
        };
        assert_eq!(at(&mut e, 64), 2000.0, "not playing: the song's value");
        e.play_song();
        assert!(at(&mut e, 64) < 21.0);
        // Two lines in, halfway, is the middle of the slider: 632 Hz.
        render(&mut e, 12 * TICK - 64);
        let mid = at(&mut e, 16);
        assert!((mid - 632.5).abs() < 0.5, "{mid}");
        assert!(at(&mut e, 12 * TICK + 64) > 19900.0);
        e.handle(Cmd::Stop);
        assert_eq!(at(&mut e, 64), 2000.0);
    }

    #[test]
    fn envelopes_move_the_mixer_and_are_published() {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        p.connect(id, OUTPUT_ID);
        p.patterns[0].tracks[0][0] = Cell { module: Some(id), ..note(60, None, None) };
        // The fader, the first parameter after the module's own, held at 0.
        let fader = ModuleKind::Generator.params().len();
        p.patterns[0].automation.push(crate::project::Envelope::new(id, fader, vec![(0.0, 0.0)]));
        let shared = Arc::new(Shared::default());
        let mut e = Engine::new(SR, Arc::new(p), None, None, shared.clone());
        e.play_song();
        let out = render(&mut e, 2000);
        assert!(loudness(&out) < 1e-6, "the fader is down");
        assert_eq!(*shared.automated.lock().unwrap(), [(id, fader, 0.0)]);
        e.handle(Cmd::Stop);
        render(&mut e, 64);
        assert!(shared.automated.lock().unwrap().is_empty(), "nothing while stopped");
    }

    /// A MultiSynth feeding two recorders, with `params` set.
    fn multisynth(params: &[(usize, f32)]) -> (Engine, u8, [Log; 2]) {
        let mut p = Project::empty();
        let ms = p.add_module(ModuleKind::MultiSynth, [0.0, 0.0]).unwrap();
        let a = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        let b = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        assert!(!p.connect(ms, OUTPUT_ID), "notes don't go to the output");
        assert!(p.connect(ms, a) && p.connect(ms, b));
        assert!(!p.connect(a, ms), "nor sound into a MultiSynth");
        for &(i, v) in params {
            p.module_mut(ms).unwrap().params[i] = v;
        }
        let mut e = engine(p);
        let logs = [Log::default(), Log::default()];
        e.node_mut(a).unwrap().dsp = Box::new(Recorder(logs[0].clone()));
        e.node_mut(b).unwrap().dsp = Box::new(Recorder(logs[1].clone()));
        (e, ms, logs)
    }

    /// A Glide in `mode` taking 0.1 s, feeding a recorder.
    fn glide(mode: f32) -> (Engine, u8, Log) {
        let mut p = Project::empty();
        let g = p.add_module(ModuleKind::Glide, [0.0, 0.0]).unwrap();
        let synth = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        assert!(p.connect(g, synth) && !p.connect(synth, g));
        p.module_mut(g).unwrap().params = vec![mode, 0.1];
        let mut e = engine(p);
        let log = Log::default();
        e.node_mut(synth).unwrap().dsp = Box::new(Recorder(log.clone()));
        (e, g, log)
    }

    #[test]
    fn glide_slides_each_note_from_the_last_in_its_time() {
        let (mut e, g, log) = glide(0.0);
        e.handle(Cmd::NoteOn { module: g, key: 1, note: 48, vel: 1.0 });
        render(&mut e, 1000);
        e.handle(Cmd::NoteOff { module: g, key: 1 });
        e.handle(Cmd::NoteOn { module: g, key: 1, note: 60, vel: 1.0 });
        render(&mut e, (SR * 0.2) as usize);
        let events = log.take();
        assert_eq!(values(&events, "on"), [48.0, 48.0], "the second note starts where the first was");
        let pitches = values(&events, "pitch");
        assert!(pitches.windows(2).all(|w| w[1] > w[0]) && pitches.last() == Some(&60.0), "{pitches:?}");
        // In steps of at most GLIDE_STEP frames over the 0.1 s.
        let steps = (SR * 0.1) as usize / GLIDE_STEP;
        assert!(pitches.len() >= steps && pitches.len() <= steps + 2, "{}", pitches.len());
    }

    #[test]
    fn legato_glides_only_over_a_held_note_without_starting_again() {
        let (mut e, g, log) = glide(1.0);
        e.handle(Cmd::NoteOn { module: g, key: 1, note: 48, vel: 1.0 });
        render(&mut e, 1000);
        // Let go and played again at once, as a track does: one note on.
        e.handle(Cmd::NoteOff { module: g, key: 1 });
        e.handle(Cmd::NoteOn { module: g, key: 1, note: 55, vel: 0.5 });
        render(&mut e, (SR * 0.2) as usize);
        let events = log.take();
        assert_eq!(values(&events, "on"), [48.0]);
        assert!(values(&events, "off").is_empty() && values(&events, "pitch").last() == Some(&55.0));
        // Let go for good, then a note after the gap jumps.
        e.handle(Cmd::NoteOff { module: g, key: 1 });
        render(&mut e, 1000);
        e.handle(Cmd::NoteOn { module: g, key: 1, note: 40, vel: 1.0 });
        render(&mut e, 1000);
        let events = log.take();
        assert_eq!((values(&events, "off").len(), values(&events, "on")), (1, vec![40.0]));
        assert!(values(&events, "pitch").is_empty());
    }

    #[test]
    fn multisynth_passes_notes_to_all_its_instruments() {
        let (mut e, ms, logs) = multisynth(&[(1, 12.0)]);
        e.handle(Cmd::NoteOn { module: ms, key: 5, note: 48, vel: 1.0 });
        e.handle(Cmd::NoteOff { module: ms, key: 5 });
        for log in &logs {
            assert_eq!(log.take(), [("on", 60.0), ("off", 0.0)]);
        }
    }

    #[test]
    fn multisynth_round_robin_and_range() {
        // Round robin, notes C-4 to C-5 only.
        let (mut e, ms, logs) = multisynth(&[(0, 1.0), (6, 48.0), (7, 60.0)]);
        for (key, note) in [(1, 48), (2, 50), (3, 72), (4, 52)] {
            e.handle(Cmd::NoteOn { module: ms, key, note, vel: 1.0 });
        }
        e.handle(Cmd::NoteOff { module: ms, key: 2 });
        assert_eq!(logs[0].take(), [("on", 48.0), ("on", 52.0)]);
        assert_eq!(logs[1].take(), [("on", 50.0), ("off", 0.0)], "the note off goes where its note went");
    }

    #[test]
    fn multisynth_in_the_pattern() {
        let (mut e, ms, logs) = multisynth(&[(1, -12.0)]);
        let mut p = (*e.project).clone();
        p.patterns[0].tracks[0][0] = Cell { module: Some(ms), ..note(60, None, Some((0x1, 0x10))) };
        e.handle(Cmd::Project(Arc::new(p)));
        e.play_song();
        render(&mut e, 3 * TICK);
        let events = logs[1].take();
        assert_eq!(events[0], ("on", 48.0));
        assert_eq!(values(&events, "pitch"), [49.0, 50.0], "slides keep the transpose");
    }

    #[test]
    fn note_columns_play_together() {
        let (mut e, log) = sequencer(&[(0, note(48, None, None))]);
        let mut p = (*e.project).clone();
        let id = p.patterns[0].tracks[0][0].module;
        p.set_columns(0, 2);
        *p.patterns[0].cell_mut(0, 1, 0) = Cell { module: id, ..note(52, None, None) };
        *p.patterns[0].cell_mut(0, 1, 1) = Cell { note: Some(Note::Off), ..Cell::default() };
        e.handle(Cmd::Project(Arc::new(p.clone())));
        e.play_song();
        render(&mut e, 12 * TICK);
        let events = log.take();
        assert_eq!(values(&events, "on"), [48.0, 52.0]);
        assert_eq!(values(&events, "off").len(), 1, "only the second column's note ends");

        // A hidden column doesn't play.
        p.set_columns(0, 1);
        e.handle(Cmd::Project(Arc::new(p)));
        e.play_song();
        render(&mut e, 2 * TICK);
        assert_eq!(values(&log.take(), "on"), [48.0]);
    }

    #[test]
    fn panning_and_delay_columns() {
        // Pan column 00 is hard left; delay 80 is half of a 6-tick line.
        let (mut e, log) = sequencer(&[
            (0, Cell { pan: Some(0x00), ..note(48, None, None) }),
            (1, Cell { delay: Some(0x80), ..note(50, None, None) }),
        ]);
        render(&mut e, TICK);
        let events = log.take();
        assert_eq!(events[..2], [("on", 48.0), ("pan", -1.0)]);
        assert!(values(&events, "pan").iter().all(|&p| p == -1.0));
        render(&mut e, 6 * TICK);
        let events = log.take();
        assert!(values(&events, "on").is_empty(), "not yet: {events:?}");
        render(&mut e, 3 * TICK);
        assert_eq!(values(&log.take(), "on"), [50.0], "three ticks in");
    }

    /// A Filter whose cutoff a Modulator moves, with `params` set on the
    /// Modulator, and a probe on the cutoff.
    fn modulated(params: &[(usize, f32)]) -> (Engine, Arc<Mutex<f32>>, u8, Project) {
        let mut p = Project::empty();
        let f = p.add_module(ModuleKind::Filter, [0.0, 0.0]).unwrap();
        let m = p.add_module(ModuleKind::Modulator, [0.0, 0.0]).unwrap();
        p.connect(f, OUTPUT_ID);
        assert!(p.connect(m, f), "a Modulator links to any module");
        assert!(!p.connect(f, m), "but not in a loop");
        p.set_control_param(m, f, 1);
        for &(i, v) in params {
            p.module_mut(m).unwrap().params[i] = v;
        }
        let e = engine(p.clone());
        let cutoff = Arc::new(Mutex::new(0.0));
        (e, cutoff, f, p)
    }

    #[test]
    fn modulators_move_parameters() {
        // A square LFO over 4 lines, at full amount: the cutoff jumps half
        // the slider up and down from 2 kHz.
        let (mut e, cutoff, f, _) = modulated(&[(1, 2.0), (3, 1.0), (4, 4.0), (5, 1.0)]);
        e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
        let spec = &ModuleKind::Filter.params()[1];
        let (up, down) = (spec.value_at(spec.position(2000.0) + 0.5), spec.value_at(spec.position(2000.0) - 0.5));
        render(&mut e, 64);
        assert_eq!(*cutoff.lock().unwrap(), up);
        render(&mut e, 12 * TICK);
        assert!((*cutoff.lock().unwrap() - down).abs() < 1e-3, "{} {down}", *cutoff.lock().unwrap());
        assert_eq!(e.shared.automated.lock().unwrap().len(), 1, "its value is published");
    }

    #[test]
    fn modulators_follow_their_input() {
        let (mut e, cutoff, f, mut p) = modulated(&[(0, 1.0), (5, 1.0)]);
        let synth = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        let m = p.modules.iter().find(|m| m.kind == ModuleKind::Modulator).unwrap().id;
        assert!(p.connect(synth, m));
        e.handle(Cmd::Project(Arc::new(p)));
        e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
        render(&mut e, 256);
        assert!((*cutoff.lock().unwrap() - 2000.0).abs() < 0.01, "silence leaves it alone");
        e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note: 60, vel: 1.0 });
        render(&mut e, 2000);
        assert!(*cutoff.lock().unwrap() > 2500.0, "{}", *cutoff.lock().unwrap());
    }

    #[test]
    fn modulators_track_the_key_and_velocity_played() {
        // Key mode, at full amount: C-8 is four octaves over C-4 and takes
        // the cutoff half the slider up, C-0 half down.
        let spec = &ModuleKind::Filter.params()[1];
        let at = |pos: f32| spec.value_at(spec.position(2000.0) + pos);
        for (mode, note, vel, expect) in [(2.0, 96, 1.0, at(1.0)), (2.0, 0, 1.0, at(-1.0)), (3.0, 60, 0.5, at(0.5))] {
            let (mut e, cutoff, f, mut p) = modulated(&[(0, mode), (5, 1.0), (6, 0.001), (7, 0.001)]);
            let synth = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
            let m = p.modules.iter().find(|m| m.kind == ModuleKind::Modulator).unwrap().id;
            assert!(p.connect(synth, m));
            e.handle(Cmd::Project(Arc::new(p)));
            e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
            render(&mut e, 256);
            assert!((*cutoff.lock().unwrap() - 2000.0).abs() < 0.01, "before a note it is left alone");
            e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note, vel });
            render(&mut e, 1000);
            let got = *cutoff.lock().unwrap();
            assert!((got - expect).abs() / expect < 0.01, "mode {mode}, note {note}: {got} for {expect}");
        }
    }

    #[test]
    fn modulators_run_an_envelope_per_note_or_follow_a_knob() {
        // Envelope: 0.1 s up, 0.1 s down, at full amount.
        let (mut e, cutoff, f, mut p) = modulated(&[(0, 4.0), (5, 1.0), (6, 0.1), (7, 0.1)]);
        let synth = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        let m = p.modules.iter().find(|m| m.kind == ModuleKind::Modulator).unwrap().id;
        assert!(p.connect(synth, m));
        e.handle(Cmd::Project(Arc::new(p)));
        e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
        let spec = &ModuleKind::Filter.params()[1];
        let top = spec.value_at(spec.position(2000.0) + 1.0);
        render(&mut e, 256);
        assert!((*cutoff.lock().unwrap() - 2000.0).abs() < 0.01, "before a note it is left alone");
        e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note: 60, vel: 1.0 });
        render(&mut e, (SR * 0.1) as usize);
        assert!((*cutoff.lock().unwrap() / top - 1.0).abs() < 0.05, "at the top after the attack");
        render(&mut e, (SR * 0.15) as usize);
        assert!((*cutoff.lock().unwrap() - 2000.0).abs() < 1.0, "and back down after the release");
        // Manual: the amount, as a knob.
        let (mut e, cutoff, f, _) = modulated(&[(0, 5.0), (5, -0.5), (6, 0.001)]);
        e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
        render(&mut e, 512);
        let want = spec.value_at(spec.position(2000.0) - 0.5);
        assert!((*cutoff.lock().unwrap() / want - 1.0).abs() < 0.01, "{} {want}", *cutoff.lock().unwrap());
    }

    #[test]
    fn drawn_lfo_shapes_move_parameters_on_the_beat() {
        // A drawn shape at the top for the first half of each cycle and at
        // the bottom for the second, a beat long, at full amount.
        let (mut e, cutoff, f, mut p) =
            modulated(&[(1, crate::project::DRAWN_SHAPE as f32), (3, 2.0), (5, 1.0), (8, 4.0)]);
        let m = p.modules.iter().find(|m| m.kind == ModuleKind::Modulator).unwrap().id;
        p.module_mut(m).unwrap().shape = vec![(0.0, 1.0), (0.49, 1.0), (0.51, 0.0), (1.0, 0.0)];
        e.handle(Cmd::Project(Arc::new(p.clone())));
        e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
        let spec = &ModuleKind::Filter.params()[1];
        let (up, down) = (spec.value_at(spec.position(2000.0) + 0.5), spec.value_at(spec.position(2000.0) - 0.5));
        render(&mut e, 64);
        assert!((*cutoff.lock().unwrap() / up - 1.0).abs() < 0.01, "up at the start of the cycle");
        // A beat is lpb lines; past half of it, the shape is down.
        let beat = p.lpb as usize * 6 * TICK;
        render(&mut e, beat * 3 / 4);
        assert!(
            (*cutoff.lock().unwrap() / down - 1.0).abs() < 0.01,
            "down in its second half: {}",
            *cutoff.lock().unwrap()
        );
    }

    #[test]
    fn synced_modulators_keep_to_the_song_wherever_it_starts() {
        // The drawn gate above, a beat long: up for the first half of each
        // beat, down for the second, at the same lines of the song whether
        // it plays from the top or from its second line, and however long
        // the program ran before.
        let line = 6 * TICK;
        for start in [0, 1] {
            let (mut e, cutoff, f, mut p) =
                modulated(&[(1, crate::project::DRAWN_SHAPE as f32), (3, 2.0), (5, 1.0), (8, 4.0)]);
            let m = p.modules.iter().find(|m| m.kind == ModuleKind::Modulator).unwrap().id;
            p.module_mut(m).unwrap().shape = vec![(0.0, 1.0), (0.49, 1.0), (0.51, 0.0), (1.0, 0.0)];
            e.handle(Cmd::Project(Arc::new(p.clone())));
            e.node_mut(f).unwrap().dsp = Box::new(Probe(cutoff.clone()));
            let spec = &ModuleKind::Filter.params()[1];
            let (up, down) = (spec.value_at(spec.position(2000.0) + 0.5), spec.value_at(spec.position(2000.0) - 0.5));
            render(&mut e, 5 * TICK + 70);
            e.handle(Cmd::Play { order: 0, line: start, loop_pattern: false });
            // To the middle of the song's second line, then of its fourth.
            render(&mut e, (1 - start) * line + line / 2);
            assert!((*cutoff.lock().unwrap() / up - 1.0).abs() < 0.01, "from line {start}: up in line 1");
            render(&mut e, 2 * line);
            assert!((*cutoff.lock().unwrap() / down - 1.0).abs() < 0.01, "from line {start}: down in line 3");
        }
    }

    #[test]
    fn block_loop_repeats_its_lines() {
        let (mut e, log) = sequencer(&[(1, note(48, None, None)), (3, note(50, None, None))]);
        e.handle(Cmd::BlockLoop(Some((0, 1, 2))));
        render(&mut e, 6 * 6 * TICK);
        let events = log.take();
        assert_eq!(values(&events, "on"), [48.0, 48.0, 48.0], "line 3 never comes");
        assert!((1..=2).contains(&e.pos_line), "{}", e.pos_line);
        e.handle(Cmd::BlockLoop(None));
        render(&mut e, 3 * 6 * TICK);
        assert_eq!(values(&log.take(), "on"), [50.0], "off again, it plays on");
    }

    #[test]
    fn track_scopes_show_each_tracks_notes() {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        p.connect(id, OUTPUT_ID);
        p.patterns[0].tracks[2][0] = Cell { module: Some(id), ..note(60, None, None) };
        let shared = Arc::new(Shared::default());
        let mut e = Engine::new(SR, Arc::new(p), None, None, shared.clone());
        e.play_song();
        render(&mut e, 1000);
        let scopes = shared.track_scopes.lock().unwrap();
        let track = |t: usize| loudness(&scopes[t * TRACK_SCOPE_LEN..(t + 1) * TRACK_SCOPE_LEN]);
        assert!(track(2) > 0.05, "{}", track(2));
        assert_eq!(track(0), 0.0);
        drop(scopes);
        // Live notes belong to no track.
        e.handle(Cmd::Stop);
        render(&mut e, 2000);
        e.handle(Cmd::NoteOn { module: id, key: LIVE_KEY, note: 60, vel: 1.0 });
        render(&mut e, 1000);
        let scopes = shared.track_scopes.lock().unwrap();
        assert!(scopes.iter().all(|x| x.abs() < 1e-3), "only the release of the old note");
    }

    #[test]
    fn groove_delays_odd_lines() {
        let (mut e, log) = sequencer(&[(1, note(48, None, None)), (2, note(50, None, None))]);
        let mut p = (*e.project).clone();
        p.groove = 1.0;
        e.handle(Cmd::Project(Arc::new(p)));
        e.play_song();
        let line = 6 * TICK;
        render(&mut e, line + line / 2 - 32);
        assert!(values(&log.take(), "on").is_empty(), "line 1 waits half a line");
        render(&mut e, 64);
        assert_eq!(values(&log.take(), "on"), [48.0]);
        render(&mut e, line / 2 - 64);
        assert!(values(&log.take(), "on").is_empty());
        render(&mut e, 64);
        assert_eq!(values(&log.take(), "on"), [50.0], "line 2 is on time");
    }

    #[test]
    fn bypassed_effects_let_sound_through() {
        let mut p = Project::empty();
        let synth = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        let amp = p.add_module(ModuleKind::Amplifier, [0.0, 0.0]).unwrap();
        p.module_mut(amp).unwrap().params[0] = 0.0;
        p.connect(synth, amp);
        p.connect(amp, OUTPUT_ID);
        let mut e = engine(p.clone());
        e.handle(Cmd::NoteOn { module: synth, key: LIVE_KEY, note: 60, vel: 1.0 });
        assert!(loudness(&render(&mut e, 500)) < 1e-6, "the amplifier is down");
        p.module_mut(amp).unwrap().bypass = true;
        e.handle(Cmd::Project(Arc::new(p)));
        assert!(loudness(&render(&mut e, 500)) > 0.1, "bypassed, it lets the synth through");
    }

    #[test]
    fn phrases_play_transposed_by_the_note() {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        p.connect(id, OUTPUT_ID);
        let mut phrase = Phrase { lines: 4, ..Phrase::default() };
        phrase.cells[0].note = Some(Note::On(48));
        phrase.cells[2] = Cell { note: Some(Note::On(52)), vol: Some(0x40), ..Cell::default() };
        phrase.cells[3].note = Some(Note::Off);
        let m = p.module_mut(id).unwrap();
        m.phrases.push(phrase);
        m.phrase_mode = PhraseMode::Program;
        let mut e = engine(p.clone());
        let log = Log::default();
        e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
        // C-5 moves the phrase up an octave.
        e.handle(Cmd::NoteOn { module: id, key: LIVE_KEY, note: 60, vel: 1.0 });
        assert_eq!(log.take(), [("on", 60.0)], "the first line at once");
        let published = |e: &mut Engine| {
            e.render(&mut []);
            e.shared.phrases.lock().unwrap().clone()
        };
        assert_eq!(published(&mut e), [(id, 0, 0)]);
        let line = 6 * TICK;
        render(&mut e, line);
        assert!(log.take().is_empty());
        render(&mut e, line + 64);
        assert_eq!(log.take(), [("off", 0.0), ("on", 64.0)]);
        assert_eq!(published(&mut e), [(id, 0, 2)], "on its third line");
        render(&mut e, line);
        assert_eq!(log.take(), [("off", 0.0)]);
        render(&mut e, 4 * line);
        assert!(log.take().is_empty(), "it doesn't loop");

        // Looping, it starts again; the key's note-off stops it.
        p.module_mut(id).unwrap().phrases[0].looping = true;
        e.handle(Cmd::Project(Arc::new(p)));
        e.handle(Cmd::NoteOn { module: id, key: LIVE_KEY, note: 48, vel: 1.0 });
        render(&mut e, 4 * line + 64);
        assert_eq!(values(&log.take(), "on"), [48.0, 52.0, 48.0]);
        e.handle(Cmd::NoteOff { module: id, key: LIVE_KEY });
        log.take();
        render(&mut e, 8 * line);
        assert!(values(&log.take(), "on").is_empty(), "stopped");
    }

    /// A synth with two phrases starting on C-4 and E-4, recorded.
    fn two_phrases(mode: PhraseMode) -> (Project, u8) {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        p.connect(id, OUTPUT_ID);
        let m = p.module_mut(id).unwrap();
        for (n, keys) in [(48, [0, 59]), (52, [60, 71])] {
            let mut phrase = Phrase { keys, ..Phrase::default() };
            phrase.cells[0].note = Some(Note::On(n));
            m.phrases.push(phrase);
        }
        m.phrase_mode = mode;
        m.selected_phrase = 1;
        (p, id)
    }

    #[test]
    fn notes_pick_phrases_by_mode_and_zxx() {
        let first_notes = |p: Project, id: u8, notes: &[u8]| {
            let mut e = engine(p);
            let log = Log::default();
            e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
            for (k, &n) in notes.iter().enumerate() {
                e.handle(Cmd::NoteOn { module: id, key: k as u32, note: n, vel: 1.0 });
            }
            values(&log.take(), "on")
        };
        // The key map: C-4 plays the first phrase, C-5 the second, moved
        // up an octave, and C-6, outside both, the synth itself.
        let (p, id) = two_phrases(PhraseMode::Keymap);
        assert_eq!(first_notes(p, id, &[48, 60, 72]), [48.0, 64.0, 72.0]);
        let (p, id) = two_phrases(PhraseMode::Program);
        assert_eq!(first_notes(p, id, &[48, 72]), [52.0, 76.0], "the selected phrase");
        let (p, id) = two_phrases(PhraseMode::Off);
        assert_eq!(first_notes(p, id, &[48]), [48.0]);

        // Zxx picks a phrase whatever the mode, Z00 none.
        let (mut p, id) = two_phrases(PhraseMode::Off);
        for (line, z) in [(0, 1), (4, 2), (8, 0)] {
            p.patterns[0].tracks[0][line] =
                Cell { note: Some(Note::On(48)), module: Some(id), fx: Some((FX_PHRASE, z)), ..Cell::default() };
        }
        p.module_mut(id).unwrap().phrase_mode = PhraseMode::Program;
        let mut e = engine(p);
        let log = Log::default();
        e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
        e.play_song();
        render(&mut e, 9 * 6 * TICK + 10);
        assert_eq!(values(&log.take(), "on"), [48.0, 52.0, 48.0]);
    }

    #[test]
    fn phrases_play_effect_commands() {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        p.connect(id, OUTPUT_ID);
        let mut phrase = Phrase { lines: 4, ..Phrase::default() };
        // A slide up a semitone a tick, a cut two ticks into the next
        // line, and a note delayed by three ticks after that.
        phrase.cells[0] = Cell { note: Some(Note::On(48)), fx: Some((0x1, 0x10)), ..Cell::default() };
        phrase.cells[1].fx = Some((0xC, 2));
        phrase.cells[2] = Cell { note: Some(Note::On(55)), fx: Some((0xD, 3)), ..Cell::default() };
        let m = p.module_mut(id).unwrap();
        m.phrases.push(phrase);
        m.phrase_mode = PhraseMode::Program;
        let mut e = engine(p);
        let log = Log::default();
        e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
        // Transposed by the note played, C-5.
        e.handle(Cmd::NoteOn { module: id, key: LIVE_KEY, note: 60, vel: 1.0 });
        render(&mut e, 6 * TICK + 2 * TICK + 10);
        let events = log.take();
        assert_eq!(values(&events, "pitch"), [61.0, 62.0, 63.0, 64.0, 65.0]);
        assert_eq!(events.last(), Some(&("off", 0.0)), "cut");
        render(&mut e, 4 * TICK + 2 * TICK);
        assert!(log.take().is_empty(), "delayed");
        render(&mut e, TICK);
        assert_eq!(values(&log.take(), "on"), [67.0]);
    }

    #[test]
    fn yxx_plays_notes_by_chance() {
        let played = |chance: u8| {
            let cells: Vec<(usize, Cell)> = (0..64).map(|l| (l, note(48, None, Some((FX_MAYBE, chance))))).collect();
            let (mut e, log) = sequencer(&cells);
            render(&mut e, 64 * 6 * TICK);
            values(&log.take(), "on").len()
        };
        assert_eq!(played(0xFF), 64);
        assert_eq!(played(0x00), 0);
        let half = played(0x80);
        assert!((16..=48).contains(&half), "about half: {half}");
    }

    #[test]
    fn auto_pan_swings_the_note_for_its_line() {
        let (mut e, log) = sequencer(&[(0, note(48, None, Some((FX_AUTOPAN, 0x8F)))), (1, Cell::default())]);
        render(&mut e, 6 * TICK + TICK / 2);
        let pans = values(&log.take(), "pan");
        // A quarter cycle a tick at full depth: right, centre, left, ...
        assert_eq!(pans.len(), 6, "ticks 1 to 5, then the next line: {pans:?}");
        assert!((pans[0] - 1.0).abs() < 1e-4 && (pans[2] + 1.0).abs() < 1e-4, "{pans:?}");
        assert_eq!(pans.last(), Some(&0.0), "back in the middle on the next line");
    }

    #[test]
    fn autoseek_plays_a_sample_started_before_playback_from_where_it_would_be() {
        let level = |autoseek: bool, line: usize| {
            let mut p = Project::empty();
            p.modules[0].params[0] = 1.0;
            let id = p.add_module(ModuleKind::Sampler, [0.0, 0.0]).unwrap();
            p.connect(id, OUTPUT_ID);
            // A rising ramp four seconds long, so its level tells the position.
            let frames = (0..SR as usize * 4).map(|i| [i as f32 / (SR * 4.0); 2]).collect();
            let sample = Sample { name: "ramp".into(), sample_rate: SR, channels: 1, frames };
            let mut slot = crate::project::SampleSlot::new(sample, None);
            (slot.base_note, slot.autoseek) = (48, autoseek);
            p.module_mut(id).unwrap().samples.push(slot);
            *p.patterns[0].cell_mut(0, 0, 0) = Cell { note: Some(Note::On(48)), module: Some(id), ..Cell::default() };
            let mut e = engine(p);
            e.handle(Cmd::Play { order: 0, line, loop_pattern: false });
            let out = render(&mut e, 600);
            out[500..].iter().sum::<f32>() / 100.0
        };
        assert!(level(false, 8).abs() < 1e-4, "without autoseek it waits for its next note");
        let (early, late) = (level(true, 8), level(true, 16));
        assert!(early > 0.0, "it plays at once");
        // Twice as far in, nearly twice as high up the ramp.
        assert!((1.8..2.1).contains(&(late / early)), "{early} {late}");
        assert_eq!(level(true, 0), level(false, 0), "from the note itself, nothing changes");
    }

    #[test]
    fn effect_columns_act_on_every_note_column() {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Generator, [0.0, 0.0]).unwrap();
        p.connect(id, OUTPUT_ID);
        let pat = &mut p.patterns[0];
        pat.set_columns(0, 2);
        pat.set_fx_columns(0, 2);
        *pat.cell_mut(0, 0, 0) = Cell { note: Some(Note::On(48)), module: Some(id), ..Cell::default() };
        *pat.cell_mut(0, 1, 0) = Cell { note: Some(Note::On(55)), module: Some(id), ..Cell::default() };
        // The track's first effect column slides both up, its second sets the tempo.
        pat.cell_mut(0, 2, 0).fx = Some((0x1, 0x10));
        pat.cell_mut(0, 3, 0).fx = Some((0xF, 0x60));
        let mut e = engine(p);
        let log = Log::default();
        e.node_mut(id).unwrap().dsp = Box::new(Recorder(log.clone()));
        e.play_song();
        render(&mut e, 2 * TICK + 10);
        let pitches = values(&log.take(), "pitch");
        assert!(pitches.contains(&49.0) && pitches.contains(&56.0), "{pitches:?}");
        assert_eq!(e.bpm, 96.0);
    }
}
