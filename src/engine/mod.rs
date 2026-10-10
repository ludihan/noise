//! The audio engine: runs the sequencer and the module graph.
//!
//! The engine lives on the audio thread. The UI talks to it through a
//! command channel and reads playback state back from `Shared`.
//!
//! The `Engine` and its commands are here; how it routes notes, plays the
//! song and renders a block are in `notes`, `sequencer` and `render`,
//! with the module graph's nodes in `node` and note columns in `track`.

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

mod node;
mod notes;
mod render;
mod sequencer;
mod shared;
mod track;

use node::*;
pub use shared::*;
use track::*;

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

impl NoteEv {
    /// The event `transpose` semitones up and its velocity scaled by `vel`,
    /// as a MultiSynth or a phrase plays it.
    fn moved(self, transpose: f32, vel: f32) -> Self {
        match self {
            NoteEv::On(note, v) => NoteEv::On(note + transpose, (v * vel).min(1.0)),
            NoteEv::Pitch(note) => NoteEv::Pitch(note + transpose),
            NoteEv::Vel(v) => NoteEv::Vel((v * vel).min(1.0)),
            ev => ev,
        }
    }

    /// Gives the event to `dsp` for the notes on `key`.
    fn apply(self, dsp: &mut dyn Dsp, key: u32) {
        match self {
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
    }
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
        ev.moved(self.transpose, self.vel).apply(dsp, self.key);
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
    rng: crate::rng::Rng,
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
            rng: crate::rng::Rng(0x2545_F491),
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
                    n.dry.clear();
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
                    dry: Vec::new(),
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
            // Which keep their sound before the mixer for it.
            for k in 0..self.nodes[j].key.len() {
                let s = self.nodes[j].key[k];
                self.nodes[s].dry.resize(BLOCK, [0.0; 2]);
            }
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
}

#[cfg(test)]
mod tests;
