//! Audio processing for every module kind.

use crate::engine::Tape;
use crate::project::{
    EQ10_FREQS, MAX_COLUMNS, MAX_TRACKS, Modulation, ModuleKind, SampleSlot, VoiceEnvelope, VoiceLfo,
};
use crate::sample::Sample;

pub mod fft;
use std::f32::consts::{PI, TAU};
use std::sync::Arc;

pub type Frame = [f32; 2];

/// The most frames the engine asks a module for at once.
pub const MAX_BLOCK: usize = 64;

/// What an instrument's notes from each track add up to in a block, before
/// its effects, for the track scopes. Notes are told apart by
/// key, and the sequencer's keys are its channels.
pub struct TrackTap {
    pub sums: Vec<[f32; MAX_BLOCK]>,
    /// Bit `t` is set when track `t` has anything in `sums`.
    pub used: u32,
}

impl TrackTap {
    fn new() -> Self {
        TrackTap { sums: vec![[0.0; MAX_BLOCK]; MAX_TRACKS], used: 0 }
    }

    /// Adds `x` (both channels) at frame `i` for the track playing `key`;
    /// live notes from the keyboard belong to no track.
    fn add(&mut self, key: u32, i: usize, x: Frame) {
        let key = key as usize;
        if key < MAX_TRACKS * MAX_COLUMNS && i < MAX_BLOCK {
            let t = key % MAX_TRACKS;
            self.sums[t][i] += (x[0] + x[1]) * 0.5;
            self.used |= 1 << t;
        }
    }

    /// Empties the tracks used.
    pub fn clear(&mut self) {
        let mut used = self.used;
        while used != 0 {
            let t = used.trailing_zeros() as usize;
            self.sums[t].fill(0.0);
            used &= used - 1;
        }
        self.used = 0;
    }
}

/// Where a voice is in a sample, for drawing playheads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Playhead {
    pub module: u8,
    /// Index of the sample slot.
    pub slot: usize,
    /// Position in frames.
    pub pos: f64,
    /// Current loudness of the voice, 0..1.
    pub level: f32,
    /// Where the voice is in the pitch and filter envelopes, in seconds.
    pub envelopes: [f32; 2],
}

/// Has the CPU treat numbers too small to matter (denormals) as zero on
/// this thread. Filters and reverbs fading out pass through them, and on
/// most CPUs each sum with one is many times slower, enough to make a busy
/// song stutter.
/// Points in a cycle of `sine`'s table.
const SINE_LEN: usize = 4096;

/// A cycle of a sine, and its first point again at the end, worked out
/// while compiling.
static SINE: [f32; SINE_LEN + 1] = sine_table();

const fn sine_table() -> [f32; SINE_LEN + 1] {
    let mut table = [0.0; SINE_LEN + 1];
    let mut i = 0;
    while i <= SINE_LEN {
        // Taylor's series around 0, after folding the angle into -pi..pi.
        let mut x = i as f64 / SINE_LEN as f64 * std::f64::consts::TAU;
        if x > std::f64::consts::PI {
            x -= std::f64::consts::TAU;
        }
        let (mut term, mut sum, mut k) = (x, x, 1);
        while k < 30 {
            term *= -x * x / ((2 * k) * (2 * k + 1)) as f64;
            sum += term;
            k += 1;
        }
        table[i] = sum as f32;
        i += 1;
    }
    table
}

/// The sine of `cycles` whole turns, `sin(TAU * cycles)`, from a table:
/// within 2e-6 of it and a little faster, for oscillators and LFOs that
/// run every sample.
#[inline]
fn sine(cycles: f32) -> f32 {
    // The fraction of a turn: a cast is an instruction where floor() is
    // a call on most CPUs.
    let turn = cycles - (cycles as i64) as f32;
    let at = (if turn < 0.0 { turn + 1.0 } else { turn }) * SINE_LEN as f32;
    let i = (at as usize).min(SINE_LEN - 1);
    let (a, b) = (SINE[i], SINE[i + 1]);
    a + (b - a) * (at - i as f32)
}

/// The fraction of `x` past its whole part, as `f32::fract` (negative
/// below zero): a cast is an instruction where `fract` is a call on most
/// CPUs, and oscillators and delay taps want it every sample.
#[inline]
fn fract(x: f32) -> f32 {
    x - (x as i64) as f32
}

pub fn flush_denormals() {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: sets the flush-to-zero (bit 15) and denormals-are-zero
    // (bit 6) bits of this thread's SSE control register.
    unsafe {
        let mut csr = 0u32;
        std::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack, preserves_flags));
        csr |= 0x8040;
        std::arch::asm!("ldmxcsr [{}]", in(reg) &csr, options(nostack, preserves_flags, readonly));
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: sets the flush-to-zero bit (24) of this thread's
    // floating-point control register.
    unsafe {
        let fpcr: u64;
        std::arch::asm!("mrs {}, fpcr", out(reg) fpcr, options(nomem, nostack, preserves_flags));
        std::arch::asm!("msr fpcr, {}", in(reg) fpcr | 1 << 24, options(nomem, nostack, preserves_flags));
    }
}

pub struct Ctx {
    pub sr: f32,
    pub samples_per_line: f32,
    /// While the song plays, where it is at the start of the block, in
    /// lines from its start: what LFOs synced to lines follow, so they keep
    /// to the beat however playback started.
    pub song_line: Option<f64>,
}

/// Note events are addressed by `key` so a module can tell voices apart:
/// the sequencer uses the track number, live keyboard input uses the note.
pub trait Dsp: Send {
    fn note_on(&mut self, _key: u32, _note: f32, _vel: f32) {}
    fn note_off(&mut self, _key: u32) {}
    fn set_pitch(&mut self, _key: u32, _note: f32) {}
    fn set_velocity(&mut self, _key: u32, _vel: f32) {}
    /// Pan the notes on `key`, -1..1, on top of the module's own panning.
    fn set_pan(&mut self, _key: u32, _pan: f32) {}
    /// Start the most recent note on `key` at `pos` (0..1) of its sample.
    fn sample_offset(&mut self, _key: u32, _pos: f32) {}
    /// Play the most recent note on `key` backwards (from the end, if it
    /// has only just started), or forwards again.
    fn reverse(&mut self, _key: u32, _on: bool) {}
    /// Play slice `slice` of the sample the note just played on `key`
    /// instead, keeping its pitch.
    fn play_slice(&mut self, _key: u32, _slice: usize) {}
    /// The most recent note on `key` was started `frames` of output ago,
    /// as playback started partway through the song: a sample with
    /// autoseek plays on from where it would be; others stay silent.
    fn seek(&mut self, _key: u32, _frames: f64) {}
    /// The module's samples changed (or the project was edited).
    fn set_samples(&mut self, _slots: &[SampleSlot]) {}
    /// The module's voice modulation, on every edit.
    fn set_modulation(&mut self, _m: &Modulation) {}
    /// What the last `process` played for each track, for instruments.
    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        None
    }
    /// Release everything that is sounding.
    fn release_all(&mut self) {}
    /// Silence immediately and clear internal state (tails, buffers).
    fn reset(&mut self) {}
    /// Adds the sample positions of sounding voices to `out`, with
    /// `module` left for the caller to fill in.
    fn playheads(&self, _out: &mut Vec<Playhead>) {}
    fn process(&mut self, ctx: &Ctx, params: &[f32], input: &[Frame], out: &mut [Frame]);
}

pub fn create(kind: ModuleKind, sr: f32) -> Box<dyn Dsp> {
    match kind {
        ModuleKind::Output => Box::new(Output),
        ModuleKind::Generator => Box::new(Generator::new()),
        ModuleKind::Wavetable => Box::new(wavetable::Wavetable::new()),
        ModuleKind::Fm => Box::new(Fm::new()),
        ModuleKind::Drums => Box::new(Drums::new()),
        ModuleKind::Sampler => Box::new(Sampler::new()),
        ModuleKind::Granular => Box::new(granular::Granular::new()),
        ModuleKind::Filter => Box::new(Filter::default()),
        ModuleKind::Distortion => Box::new(Distortion::default()),
        ModuleKind::Delay => Box::new(Delay::new(sr)),
        ModuleKind::Reverb => Box::new(Reverb::new(sr)),
        ModuleKind::Amplifier => Box::new(Amplifier),
        ModuleKind::Lfo => Box::new(Lfo::default()),
        ModuleKind::Flanger => Box::new(Flanger::new(sr)),
        ModuleKind::Phaser => Box::new(Phaser::default()),
        ModuleKind::VocalFilter => Box::new(VocalFilter::default()),
        ModuleKind::Repeater => Box::new(Repeater::new(sr)),
        ModuleKind::RingMod => Box::new(RingMod::default()),
        ModuleKind::Gate => Box::new(Gate::default()),
        ModuleKind::Kicker => Box::new(Kicker::new()),
        ModuleKind::SpectraVoice => Box::new(SpectraVoice::new()),
        ModuleKind::PitchShifter => Box::new(PitchShifter::new(sr)),
        ModuleKind::StereoExpander => Box::new(StereoExpander::default()),
        ModuleKind::CombFilter => Box::new(CombFilter::new(sr)),
        ModuleKind::Maximizer => Box::new(Maximizer::new(sr)),
        ModuleKind::Exciter => Box::new(Exciter::default()),
        ModuleKind::DcBlocker => Box::new(DcBlocker::default()),
        ModuleKind::ScreamFilter => Box::new(ScreamFilter::default()),
        ModuleKind::Multitap => Box::new(Multitap::new(sr)),
        ModuleKind::Eq10 => Box::new(Eq10::default()),
        ModuleKind::Cabinet => Box::new(Cabinet::default()),
        ModuleKind::Vibrato => Box::new(Vibrato::new(sr)),
        ModuleKind::Fmx => Box::new(Fmx::new()),
        ModuleKind::Input => Box::new(LiveInput::new(Arc::default())),
        ModuleKind::WaveShaper => Box::new(WaveShaper),
        ModuleKind::FilterPro => Box::new(FilterPro::default()),
        ModuleKind::Chorus => Box::new(Chorus::new(sr)),
        ModuleKind::Echo => Box::new(Echo::new(sr)),
        ModuleKind::AnalogFilter => Box::new(AnalogFilter::default()),
        ModuleKind::PlateReverb => Box::new(PlateReverb::new(sr)),
        ModuleKind::Eq5 => Box::new(Eq5::default()),
        ModuleKind::Compressor => Box::new(Compressor::default()),
        ModuleKind::Eq => Box::new(Eq::default()),
        ModuleKind::MultiSynth | ModuleKind::Glide | ModuleKind::Modulator => Box::new(Silent),
    }
}

fn note_to_freq(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// Equal-power gains for a mono source at `pan`, clamped to -1..1, with
/// both at 1 in the middle.
fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
    (a.cos() * std::f32::consts::SQRT_2, a.sin() * std::f32::consts::SQRT_2)
}

#[derive(Clone, Copy)]
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

// ---------------------------------------------------------------- envelopes

#[derive(Clone, Copy, Default, PartialEq)]
enum Stage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

#[derive(Clone, Copy, Default)]
struct Adsr {
    stage: Stage,
    level: f32,
    /// The time and rate in samples the last decay or release step fell
    /// by, kept so `exp` runs when the time changes, not every sample.
    fall: (f32, f32, f32),
}

impl Adsr {
    fn trigger(&mut self) {
        self.stage = Stage::Attack;
    }

    fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    fn active(&self) -> bool {
        self.stage != Stage::Idle
    }

    /// What the level is multiplied by each sample to fall over `t`
    /// seconds: by five time constants.
    fn fall(&mut self, t: f32, sr: f32) -> f32 {
        if (self.fall.0, self.fall.1) != (t, sr) {
            self.fall = (t, sr, (-5.0 / (t.max(0.0005) * sr)).exp());
        }
        self.fall.2
    }

    /// `a`, `d`, `r` in seconds.
    fn next(&mut self, sr: f32, a: f32, d: f32, s: f32, r: f32) -> f32 {
        let rate = |t: f32| 1.0 / (t.max(0.0005) * sr);
        match self.stage {
            Stage::Idle => self.level = 0.0,
            Stage::Attack => {
                self.level += rate(a);
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                // Exponential approach to the sustain level.
                self.level = s + (self.level - s) * self.fall(d, sr);
                if (self.level - s).abs() < 0.0005 {
                    self.level = s;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {
                self.level = s;
                if s <= 0.0 {
                    self.stage = Stage::Idle;
                }
            }
            Stage::Release => {
                self.level *= self.fall(r, sr);
                if self.level < 0.0001 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

// ---------------------------------------------------------------- voices

const MAX_VOICES: usize = 16;

/// Bookkeeping shared by the polyphonic instruments.
#[derive(Clone, Copy, Default)]
struct VoiceSlot {
    key: u32,
    note: f32,
    vel: f32,
    pan: f32,
    age: u64,
    released: bool,
}

fn alloc_voice<V>(voices: &mut [V], slot: impl Fn(&V) -> (&VoiceSlot, bool)) -> usize {
    // Prefer an idle voice, otherwise steal the oldest one.
    let mut best = 0;
    let mut best_age = u64::MAX;
    for (i, v) in voices.iter().enumerate() {
        let (s, active) = slot(v);
        if !active {
            return i;
        }
        if s.age < best_age {
            best_age = s.age;
            best = i;
        }
    }
    best
}

/// The `Dsp` note methods the enveloped synths share, for a `voices`
/// array whose voices have a `slot` and an `env`: changes reach the notes
/// of a key not yet let go. `note_off` adds letting go of them.
macro_rules! voice_controls {
    (note_off) => {
        voice_controls!();

        fn note_off(&mut self, key: u32) {
            for v in self.voices.iter_mut().filter(|v| v.slot.key == key && !v.slot.released) {
                v.slot.released = true;
                v.env.release();
            }
        }
    };
    () => {
        fn set_pitch(&mut self, key: u32, note: f32) {
            for v in self.voices.iter_mut().filter(|v| v.slot.key == key && !v.slot.released) {
                v.slot.note = note;
            }
        }

        fn set_velocity(&mut self, key: u32, vel: f32) {
            for v in self.voices.iter_mut().filter(|v| v.slot.key == key && !v.slot.released) {
                v.slot.vel = vel;
            }
        }

        fn set_pan(&mut self, key: u32, pan: f32) {
            for v in self.voices.iter_mut().filter(|v| v.slot.key == key && !v.slot.released) {
                v.slot.pan = pan;
            }
        }

        fn release_all(&mut self) {
            for v in &mut self.voices {
                v.slot.released = true;
                v.env.release();
            }
        }

        fn reset(&mut self) {
            for v in &mut self.voices {
                v.env = Default::default();
            }
        }
    };
}

// The modules in files of their own come after the macro they use.
pub mod granular;
pub mod wavetable;

// ---------------------------------------------------------------- output / amp

struct Output;

impl Dsp for Output {
    fn process(&mut self, _: &Ctx, params: &[f32], input: &[Frame], out: &mut [Frame]) {
        let vol = params[0];
        for (o, i) in out.iter_mut().zip(input) {
            *o = [i[0] * vol, i[1] * vol];
        }
    }
}

/// Modules that make no sound of their own; the engine routes their notes.
struct Silent;

impl Dsp for Silent {
    fn process(&mut self, _: &Ctx, _: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
    }
}

struct Amplifier;

impl Dsp for Amplifier {
    fn process(&mut self, _: &Ctx, params: &[f32], input: &[Frame], out: &mut [Frame]) {
        let sign = if params[2] >= 0.5 { -1.0 } else { 1.0 };
        let (l, r) = pan_gains(params[1]);
        let vol = params[0] * sign;
        for (o, i) in out.iter_mut().zip(input) {
            *o = [i[0] * vol * l, i[1] * vol * r];
        }
    }
}

// ---------------------------------------------------------------- generator

fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Default)]
struct GenVoice {
    slot: VoiceSlot,
    env: Adsr,
    phase: [f32; 4],
    tri: [f32; 4],
    md: VoiceMod,
}

struct Generator {
    voices: [GenVoice; MAX_VOICES],
    clock: u64,
    rng: Rng,
    mods: Modulation,
    tap: TrackTap,
}

impl Generator {
    fn new() -> Self {
        let mut voices = [GenVoice::default(); MAX_VOICES];
        // Spread initial phases so unison voices don't start in lockstep.
        for v in &mut voices {
            v.phase = [0.0, 0.31, 0.67, 0.13];
        }
        Self { voices, clock: 0, rng: Rng(0x1234_5678), mods: Modulation::default(), tap: TrackTap::new() }
    }
}

impl Dsp for Generator {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        let v = &mut self.voices[i];
        v.slot = VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false };
        v.md = VoiceMod::default();
        v.env.trigger();
    }

    fn set_modulation(&mut self, m: &Modulation) {
        copy_modulation(&mut self.mods, m);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, wave) = (p[0], p[1].round() as u32);
        let (a, d, s, r) = (p[2], p[3], p[4], p[5]);
        let detune = p[6];
        let unison = (p[7].round() as usize).clamp(1, 4);
        let pw = p[8];
        let norm = 1.0 / (unison as f32).sqrt();

        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let (pl, pr) = pan_gains(p[9] + v.slot.pan);
            let mut mb = v.md.block(&self.mods, !v.slot.released, out.len(), ctx.sr);
            let mut dts = [0.0; 4];
            for (u, dt) in dts.iter_mut().enumerate().take(unison) {
                // Unison voices fan out symmetrically around the note.
                let spread = if unison > 1 { (u as f32 / (unison - 1) as f32 - 0.5) * 2.0 } else { 0.0 };
                let cents = detune * spread;
                *dt = (note_to_freq(v.slot.note + mb.bend + cents / 100.0) / ctx.sr).min(0.49);
            }
            for (i, o) in out.iter_mut().enumerate() {
                let env = v.env.next(ctx.sr, a, d, s, r);
                let mut sl = 0.0;
                let mut sr_ = 0.0;
                for (u, &dt) in dts.iter().enumerate().take(unison) {
                    let t = v.phase[u];
                    let x = match wave {
                        0 => 2.0 * t - 1.0 - poly_blep(t, dt),
                        1 => {
                            let mut y = if t < pw { 1.0 } else { -1.0 };
                            y += poly_blep(t, dt);
                            y -= poly_blep(fract(t - pw + 1.0), dt);
                            y
                        }
                        2 => {
                            // Integrated band-limited square.
                            let mut sq = if t < 0.5 { 1.0 } else { -1.0 };
                            sq += poly_blep(t, dt);
                            sq -= poly_blep(fract(t + 0.5), dt);
                            v.tri[u] = dt * 4.0 * sq + (1.0 - dt * 0.5) * v.tri[u];
                            v.tri[u]
                        }
                        3 => sine(t),
                        _ => self.rng.next(),
                    };
                    v.phase[u] = fract(t + dt);
                    // Alternate unison voices left/right for width.
                    if unison > 1 && u % 2 == 1 {
                        sr_ += x * 1.3;
                        sl += x * 0.7;
                    } else if unison > 1 {
                        sl += x * 1.3;
                        sr_ += x * 0.7;
                    } else {
                        sl += x;
                        sr_ += x;
                    }
                }
                let [sl, sr_] = v.md.apply(&mut mb, [sl, sr_]);
                let g = env * v.slot.vel * vol * norm;
                let y = [sl * g * pl, sr_ * g * pr];
                o[0] += y[0];
                o[1] += y[1];
                self.tap.add(v.slot.key, i, y);
            }
        }
    }
}

// ---------------------------------------------------------------- FM

#[derive(Clone, Copy, Default)]
struct FmVoice {
    slot: VoiceSlot,
    env: Adsr,
    car: f32,
    modu: f32,
    mod_env: f32,
    last: f32,
    md: VoiceMod,
}

struct Fm {
    voices: [FmVoice; MAX_VOICES],
    clock: u64,
    mods: Modulation,
    tap: TrackTap,
}

impl Fm {
    fn new() -> Self {
        Self { voices: [FmVoice::default(); MAX_VOICES], clock: 0, mods: Modulation::default(), tap: TrackTap::new() }
    }
}

impl Dsp for Fm {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        let v = &mut self.voices[i];
        v.slot = VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false };
        v.env.trigger();
        v.mod_env = 1.0;
        v.md = VoiceMod::default();
    }

    fn set_modulation(&mut self, m: &Modulation) {
        copy_modulation(&mut self.mods, m);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, ratio, index, mod_decay, fb) = (p[0], p[1], p[2], p[3], p[4]);
        let (a, d, s, r) = (p[5], p[6], p[7], p[8]);
        let mod_mul = (-1.0 / (mod_decay.max(0.001) * ctx.sr)).exp();
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let (pl, pr) = pan_gains(v.slot.pan);
            let mut mb = v.md.block(&self.mods, !v.slot.released, out.len(), ctx.sr);
            let f = note_to_freq(v.slot.note + mb.bend);
            let dc = f / ctx.sr;
            let dm = f * ratio / ctx.sr;
            for (i, o) in out.iter_mut().enumerate() {
                let env = v.env.next(ctx.sr, a, d, s, r);
                let m = sine(v.modu + v.last * fb * 0.5);
                v.last = m;
                let idx = index * (0.15 + 0.85 * v.mod_env);
                let x = sine(v.car + m * idx / TAU) * env * v.slot.vel * vol;
                v.car = fract(v.car + dc);
                v.modu = fract(v.modu + dm);
                v.mod_env *= mod_mul;
                let [l, r] = v.md.apply(&mut mb, [x, x]);
                o[0] += l * pl;
                o[1] += r * pr;
                self.tap.add(v.slot.key, i, [l * pl, r * pr]);
            }
        }
    }
}

// ---------------------------------------------------------------- drums

#[derive(Clone, Copy, Default, PartialEq)]
enum DrumKind {
    #[default]
    Kick,
    Snare,
    ClosedHat,
    OpenHat,
    Tom,
}

#[derive(Clone, Copy, Default)]
struct DrumVoice {
    slot: VoiceSlot,
    kind: DrumKind,
    active: bool,
    t: f32,
    phase: f32,
    hp: [f32; 2],
}

struct Drums {
    voices: [DrumVoice; MAX_VOICES],
    clock: u64,
    rng: Rng,
    tap: TrackTap,
}

impl Drums {
    fn new() -> Self {
        Self { voices: [DrumVoice::default(); MAX_VOICES], clock: 0, rng: Rng(0x9e37_79b9), tap: TrackTap::new() }
    }
}

impl Dsp for Drums {
    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        // Pitch class picks the drum: C kick, D snare, F# closed hat,
        // A# open hat, anything else a tom tuned to the note.
        let kind = match (note.round() as i32).rem_euclid(12) {
            0 | 1 => DrumKind::Kick,
            2..=4 => DrumKind::Snare,
            6 | 8 => DrumKind::ClosedHat,
            10 | 11 => DrumKind::OpenHat,
            _ => DrumKind::Tom,
        };
        if kind == DrumKind::ClosedHat {
            // Closed hat chokes the open one.
            for v in self.voices.iter_mut().filter(|v| v.kind == DrumKind::OpenHat) {
                v.active = false;
            }
        }
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.active));
        self.voices[i] = DrumVoice {
            slot: VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false },
            kind,
            active: true,
            ..Default::default()
        };
    }

    fn set_pan(&mut self, key: u32, pan: f32) {
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.active) {
            v.slot.pan = pan;
        }
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.active = false;
        }
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, kick_tone, kick_decay, snare_tone, hat_decay) = (p[0], p[1], p[2], p[3], p[4]);
        let dt = 1.0 / ctx.sr;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let (pl, pr) = pan_gains(v.slot.pan);
            for (i, o) in out.iter_mut().enumerate() {
                let t = v.t;
                let (x, done) = match v.kind {
                    DrumKind::Kick => {
                        let f = kick_tone + 350.0 * (-t * 40.0).exp();
                        v.phase = fract(v.phase + f * dt);
                        let env = (-t / kick_decay * 4.0).exp();
                        let click = if t < 0.002 { self.rng.next() * 0.3 } else { 0.0 };
                        ((v.phase * TAU).sin() * env + click, env < 0.001)
                    }
                    DrumKind::Snare => {
                        let f = 160.0 + 60.0 * snare_tone + 100.0 * (-t * 60.0).exp();
                        v.phase = fract(v.phase + f * dt);
                        let body = (v.phase * TAU).sin() * (-t * 25.0).exp() * snare_tone;
                        let n = self.rng.next();
                        // One-pole highpass on the noise.
                        let hp = n - v.hp[0];
                        v.hp[0] += 0.3 * hp;
                        let noise = hp * (-t * 14.0).exp() * (1.2 - snare_tone * 0.6);
                        let x = body + noise;
                        (x, t > 0.6)
                    }
                    DrumKind::ClosedHat | DrumKind::OpenHat => {
                        let decay = if v.kind == DrumKind::OpenHat { hat_decay * 6.0 } else { hat_decay };
                        let n = self.rng.next();
                        // Two cascaded highpasses for a thin metallic hiss.
                        let h1 = n - v.hp[0];
                        v.hp[0] += 0.6 * h1;
                        let h2 = h1 - v.hp[1];
                        v.hp[1] += 0.6 * h2;
                        let env = (-t / decay).exp();
                        (h2 * env * 0.8, env < 0.001)
                    }
                    DrumKind::Tom => {
                        let base = note_to_freq(v.slot.note);
                        let f = base * (1.0 + 0.6 * (-t * 20.0).exp());
                        v.phase = fract(v.phase + f * dt);
                        let env = (-t * 6.0).exp();
                        ((v.phase * TAU).sin() * env, env < 0.001)
                    }
                };
                let x = x * v.slot.vel * vol;
                o[0] += x * pl;
                o[1] += x * pr;
                self.tap.add(v.slot.key, i, [x * pl, x * pr]);
                v.t += dt;
                if done {
                    v.active = false;
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- kicker

#[derive(Clone, Copy, Default)]
struct KickVoice {
    slot: VoiceSlot,
    active: bool,
    /// Seconds since the note started.
    t: f32,
    phase: f32,
}

/// The Kicker: a wave that falls from octaves above the note to the
/// note, fading out over the decay, with a drive (Boost) that squares it
/// off. Notes play out in full; a note off doesn't stop them.
struct Kicker {
    voices: [KickVoice; MAX_VOICES],
    clock: u64,
    tap: TrackTap,
}

impl Kicker {
    fn new() -> Self {
        Self { voices: [KickVoice::default(); MAX_VOICES], clock: 0, tap: TrackTap::new() }
    }
}

impl Dsp for Kicker {
    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.active));
        self.voices[i] = KickVoice {
            slot: VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false },
            active: true,
            ..Default::default()
        };
    }

    fn set_pitch(&mut self, key: u32, note: f32) {
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.active) {
            v.slot.note = note;
        }
    }

    fn set_pan(&mut self, key: u32, pan: f32) {
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.active) {
            v.slot.pan = pan;
        }
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.active = false;
        }
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, wave, drop, sweep, attack, decay) = (p[0], p[1].round() as u32, p[2], p[3], p[4], p[5]);
        let drive = 1.0 + p[6] * 15.0;
        let norm = 1.0 / drive.tanh();
        let dt = 1.0 / ctx.sr;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let (pl, pr) = pan_gains(p[7] + v.slot.pan);
            let base = note_to_freq(v.slot.note);
            for (i, o) in out.iter_mut().enumerate() {
                let t = v.t;
                let f = base * 2f32.powf(drop * (-t / sweep).exp());
                v.phase = fract(v.phase + f * dt);
                let x = match wave {
                    0 => sine(v.phase),
                    1 => 1.0 - 4.0 * (v.phase - 0.5).abs(),
                    _ => (v.phase * TAU).sin().signum(),
                };
                let env = (t / attack.max(1e-4)).min(1.0) * (-t / decay * 5.0).exp();
                let x = (x * drive).tanh() * norm * env * v.slot.vel * vol;
                o[0] += x * pl;
                o[1] += x * pr;
                self.tap.add(v.slot.key, i, [x * pl, x * pr]);
                v.t += dt;
                if env < 0.0005 && t > attack {
                    v.active = false;
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- SpectraVoice

const MAX_PARTIALS: usize = 32;

#[derive(Clone, Copy)]
struct SpectraVoiceVoice {
    slot: VoiceSlot,
    env: Adsr,
    /// Each partial's phase as a point on the unit circle, turned a step
    /// each frame: cheaper than a sine per partial per frame.
    z: [[f32; 2]; MAX_PARTIALS],
    /// Seconds since the note started, for the shimmer.
    t: f32,
}

impl Default for SpectraVoiceVoice {
    fn default() -> Self {
        Self { slot: VoiceSlot::default(), env: Adsr::default(), z: [[1.0, 0.0]; MAX_PARTIALS], t: 0.0 }
    }
}

/// The SpectraVoice: additive synthesis from up to 32
/// harmonics, the k-th at 1/k^slope of the first, with the even ones
/// turned down by Even (none is hollow, like a square), Stretch pulling
/// them sharp or flat like a stiff string, and Shimmer making each one
/// swell and fade at its own slow rate.
struct SpectraVoice {
    voices: [SpectraVoiceVoice; MAX_VOICES],
    clock: u64,
    tap: TrackTap,
}

impl SpectraVoice {
    fn new() -> Self {
        Self { voices: [SpectraVoiceVoice::default(); MAX_VOICES], clock: 0, tap: TrackTap::new() }
    }
}

impl Dsp for SpectraVoice {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        let v = &mut self.voices[i];
        *v = SpectraVoiceVoice::default();
        v.slot = VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false };
        v.env.trigger();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, harmonics, slope, even, stretch, shimmer) =
            (p[0], (p[1].round() as usize).clamp(1, MAX_PARTIALS), p[2], p[3], p[4], p[5]);
        let (a, d, s, r) = (p[6], p[7], p[8], p[9]);
        let block = out.len() as f32 / ctx.sr;
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let (pl, pr) = pan_gains(p[10] + v.slot.pan);
            let f0 = note_to_freq(v.slot.note);
            // Each partial's level and turn per frame for this block;
            // partials above Nyquist are left out.
            let mut amps = [0.0; MAX_PARTIALS];
            let mut rots = [[1.0, 0.0]; MAX_PARTIALS];
            let mut used = 0;
            let mut power = 0.0;
            for k in 0..harmonics {
                let h = (k + 1) as f32;
                let f = f0 * h * (1.0 + stretch * (h - 1.0));
                if f <= 0.0 || f >= ctx.sr * 0.45 {
                    break;
                }
                let mut amp = h.powf(-slope) * if k % 2 == 1 { even } else { 1.0 };
                power += amp * amp;
                let rate = 0.21 + 0.13 * h;
                amp *= 1.0 - shimmer * 0.5 * (1.0 + sine(v.t * rate + h * 0.37));
                amps[k] = amp;
                let (sn, cs) = (TAU * f / ctx.sr).sin_cos();
                rots[k] = [cs, sn];
                used = k + 1;
            }
            let norm = if power > 0.0 { 0.5 / power.sqrt() } else { 0.0 };
            for (i, o) in out.iter_mut().enumerate() {
                let env = v.env.next(ctx.sr, a, d, s, r);
                let mut x = 0.0;
                for k in 0..used {
                    let [c, sn] = v.z[k];
                    let [rc, rs] = rots[k];
                    v.z[k] = [c * rc - sn * rs, c * rs + sn * rc];
                    x += v.z[k][1] * amps[k];
                }
                let x = x * norm * env * v.slot.vel * vol;
                o[0] += x * pl;
                o[1] += x * pr;
                self.tap.add(v.slot.key, i, [x * pl, x * pr]);
            }
            // Keep the points on the circle as rounding drifts them.
            for z in &mut v.z[..used] {
                let m = (z[0] * z[0] + z[1] * z[1]).sqrt();
                *z = [z[0] / m, z[1] / m];
            }
            v.t += block;
        }
    }
}

// ---------------------------------------------------------------- FMX

/// Operators of an FMX voice.
const OPS: usize = 4;

/// For each FMX algorithm, which operators each one modulates, as a bit
/// per operator (bit 0 is operator 1). Operators only modulate lower ones,
/// so running them from 4 down to 1 has each one's input ready.
const FMX_ROUTES: [[u8; OPS]; 8] = [
    [0, 0b0001, 0b0010, 0b0100],
    [0, 0b0001, 0b0010, 0b0010],
    [0, 0b0001, 0b0010, 0b0001],
    [0, 0b0001, 0b0001, 0b0100],
    [0, 0b0001, 0, 0b0100],
    [0, 0, 0, 0b0111],
    [0, 0, 0, 0b0100],
    [0, 0, 0, 0],
];

/// How far a modulator at full level pushes its targets' phase, in cycles.
const FMX_DEPTH: f32 = 1.5;

/// An FMX voice's envelopes, one per operator.
#[derive(Clone, Copy, Default)]
struct OpEnvs([Adsr; OPS]);

impl OpEnvs {
    fn release(&mut self) {
        self.0.iter_mut().for_each(Adsr::release);
    }
}

#[derive(Clone, Copy, Default)]
struct FmxVoice {
    slot: VoiceSlot,
    env: OpEnvs,
    phase: [f32; OPS],
    /// Operator 4's last two outputs, for its feedback.
    fb: [f32; 2],
}

/// The FMX, a four-operator FM synth: each operator a sine at its
/// ratio of the note with its own level and envelope, wired by one of
/// eight algorithms, with feedback on operator 4. A voice sounds while an
/// operator that is heard does.
struct Fmx {
    voices: [FmxVoice; MAX_VOICES],
    clock: u64,
    tap: TrackTap,
}

impl Fmx {
    fn new() -> Self {
        Self { voices: [FmxVoice::default(); MAX_VOICES], clock: 0, tap: TrackTap::new() }
    }
}

/// The operators that modulate none, which are heard, in `routes`.
fn carriers(routes: &[u8; OPS]) -> impl Iterator<Item = usize> + '_ {
    (0..OPS).filter(|&k| routes[k] == 0)
}

impl Dsp for Fmx {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.0.iter().any(Adsr::active)));
        let v = &mut self.voices[i];
        *v = FmxVoice::default();
        v.slot = VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false };
        v.env.0.iter_mut().for_each(Adsr::trigger);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, fb) = (p[0], p[2]);
        let routes = &FMX_ROUTES[(p[1].round() as usize).min(FMX_ROUTES.len() - 1)];
        let op = |k: usize| &p[3 + k * 6..9 + k * 6];
        let heard = carriers(routes).count() as f32;
        for v in self.voices.iter_mut().filter(|v| carriers(routes).any(|k| v.env.0[k].active())) {
            let (pl, pr) = pan_gains(v.slot.pan);
            let f = note_to_freq(v.slot.note);
            let incs: [f32; OPS] = std::array::from_fn(|k| (f * op(k)[1] / ctx.sr).min(0.49));
            for (i, o) in out.iter_mut().enumerate() {
                let mut mods = [0.0; OPS];
                let mut x = 0.0;
                for k in (0..OPS).rev() {
                    let q = op(k);
                    let env = v.env.0[k].next(ctx.sr, q[2], q[3], q[4], q[5]);
                    let mut m = mods[k];
                    if k == OPS - 1 {
                        m += fb * 0.5 * (v.fb[0] + v.fb[1]);
                    }
                    let y = sine(v.phase[k] + m) * env * q[0];
                    v.phase[k] = fract(v.phase[k] + incs[k]);
                    if k == OPS - 1 {
                        v.fb = [y, v.fb[0]];
                    }
                    if routes[k] == 0 {
                        x += y;
                    }
                    for (t, to) in mods.iter_mut().enumerate().take(k) {
                        if routes[k] >> t & 1 == 1 {
                            *to += y * FMX_DEPTH;
                        }
                    }
                }
                let x = x / heard.sqrt() * v.slot.vel * vol;
                o[0] += x * pl;
                o[1] += x * pr;
                self.tap.add(v.slot.key, i, [x * pl, x * pr]);
            }
        }
    }
}

// ---------------------------------------------------------------- voice modulation

/// Copies `from` into `to` without giving up `to`'s storage, so edits
/// don't allocate on the audio thread once the points have room.
fn copy_envelope(to: &mut VoiceEnvelope, from: &VoiceEnvelope) {
    to.points.clear();
    to.points.extend_from_slice(&from.points);
    (to.on, to.sustain, to.curve, to.amount) = (from.on, from.sustain, from.curve, from.amount);
}

fn copy_modulation(to: &mut Modulation, m: &Modulation) {
    copy_envelope(&mut to.pitch, &m.pitch);
    copy_envelope(&mut to.filter_env, &m.filter_env);
    (to.filter, to.filter_mode, to.cutoff, to.resonance) = (m.filter, m.filter_mode, m.cutoff, m.resonance);
    to.vibrato = m.vibrato.clone();
    to.tremolo = m.tremolo.clone();
}

/// How far an LFO with a delay has faded in, `age` seconds into a note.
fn lfo_fade(lfo: &VoiceLfo, age: f32) -> f32 {
    if lfo.delay <= 0.0 { 1.0 } else { ((age - lfo.delay) / lfo.delay).clamp(0.0, 1.0) }
}

/// Where a voice is in its instrument's `Modulation`: seconds since the
/// note started, where the pitch and filter envelopes are (they stop at
/// their sustain points while it is held), the LFOs' phases and the
/// filter's state.
#[derive(Clone, Copy, Default)]
struct VoiceMod {
    age: f32,
    pitch_t: f32,
    filter_t: f32,
    vibrato_phase: f32,
    tremolo_phase: f32,
    svf: [Svf; 2],
}

/// What modulation does to a voice during one block.
struct ModBlock {
    /// Semitones to add to the pitch.
    bend: f32,
    /// The tremolo gain, moving by `trem_step` each frame.
    trem: f32,
    trem_step: f32,
    /// The filter's coefficients and mode, when it is on.
    filter: Option<(f32, f32, u8)>,
}

impl VoiceMod {
    /// Works out modulation for the next `frames` frames and moves on.
    fn block(&mut self, m: &Modulation, held: bool, frames: usize, sr: f32) -> ModBlock {
        let block = frames as f32 / sr;
        let mut bend = 0.0;
        if m.pitch.on {
            bend += (m.pitch.value(self.pitch_t) - 0.5) * 2.0 * m.pitch.amount;
            self.pitch_t = m.pitch.advance(self.pitch_t, block, held);
        }
        if m.vibrato.on {
            bend += lfo_shape(m.vibrato.shape as u32, self.vibrato_phase)
                * m.vibrato.depth
                * lfo_fade(&m.vibrato, self.age);
            self.vibrato_phase = fract(self.vibrato_phase + m.vibrato.rate * block);
        }
        let tremolo = |phase: f32, age: f32| {
            let depth = m.tremolo.depth * lfo_fade(&m.tremolo, age);
            1.0 - depth * (0.5 - 0.5 * lfo_shape(m.tremolo.shape as u32, phase))
        };
        let (from, to) = if m.tremolo.on {
            let from = tremolo(self.tremolo_phase, self.age);
            self.tremolo_phase = fract(self.tremolo_phase + m.tremolo.rate * block);
            (from, tremolo(self.tremolo_phase, self.age + block))
        } else {
            (1.0, 1.0)
        };
        let filter = m.filter.then(|| {
            let env = if m.filter_env.on { m.filter_env.value(self.filter_t) * m.filter_env.amount } else { 0.0 };
            self.filter_t = m.filter_env.advance(self.filter_t, block, held);
            let cutoff = (m.cutoff * 2f32.powf(env)).clamp(20.0, sr * 0.45);
            ((PI * cutoff / sr).tan(), 2.0 - 2.0 * m.resonance, m.filter_mode)
        });
        self.age += block;
        ModBlock { bend, trem: from, trem_step: (to - from) / frames.max(1) as f32, filter }
    }

    /// Filters a frame of the voice and applies tremolo.
    fn apply(&mut self, b: &mut ModBlock, mut x: Frame) -> Frame {
        if let Some((g, k, mode)) = b.filter {
            for (x, svf) in x.iter_mut().zip(&mut self.svf) {
                let (l, h, band) = svf.tick(*x, g, k);
                *x = match mode {
                    0 => l,
                    1 => h,
                    _ => band,
                };
            }
        }
        let g = b.trem;
        b.trem += b.trem_step;
        [x[0] * g, x[1] * g]
    }
}

// ---------------------------------------------------------------- sampler

/// What the sampler needs from a `SampleSlot`, without the strings.
struct Zone {
    /// `None` for a slot without audio, which keeps zone and slot indices
    /// lined up.
    data: Option<Arc<Sample>>,
    gain: f32,
    pan: f32,
    /// Semitones to add to the note before comparing with the base note.
    tune: f32,
    base_note: f32,
    loop_mode: u8,
    loop_start: f64,
    loop_end: f64,
    keys: [u8; 2],
    velocities: [u8; 2],
    /// Lines the whole sample takes, or 0.
    beat_sync: f64,
    oneshot: bool,
    mute_group: u8,
    autoseek: bool,
    /// The sample slot the zone plays, and the frames it plays: all of
    /// them, or one slice.
    slot: usize,
    /// Which slice of the slot this is, if one.
    slice: Option<usize>,
    start: f64,
    end: f64,
}

/// How long a voice cut by its mute group takes to fade out, in seconds.
const CHOKE_TIME: f32 = 0.005;

#[derive(Clone, Copy, Default)]
struct SamplerVoice {
    slot: VoiceSlot,
    env: Adsr,
    zone: usize,
    /// Address of the zone's audio when the voice started; if the audio is
    /// replaced the voice is stopped.
    data: usize,
    /// Read position in sample frames.
    pos: f64,
    /// Playing backwards inside a backward or ping-pong loop.
    backwards: bool,
    /// Playing the whole sample backwards, as Rxx asks, past its loop.
    reversed: bool,
    /// Semitones the note moves by to keep its pitch on a slice Sxx
    /// switched to.
    shift: f32,
    /// Fading out after its mute group cut it, from 1 down.
    choke: Option<f32>,
    /// Output frames to move on by before playing, for autoseek.
    seek: f64,
    md: VoiceMod,
}

struct Sampler {
    zones: Vec<Zone>,
    voices: [SamplerVoice; MAX_VOICES],
    clock: u64,
    /// The last note played: its key, note and velocity, for Sxx.
    last_on: Option<(u32, f32, f32)>,
    mods: Modulation,
    tap: TrackTap,
}

impl Sampler {
    fn new() -> Self {
        let voices = [SamplerVoice::default(); MAX_VOICES];
        Self { zones: Vec::new(), voices, clock: 0, last_on: None, mods: Modulation::default(), tap: TrackTap::new() }
    }

    /// Starts a voice playing zone `zi`, `shift` semitones from the note,
    /// cutting the others of its mute group.
    fn start(&mut self, zi: usize, key: u32, note: f32, vel: f32, shift: f32) {
        let z = &self.zones[zi];
        let Some(data) = &z.data else { return };
        let data = Arc::as_ptr(data) as usize;
        if z.mute_group > 0 {
            let zones = &self.zones;
            let same_group =
                |v: &SamplerVoice| v.zone != zi && zones.get(v.zone).is_some_and(|o| o.mute_group == z.mute_group);
            for v in self.voices.iter_mut().filter(|v| v.env.active() && v.choke.is_none() && same_group(v)) {
                v.choke = Some(1.0);
            }
        }
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        self.voices[i] = SamplerVoice {
            slot: VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false },
            zone: zi,
            data,
            pos: z.start,
            shift,
            ..Default::default()
        };
        self.voices[i].env.trigger();
    }
}

/// 4-point Hermite interpolation of `frames` at fractional position `pos`.
fn hermite(frames: &[Frame], pos: f64) -> Frame {
    let i = pos.floor() as isize;
    let t = (pos - i as f64) as f32;
    let last = frames.len() as isize - 1;
    let at = |k: isize| frames[(i + k).clamp(0, last) as usize];
    let (xm1, x0, x1, x2) = (at(-1), at(0), at(1), at(2));
    let mut out = [0.0; 2];
    for ch in 0..2 {
        let c1 = 0.5 * (x1[ch] - xm1[ch]);
        let c2 = xm1[ch] - 2.5 * x0[ch] + 2.0 * x1[ch] - 0.5 * x2[ch];
        let c3 = 0.5 * (x2[ch] - xm1[ch]) + 1.5 * (x0[ch] - x1[ch]);
        out[ch] = ((c3 * t + c2) * t + c1) * t + x0[ch];
    }
    out
}

impl Dsp for Sampler {
    voice_controls!();

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        // Every sample whose keyzone holds the note plays, so overlapping
        // zones layer.
        self.clock += 1;
        self.last_on = Some((key, note, vel));
        let (n, v) = (note.round().clamp(0.0, 127.0) as u8, (vel * 127.0).round().clamp(0.0, 127.0) as u8);
        for zi in 0..self.zones.len() {
            let z = &self.zones[zi];
            if z.data.is_some()
                && (z.keys[0]..=z.keys[1]).contains(&n)
                && (z.velocities[0]..=z.velocities[1]).contains(&v)
            {
                self.start(zi, key, note, vel, 0.0);
            }
        }
    }

    fn sample_offset(&mut self, key: u32, pos: f32) {
        let newest = self.voices.iter().filter(|v| v.slot.key == key && v.env.active()).map(|v| v.slot.age).max();
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.env.active() && Some(v.slot.age) == newest) {
            if let Some(data) = self.zones.get(v.zone).and_then(|z| z.data.as_ref()) {
                let z = &self.zones[v.zone];
                v.pos = z.start + pos as f64 * (z.end - z.start).min(data.len() as f64);
            }
        }
    }

    fn reverse(&mut self, key: u32, on: bool) {
        let newest = self.voices.iter().filter(|v| v.slot.key == key && v.env.active()).map(|v| v.slot.age).max();
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.env.active() && Some(v.slot.age) == newest) {
            let Some(z) = self.zones.get(v.zone) else { continue };
            if on && !v.reversed && v.pos <= z.start {
                let len = z.data.as_ref().map_or(0.0, |d| d.len() as f64);
                v.pos = (z.end.min(len) - 1.0).max(z.start);
            }
            v.reversed = on;
        }
    }

    fn play_slice(&mut self, key: u32, slice: usize) {
        // It follows the note it changes.
        let Some((_, note, vel)) = self.last_on.filter(|l| l.0 == key) else { return };
        let clock = self.clock;
        let mut started = false;
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.env.active() && v.slot.age == clock) {
            started = true;
            let Some(from) = self.zones.get(v.zone) else { continue };
            let Some(to) = self.zones.iter().position(|z| z.slot == from.slot && z.slice == Some(slice)) else {
                continue;
            };
            v.shift += self.zones[to].base_note - from.base_note;
            v.zone = to;
            v.pos = self.zones[to].start;
        }
        // A note that played nothing plays the slice of the first sliced
        // sample, pitched from the sample's base note.
        if !started && let Some(zi) = self.zones.iter().position(|z| z.slice == Some(slice)) {
            let whole = self.zones[self.zones[zi].slot].base_note;
            self.start(zi, key, note, vel, self.zones[zi].base_note - whole);
        }
    }

    fn seek(&mut self, key: u32, frames: f64) {
        let newest = self.voices.iter().filter(|v| v.slot.key == key && v.env.active()).map(|v| v.slot.age).max();
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.env.active() && Some(v.slot.age) == newest) {
            if self.zones.get(v.zone).is_some_and(|z| z.autoseek) {
                v.seek = frames;
            } else {
                v.env = Adsr::default();
            }
        }
    }

    fn note_off(&mut self, key: u32) {
        let zones = &self.zones;
        let oneshot = |v: &SamplerVoice| zones.get(v.zone).is_some_and(|z| z.oneshot);
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && !v.slot.released && !oneshot(v)) {
            v.slot.released = true;
            v.env.release();
        }
    }

    fn set_samples(&mut self, slots: &[SampleSlot]) {
        // A zone for each slot, lined up with them, then one for each slice.
        self.zones.clear();
        for (i, s) in slots.iter().enumerate() {
            let len = s.len() as f64;
            let loop_end = (s.loop_end as f64).clamp(1.0, len.max(1.0));
            let sliced = !s.slices.is_empty();
            let base = s.base_note;
            self.zones.push(Zone {
                data: s.data.clone(),
                gain: s.volume,
                pan: s.panning,
                tune: s.transpose as f32 + s.finetune as f32 / 100.0,
                base_note: s.base_note as f32,
                loop_mode: s.loop_mode,
                loop_start: (s.loop_start as f64).min(loop_end - 1.0),
                loop_end,
                // A sliced sample plays whole on its base note only.
                keys: if sliced { [base, base] } else { s.keys },
                velocities: s.velocities,
                beat_sync: s.beat_sync as f64,
                oneshot: s.oneshot,
                mute_group: s.mute_group,
                autoseek: s.autoseek,
                slot: i,
                slice: None,
                start: 0.0,
                end: len,
            });
        }
        for (i, s) in slots.iter().enumerate() {
            for (k, (from, to)) in s.slice_ranges().into_iter().enumerate() {
                let note = s.slice_note(k);
                let st = s.slice(k);
                let whole = &self.zones[i];
                let zone = Zone {
                    data: whole.data.clone(),
                    gain: whole.gain * st.volume,
                    pan: whole.pan + st.panning,
                    tune: whole.tune + st.transpose as f32 + st.finetune as f32 / 100.0,
                    base_note: note as f32,
                    // A slice loops over all of itself.
                    loop_mode: st.loop_mode,
                    loop_start: from as f64,
                    loop_end: to as f64,
                    oneshot: whole.oneshot || st.oneshot,
                    keys: [note, note],
                    slice: Some(k),
                    start: from as f64,
                    end: to as f64,
                    ..*whole
                };
                self.zones.push(zone);
            }
        }
        // Voices whose sample was removed or replaced stop; the rest pick
        // up the new settings.
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            match self.zones.get(v.zone).and_then(|z| z.data.as_ref()) {
                Some(data) if Arc::as_ptr(data) as usize == v.data => {}
                _ => v.env = Adsr::default(),
            }
        }
    }

    fn set_modulation(&mut self, m: &Modulation) {
        copy_modulation(&mut self.mods, m);
    }

    fn playheads(&self, out: &mut Vec<Playhead>) {
        for v in self.voices.iter().filter(|v| v.env.active()) {
            let envelopes = [v.md.pitch_t, v.md.filter_t];
            let slot = self.zones.get(v.zone).map_or(v.zone, |z| z.slot);
            out.push(Playhead { module: 0, slot, pos: v.pos, level: v.env.level * v.slot.vel, envelopes });
        }
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, pan, transpose) = (p[0], p[1], p[2].round());
        let (a, d, s, r) = (p[3], p[4], p[5], p[6]);

        let m = &self.mods;
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let Some(z) = self.zones.get(v.zone) else { continue };
            let Some(data) = &z.data else { continue };
            let frames = &data.frames[..];
            let len = frames.len() as f64;
            let (pl, pr) = pan_gains(pan + z.pan + v.slot.pan);

            let mut mb = v.md.block(m, !v.slot.released, out.len(), ctx.sr);
            let semis = v.slot.note + v.shift + transpose + z.tune - z.base_note + mb.bend;
            let pitch = 2f64.powf(semis as f64 / 12.0);
            let rate = if z.beat_sync > 0.0 {
                pitch * len / (z.beat_sync * ctx.samples_per_line as f64)
            } else {
                pitch * (data.sample_rate / ctx.sr) as f64
            };
            let (ls, le) = (z.loop_start, z.loop_end);
            let mut pos = v.pos;
            if v.seek > 0.0 {
                match seek_position(z, pos, rate * std::mem::take(&mut v.seek)) {
                    Some((p, backwards)) => (pos, v.backwards) = (p, backwards),
                    None => {
                        // It would have ended by now.
                        v.env = Adsr::default();
                        continue;
                    }
                }
            }
            for (i, o) in out.iter_mut().enumerate() {
                let env = v.env.next(ctx.sr, a, d, s, r);
                let x = v.md.apply(&mut mb, hermite(frames, pos));
                let g = env * v.slot.vel * vol * z.gain * v.choke.unwrap_or(1.0);
                o[0] += x[0] * g * pl;
                o[1] += x[1] * g * pr;
                self.tap.add(v.slot.key, i, [x[0] * g * pl, x[1] * g * pr]);
                if let Some(c) = &mut v.choke {
                    *c -= 1.0 / (CHOKE_TIME * ctx.sr);
                    if *c <= 0.0 {
                        v.env = Adsr::default();
                        break;
                    }
                }

                if v.reversed {
                    pos -= rate;
                } else if v.backwards {
                    pos -= rate;
                    if pos < ls {
                        if z.loop_mode == 3 {
                            // Ping-pong: bounce off the loop start.
                            v.backwards = false;
                            pos = (2.0 * ls - pos).min(le - 1.0);
                        } else {
                            pos += le - ls;
                        }
                    }
                } else {
                    pos += rate;
                    // Loops only start once playback reaches them; a 9xx
                    // offset past the loop end plays out the rest.
                    if pos >= le && pos - rate < le {
                        match z.loop_mode {
                            1 => pos = ls + (pos - ls) % (le - ls),
                            2 | 3 => {
                                // Turn around at the last frame in the loop.
                                v.backwards = true;
                                pos = (2.0 * (le - 1.0) - pos).max(ls);
                            }
                            _ => {}
                        }
                    }
                }
                if pos >= z.end.min(len) || pos < 0.0 || (v.reversed && pos < z.start) {
                    v.env = Adsr::default();
                    break;
                }
            }
            v.pos = pos.clamp(0.0, len - 1.0);
        }
    }
}

/// Where a voice of zone `z` at `pos` is after playing `dist` frames of
/// the sample forwards, through its loop, and whether it is then playing
/// backwards; `None` if it has played out.
fn seek_position(z: &Zone, pos: f64, dist: f64) -> Option<(f64, bool)> {
    let (ls, le) = (z.loop_start, z.loop_end);
    let to = pos + dist;
    let span = le - ls;
    if z.loop_mode == 0 || pos >= le || to < le || span < 1.0 {
        return (to < z.end).then_some((to, false));
    }
    let over = to - le;
    Some(match z.loop_mode {
        1 => (ls + over % span, false),
        // Backward: from the loop end down to its start, over and over.
        2 => (le - 1.0 - over % span, true),
        // Ping-pong: down from the end, then up from the start.
        _ => {
            let k = over % (2.0 * span);
            if k < span { ((le - 1.0 - k).max(ls), true) } else { (ls + (k - span), false) }
        }
    })
}

// ---------------------------------------------------------------- filter

/// Topology-preserving state variable filter (Simper).
#[derive(Default, Clone, Copy)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    fn tick(&mut self, x: f32, g: f32, k: f32) -> (f32, f32, f32) {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        let low = v2;
        let band = v1;
        let high = x - k * v1 - v2;
        (low, high, band)
    }
}

#[derive(Default)]
struct Filter {
    svf: [Svf; 2],
    lfo: f32,
}

impl Dsp for Filter {
    fn reset(&mut self) {
        self.svf = Default::default();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let mode = p[0].round() as u32;
        let k = 2.0 - 2.0 * p[2];
        let lfo_inc = p[3] / ctx.sr;
        for (o, i) in out.iter_mut().zip(input) {
            let lfo = (self.lfo * TAU).sin() * p[4] * 3.0;
            self.lfo = fract(self.lfo + lfo_inc);
            let cutoff = (p[1] * 2f32.powf(lfo)).clamp(20.0, ctx.sr * 0.45);
            let g = (PI * cutoff / ctx.sr).tan();
            for ch in 0..2 {
                let (l, h, b) = self.svf[ch].tick(i[ch], g, k);
                o[ch] = match mode {
                    0 => l,
                    1 => h,
                    _ => b,
                };
            }
        }
    }
}

// ---------------------------------------------------------------- distortion

#[derive(Default)]
struct Distortion {
    lp: [f32; 2],
    /// The held input of the sample-rate reducer and how long it is held.
    hold: Frame,
    held: f32,
}

impl Dsp for Distortion {
    fn reset(&mut self) {
        self.lp = [0.0; 2];
        self.hold = [0.0; 2];
        self.held = 0.0;
    }

    fn process(&mut self, _: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (drive, tone, mix) = (p[0], p[1], p[2]);
        let kind = p[3].round() as u32;
        let steps = 2f32.powf(p[4].round() - 1.0);
        let crush = p[4] < 15.5;
        let downsample = p[5].round().max(1.0);
        let norm = 1.0 / drive.tanh();
        let coef = 0.05 + 0.95 * tone * tone;
        for (o, i) in out.iter_mut().zip(input) {
            // Sample-rate reduction: hold each input for `downsample` frames.
            self.held -= 1.0;
            if self.held <= 0.0 {
                self.hold = *i;
                self.held += downsample;
            }
            for ch in 0..2 {
                let x = self.hold[ch] * drive;
                let mut wet = match kind {
                    0 => x.tanh() * norm,
                    1 => x.clamp(-1.0, 1.0),
                    // Folds back from ±1 instead of flattening.
                    _ => {
                        let t = (x + 1.0).rem_euclid(4.0);
                        if t < 2.0 { t - 1.0 } else { 3.0 - t }
                    }
                };
                if crush {
                    wet = (wet * steps).round() / steps;
                }
                self.lp[ch] += coef * (wet - self.lp[ch]);
                o[ch] = i[ch] * (1.0 - mix) + self.lp[ch] * mix * 0.7;
            }
        }
    }
}

// ---------------------------------------------------------------- delay lines

/// A ring of the last frames written, for the modules built on delays.
struct DelayLine {
    buf: Vec<Frame>,
    pos: usize,
}

impl DelayLine {
    /// Room for `seconds` of sound, and a few frames more.
    fn new(sr: f32, seconds: f32) -> Self {
        Self { buf: vec![[0.0; 2]; (sr * seconds) as usize + 4], pos: 0 }
    }

    fn len(&self) -> usize {
        self.buf.len()
    }

    fn clear(&mut self) {
        self.buf.fill([0.0; 2]);
    }

    /// The frame written `d` frames ago: 1 is the last one.
    fn at(&self, d: usize) -> Frame {
        let len = self.buf.len();
        // Wrapped with a comparison: a division would cost more than the
        // rest of a tap.
        let i = self.pos + len - d.clamp(1, len - 1);
        self.buf[if i >= len { i - len } else { i }]
    }

    /// Channel `ch` `d` frames back, between frames.
    fn tap_ch(&self, ch: usize, d: f32) -> f32 {
        let d = d.clamp(1.0, (self.buf.len() - 2) as f32);
        let back = d as usize;
        let (a, b) = (self.at(back)[ch], self.at(back + 1)[ch]);
        a + (b - a) * fract(d)
    }

    /// Both channels `d` frames back, between frames.
    fn tap(&self, d: f32) -> Frame {
        [self.tap_ch(0, d), self.tap_ch(1, d)]
    }

    fn push(&mut self, x: Frame) {
        self.buf[self.pos] = x;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
    }
}

struct Delay {
    line: DelayLine,
}

impl Delay {
    fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 6.0) }
    }
}

impl Dsp for Delay {
    fn reset(&mut self) {
        self.line.clear();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (time, fb, mix, cross) = (p[0], p[1], p[2], p[3]);
        let d = (time * ctx.samples_per_line) as usize;
        for (o, i) in out.iter_mut().zip(input) {
            let rd = self.line.at(d);
            let wl = i[0] + fb * ((1.0 - cross) * rd[0] + cross * rd[1]);
            let wr = i[1] + fb * ((1.0 - cross) * rd[1] + cross * rd[0]);
            self.line.push([wl, wr]);
            *o = [i[0] + rd[0] * mix, i[1] + rd[1] * mix];
        }
    }
}

// ---------------------------------------------------------------- reverb

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    store: f32,
}

impl Comb {
    fn tick(&mut self, x: f32, fb: f32, damp: f32) -> f32 {
        let y = self.buf[self.pos];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.pos] = x + self.store * fb;
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

impl Allpass {
    fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.pos];
        self.buf[self.pos] = x + b * 0.5;
        self.pos = (self.pos + 1) % self.buf.len();
        b - x
    }
}

/// Freeverb.
struct Reverb {
    combs: [Vec<Comb>; 2],
    allpasses: [Vec<Allpass>; 2],
}

impl Reverb {
    fn new(sr: f32) -> Self {
        const COMBS: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
        const ALLPASSES: [usize; 4] = [556, 441, 341, 225];
        const SPREAD: usize = 23;
        let scale = |n: usize| ((n as f32 * sr / 44100.0) as usize).max(1);
        let combs = |s| COMBS.iter().map(|&n| Comb { buf: vec![0.0; scale(n + s)], pos: 0, store: 0.0 }).collect();
        let aps = |s| ALLPASSES.iter().map(|&n| Allpass { buf: vec![0.0; scale(n + s)], pos: 0 }).collect();
        Self { combs: [combs(0), combs(SPREAD)], allpasses: [aps(0), aps(SPREAD)] }
    }
}

impl Dsp for Reverb {
    fn reset(&mut self) {
        for ch in 0..2 {
            for c in &mut self.combs[ch] {
                c.buf.fill(0.0);
                c.store = 0.0;
            }
            for a in &mut self.allpasses[ch] {
                a.buf.fill(0.0);
            }
        }
    }

    fn process(&mut self, _: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let fb = 0.7 + 0.28 * p[0];
        let damp = p[1] * 0.4;
        let mix = p[2];
        for (o, i) in out.iter_mut().zip(input) {
            let x = (i[0] + i[1]) * 0.015;
            for ch in 0..2 {
                let mut y: f32 = self.combs[ch].iter_mut().map(|c| c.tick(x, fb, damp)).sum();
                for a in &mut self.allpasses[ch] {
                    y = a.tick(y);
                }
                o[ch] = i[ch] * (1.0 - mix) + y * mix * 3.0;
            }
        }
    }
}

// ---------------------------------------------------------------- LFO

/// The LFO: moves the volume (tremolo) or the panning of its input.
#[derive(Default)]
struct Lfo {
    phase: f32,
}

pub fn lfo_shape(shape: u32, phase: f32) -> f32 {
    match shape {
        0 => (phase * TAU).sin(),
        1 => 1.0 - 4.0 * (phase - 0.5).abs(),
        2 => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        3 => 1.0 - 2.0 * phase,
        _ => 2.0 * phase - 1.0,
    }
}

impl Dsp for Lfo {
    fn reset(&mut self) {
        self.phase = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (pan, shape, depth, synced) = (p[0] >= 0.5, p[1].round() as u32, p[2], p[4] >= 0.5);
        let inc = if synced { 1.0 / (p[5] * ctx.samples_per_line).max(1.0) } else { p[3] / ctx.sr };
        // Synced to lines, its cycle follows the song while it plays.
        if synced && let Some(at) = ctx.song_line {
            self.phase = (at / p[5].max(1e-3) as f64).rem_euclid(1.0) as f32;
        }
        for (o, i) in out.iter_mut().zip(input) {
            let w = lfo_shape(shape, self.phase);
            self.phase = fract(self.phase + inc);
            if pan {
                let (l, r) = pan_gains(w * depth);
                *o = [i[0] * l, i[1] * r];
            } else {
                let g = 1.0 - depth * (0.5 - 0.5 * w);
                *o = [i[0] * g, i[1] * g];
            }
        }
    }
}

// ---------------------------------------------------------------- flanger

/// A modulated delay: one voice with feedback as a flanger, or two voices
/// moving in opposite directions per channel as a chorus.
struct Flanger {
    line: DelayLine,
    phase: f32,
}

impl Flanger {
    fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 0.08), phase: 0.0 }
    }
}

impl Dsp for Flanger {
    fn reset(&mut self) {
        self.line.clear();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let chorus = p[0] >= 0.5;
        let base = p[1] * ctx.sr;
        let depth = p[2];
        let inc = p[3] / ctx.sr;
        let fb = p[4];
        let mix = p[5];
        // A chorus sweeps further, around a longer delay.
        let (base, sweep) = if chorus { (base + 0.01 * ctx.sr, 0.008 * ctx.sr * depth) } else { (base, base * depth) };
        for (o, i) in out.iter_mut().zip(input) {
            let mut wet = [0.0; 2];
            for (ch, w) in wet.iter_mut().enumerate() {
                // The right channel runs a quarter cycle behind.
                let ph = self.phase + ch as f32 * 0.25;
                let d = |ph: f32| base + sweep * 0.5 * (1.0 + sine(ph));
                let tap = |ph| self.line.tap_ch(ch, d(ph));
                *w = if chorus { 0.5 * (tap(ph) + tap(ph + 0.5)) } else { tap(ph) };
            }
            let fb = if chorus { fb * 0.5 } else { fb };
            self.line.push([i[0] + wet[0] * fb, i[1] + wet[1] * fb]);
            self.phase = fract(self.phase + inc);
            for ch in 0..2 {
                o[ch] = i[ch] * (1.0 - mix * 0.5) + wet[ch] * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- phaser

/// The most allpass stages a phaser runs.
const MAX_STAGES: usize = 12;

/// A phaser: a chain of first-order allpass filters swept
/// between two frequencies by an LFO, mixed back with the input so their
/// phase shifts cut notches. The right channel runs a quarter cycle
/// behind.
#[derive(Default)]
struct Phaser {
    /// Each channel's stages, as (last input, last output).
    stages: [[(f32, f32); MAX_STAGES]; 2],
    last: [f32; 2],
    phase: f32,
}

impl Dsp for Phaser {
    fn reset(&mut self) {
        *self = Phaser { phase: self.phase, ..Default::default() };
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (rate, depth, low, high) = (p[0], p[1], p[2], p[3].max(p[2]));
        let (stages, fb, mix) = ((p[4].round() as usize).clamp(1, MAX_STAGES), p[5], p[6]);
        let inc = rate / ctx.sr;
        let nyquist = ctx.sr * 0.45;
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let lfo = 0.5 + 0.5 * sine(self.phase + ch as f32 * 0.25);
                // Sweeps in octaves around the middle of the range.
                let f = (low * (high / low).powf(0.5 + (lfo - 0.5) * depth)).clamp(10.0, nyquist);
                let t = (PI * f / ctx.sr).tan();
                let a = (t - 1.0) / (t + 1.0);
                let mut x = i[ch] + fb * self.last[ch];
                for s in &mut self.stages[ch][..stages] {
                    let y = a * x + s.0 - a * s.1;
                    *s = (x, y);
                    x = y;
                }
                self.last[ch] = x;
                o[ch] = i[ch] * (1.0 - mix) + x * mix;
            }
            self.phase = fract(self.phase + inc);
        }
    }
}

// ---------------------------------------------------------------- vocal filter

/// The formants of a tenor singing A, E, I, O and U (Csound's table): the
/// frequency in Hz, the level in dB and the bandwidth in Hz of each.
const VOWELS: [[(f32, f32, f32); 5]; 5] = [
    [(650.0, 0.0, 80.0), (1080.0, -6.0, 90.0), (2650.0, -7.0, 120.0), (2900.0, -8.0, 130.0), (3250.0, -22.0, 140.0)],
    [(400.0, 0.0, 70.0), (1700.0, -14.0, 80.0), (2600.0, -12.0, 100.0), (3200.0, -14.0, 120.0), (3580.0, -20.0, 120.0)],
    [(290.0, 0.0, 40.0), (1870.0, -15.0, 90.0), (2800.0, -18.0, 100.0), (3250.0, -20.0, 120.0), (3540.0, -30.0, 120.0)],
    [(400.0, 0.0, 40.0), (800.0, -10.0, 80.0), (2600.0, -12.0, 100.0), (2800.0, -12.0, 120.0), (3000.0, -26.0, 120.0)],
    [(350.0, 0.0, 40.0), (600.0, -20.0, 60.0), (2700.0, -17.0, 100.0), (2900.0, -14.0, 120.0), (3300.0, -26.0, 120.0)],
];

/// The Vocal Filter: band-pass filters on the formants of a vowel,
/// morphing from one vowel to the next, which make any sound say it.
#[derive(Default)]
struct VocalFilter {
    bands: [[Svf; 2]; 5],
}

impl Dsp for VocalFilter {
    fn reset(&mut self) {
        self.bands = Default::default();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let vowel = p[0].clamp(0.0, 4.0);
        let (shift, width) = (2f32.powf(p[1] / 12.0), p[2]);
        let (count, gain, mix) = ((p[3].round() as usize).clamp(1, 5), p[4], p[5]);
        let (k, t) = ((vowel.floor() as usize).min(3), vowel - (vowel.floor()).min(3.0));
        // Each formant's filter coefficients and level, for this block.
        let mut formants = [(0.0f32, 0.0f32, 0.0f32); 5];
        for (n, f) in formants.iter_mut().enumerate().take(count) {
            let (a, b) = (VOWELS[k][n], VOWELS[k + 1][n]);
            let lerp = |x: f32, y: f32| x + (y - x) * t;
            let freq = (lerp(a.0, b.0) * shift).clamp(20.0, ctx.sr * 0.45);
            let level = 10f32.powf(lerp(a.1, b.1) / 20.0);
            let bw = lerp(a.2, b.2) * width * shift;
            // The band output peaks at 1/k, so k scales it back to the level.
            let k = (bw / freq).clamp(0.005, 2.0);
            *f = ((PI * freq / ctx.sr).tan(), k, level * k);
        }
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let mut wet = 0.0;
                for (band, &(g, k, level)) in self.bands.iter_mut().zip(&formants).take(count) {
                    wet += band[ch].tick(i[ch], g, k).2 * level;
                }
                o[ch] = i[ch] * (1.0 - mix) + wet * gain * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- repeater

/// How long the Repeater takes to switch between its input and its loop,
/// and to blend the loop's end into its start, in seconds.
const REPEATER_FADE: f32 = 0.003;

/// The Repeater: while Hold is on it plays the last
/// Length of its input over and over, a stutter that follows the tempo.
struct Repeater {
    buf: Vec<Frame>,
    pos: usize,
    /// Where the held loop starts in `buf`, and how far into it the
    /// playback is; `None` while not holding.
    held: Option<(usize, usize)>,
    /// How much of the loop is heard, moving towards Hold.
    wet: f32,
}

impl Repeater {
    fn new(sr: f32) -> Self {
        Self { buf: vec![[0.0; 2]; (sr * 8.0) as usize], pos: 0, held: None, wet: 0.0 }
    }
}

impl Dsp for Repeater {
    fn reset(&mut self) {
        self.buf.fill([0.0; 2]);
        self.held = None;
        self.wet = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let hold = p[0] >= 0.5;
        let lines =
            crate::project::REPEATER_LINES[(p[1].round() as usize).min(crate::project::REPEATER_LINES.len() - 1)];
        let mix = p[2];
        let size = self.buf.len();
        let len = ((lines * ctx.samples_per_line) as usize).clamp(16, size / 2);
        let fade = ((REPEATER_FADE * ctx.sr) as usize).clamp(1, len / 4);
        let step = 1.0 / fade as f32;
        if hold && self.held.is_none() {
            // The loop is the stretch of input just before Hold came on.
            self.held = Some(((self.pos + size - len) % size, 0));
        }
        for (o, i) in out.iter_mut().zip(input) {
            let target = if hold { 1.0 } else { 0.0 };
            self.wet = if self.wet < target { (self.wet + step).min(target) } else { (self.wet - step).max(target) };
            let mut looped = [0.0; 2];
            if let Some((start, at)) = &mut self.held {
                let at_ = *at % len;
                looped = self.buf[(*start + at_) % size];
                // Near its end the loop fades into the audio before its
                // start, so it wraps without a click.
                if at_ + fade >= len {
                    let t = (len - at_) as f32 * step;
                    let before = self.buf[(*start + size - (len - at_)) % size];
                    for ch in 0..2 {
                        looped[ch] = looped[ch] * t + before[ch] * (1.0 - t);
                    }
                }
                *at = at_ + 1;
            } else {
                self.buf[self.pos] = *i;
                self.pos = (self.pos + 1) % size;
            }
            if !hold && self.wet == 0.0 {
                self.held = None;
            }
            let w = self.wet * mix;
            for ch in 0..2 {
                o[ch] = i[ch] * (1.0 - w) + looped[ch] * w;
            }
        }
    }
}

// ---------------------------------------------------------------- ring modulator

/// The Ring Mod: the input times a carrier wave, which turns each of
/// its frequencies into their sum and difference with the carrier's.
/// Stereo puts the right channel's carrier up to half a cycle behind.
#[derive(Default)]
struct RingMod {
    phase: f32,
}

impl Dsp for RingMod {
    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (inc, shape, stereo, mix) = (p[0] / ctx.sr, p[1].round() as u32, p[2], p[3]);
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let carrier = lfo_shape(shape, fract(self.phase + ch as f32 * stereo));
                o[ch] = i[ch] * (1.0 - mix) + i[ch] * carrier * mix;
            }
            self.phase = fract(self.phase + inc);
        }
    }
}

// ---------------------------------------------------------------- gate

/// The Gate: lets the sound through while it is louder than the
/// threshold, and for the hold time after, and turns it down to the floor
/// otherwise, opening over the attack and closing over the release.
#[derive(Default)]
struct Gate {
    /// How far open the gate is, from 0 (at the floor) to 1.
    open: f32,
    /// Frames left before the gate starts closing.
    hold: f32,
}

impl Dsp for Gate {
    fn reset(&mut self) {
        self.open = 0.0;
        self.hold = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (threshold, floor) = (p[0], p[4]);
        let coef = |t: f32| 1.0 - (-1.0 / (t.max(1e-5) * ctx.sr)).exp();
        let (att, rel) = (coef(p[1]), coef(p[3]));
        for (o, i) in out.iter_mut().zip(input) {
            if i[0].abs().max(i[1].abs()) > threshold {
                self.hold = p[2] * ctx.sr;
            }
            if self.hold > 0.0 {
                self.hold -= 1.0;
                self.open += att * (1.0 - self.open);
            } else {
                self.open -= rel * self.open;
            }
            let gain = floor + (1.0 - floor) * self.open;
            *o = [i[0] * gain, i[1] * gain];
        }
    }
}

// ---------------------------------------------------------------- pitch shifter

/// The Pitch Shifter: two read heads sweep through a short delay
/// faster or slower than it fills, each fading in and out over a grain so
/// one is always loud while the other jumps back. Feedback sends the
/// shifted sound round again, for rising or falling cascades.
struct PitchShifter {
    line: DelayLine,
    /// Where the first head is in its grain, 0..1; the second is half a
    /// grain on.
    phase: f32,
}

impl PitchShifter {
    fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 0.25), phase: 0.0 }
    }
}

impl Dsp for PitchShifter {
    fn reset(&mut self) {
        self.line.clear();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let ratio = 2f32.powf((p[0] + p[1] / 100.0) / 12.0);
        let grain = (p[2] * ctx.sr).min(self.line.len() as f32 - 4.0);
        let (fb, mix) = (p[3], p[4]);
        // The delay shrinks by ratio - 1 frames a frame, so the heads read
        // `ratio` frames a frame.
        let step = (1.0 - ratio) / grain;
        for (o, i) in out.iter_mut().zip(input) {
            let mut wet = [0.0; 2];
            for head in [0.0, 0.5] {
                let ph = fract(self.phase + head);
                // sin² windows half a grain apart add up to one.
                let w = (PI * ph).sin().powi(2);
                let x = self.line.tap(1.0 + ph * grain);
                wet = [wet[0] + x[0] * w, wet[1] + x[1] * w];
            }
            self.line.push([i[0] + wet[0] * fb, i[1] + wet[1] * fb]);
            self.phase = (self.phase + step).rem_euclid(1.0);
            for ch in 0..2 {
                o[ch] = i[ch] * (1.0 - mix) + wet[ch] * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- stereo expander

/// The Stereo Expander: the difference between the channels (the
/// side) scaled by Width, from mono at 0 through unchanged at 100% to
/// twice as wide, with the side below Mono bass taken out so the low end
/// stays centred.
#[derive(Default)]
struct StereoExpander {
    /// The side's lows, for Mono bass.
    low: f32,
}

impl Dsp for StereoExpander {
    fn reset(&mut self) {
        self.low = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let width = p[0];
        let coef = 1.0 - (-TAU * p[1] / ctx.sr).exp();
        for (o, i) in out.iter_mut().zip(input) {
            let mid = (i[0] + i[1]) * 0.5;
            let side = (i[0] - i[1]) * 0.5;
            self.low += coef * (side - self.low);
            let side = (side - self.low) * width;
            *o = [mid + side, mid - side];
        }
    }
}

// ---------------------------------------------------------------- comb filter

/// The Comb Filter: a delay one cycle of a note long fed back into
/// itself, which rings at the note and its harmonics (or, with negative
/// feedback, an octave down and its odd harmonics), with damping that
/// dulls the higher ones.
struct CombFilter {
    line: DelayLine,
    lp: Frame,
}

impl CombFilter {
    fn new(sr: f32) -> Self {
        // Long enough for the lowest note, C-1 at 32.7 Hz.
        Self { line: DelayLine::new(sr, 1.0 / 30.0), lp: [0.0; 2] }
    }
}

impl Dsp for CombFilter {
    fn reset(&mut self) {
        self.line.clear();
        self.lp = [0.0; 2];
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let d = ctx.sr / note_to_freq(p[0] + p[1] / 100.0);
        let (fb, mix) = (p[2], p[4]);
        let damp = p[3] * 0.95;
        for (o, i) in out.iter_mut().zip(input) {
            let delayed = self.line.tap(d);
            let mut y = [0.0; 2];
            for ch in 0..2 {
                self.lp[ch] = delayed[ch] * (1.0 - damp) + self.lp[ch] * damp;
                y[ch] = i[ch] + self.lp[ch] * fb;
                o[ch] = i[ch] * (1.0 - mix) + y[ch] * mix * (1.0 - fb.abs());
            }
            self.line.push(y);
        }
    }
}

// ---------------------------------------------------------------- maximizer

/// How far ahead the Maximizer looks, in seconds.
const LOOKAHEAD: f32 = 0.0015;

/// The Maximizer: boosts the sound and keeps its peaks under the
/// ceiling. It looks a moment ahead, so the gain is already down when a
/// peak arrives, and lets the gain back up over the release.
struct Maximizer {
    /// The boosted input, waiting out the lookahead.
    delay: Vec<Frame>,
    /// The gain each frame in `delay` needs to stay under the ceiling.
    needs: Vec<f32>,
    pos: usize,
    gain: f32,
}

impl Maximizer {
    fn new(sr: f32) -> Self {
        let len = (sr * LOOKAHEAD) as usize + 1;
        Self { delay: vec![[0.0; 2]; len], needs: vec![1.0; len], pos: 0, gain: 1.0 }
    }
}

impl Dsp for Maximizer {
    fn reset(&mut self) {
        self.delay.fill([0.0; 2]);
        self.needs.fill(1.0);
        self.gain = 1.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (boost, ceiling) = (p[0], p[1]);
        let rel = 1.0 - (-1.0 / (p[2] * ctx.sr)).exp();
        for (o, i) in out.iter_mut().zip(input) {
            let x = [i[0] * boost, i[1] * boost];
            let peak = x[0].abs().max(x[1].abs());
            self.needs[self.pos] = if peak > ceiling { ceiling / peak } else { 1.0 };
            // The oldest frame leaves as this one comes in; the lowest
            // need over what's waiting covers every peak still to come.
            let y = std::mem::replace(&mut self.delay[self.pos], x);
            self.pos = (self.pos + 1) % self.delay.len();
            let need = self.needs.iter().fold(1.0f32, |m, &n| m.min(n));
            self.gain = if need < self.gain { need } else { self.gain + rel * (need - self.gain) };
            *o = [(y[0] * self.gain).clamp(-ceiling, ceiling), (y[1] * self.gain).clamp(-ceiling, ceiling)];
        }
    }
}

// ---------------------------------------------------------------- exciter

/// The Exciter: the highs above Frequency, driven into a soft clip
/// that gives them new overtones, added back to the sound for air and
/// presence.
#[derive(Default)]
struct Exciter {
    /// Two one-pole lowpasses per channel; the input less them is the highs.
    lp: [[f32; 2]; 2],
}

impl Dsp for Exciter {
    fn reset(&mut self) {
        self.lp = [[0.0; 2]; 2];
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let coef = 1.0 - (-TAU * p[0].min(ctx.sr * 0.45) / ctx.sr).exp();
        let (drive, amount) = (p[1], p[2]);
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let lp = &mut self.lp[ch];
                let hp1 = i[ch] - lp[0];
                lp[0] += coef * hp1;
                let hp2 = hp1 - lp[1];
                lp[1] += coef * hp2;
                o[ch] = i[ch] + (hp2 * drive).tanh() * amount;
            }
        }
    }
}

// ---------------------------------------------------------------- DC blocker

/// The DC Blocker: a highpass far below hearing that takes away an
/// offset (from distortion, ring modulation or uneven waves) so it doesn't
/// eat into the headroom.
#[derive(Default)]
struct DcBlocker {
    /// The last input and output of each channel.
    last: [Frame; 2],
}

impl Dsp for DcBlocker {
    fn reset(&mut self) {
        self.last = [[0.0; 2]; 2];
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let r = (-TAU * p[0] / ctx.sr).exp();
        let [x1, y1] = &mut self.last;
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                o[ch] = i[ch] - x1[ch] + r * y1[ch];
                x1[ch] = i[ch];
                y1[ch] = o[ch];
            }
        }
    }
}

// ---------------------------------------------------------------- scream filter

/// The Scream Filter: a resonant filter driven hard, with the
/// distortion inside its loop, so the resonance growls and can scream at
/// full without running away.
#[derive(Default)]
struct ScreamFilter {
    svf: [Svf; 2],
}

impl Dsp for ScreamFilter {
    fn reset(&mut self) {
        self.svf = Default::default();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let mode = p[0].round() as u32;
        let g = (PI * p[1].clamp(20.0, ctx.sr * 0.45) / ctx.sr).tan();
        let k = 2.0 - 2.0 * p[2];
        let (drive, mix) = (p[3], p[4]);
        let norm = 1.0 / drive.tanh();
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let svf = &mut self.svf[ch];
                let (l, h, b) = svf.tick(i[ch] * drive, g, k);
                // Saturating the band state bounds the resonance.
                svf.ic1 = svf.ic1.tanh();
                let y = match mode {
                    0 => l,
                    1 => h,
                    _ => b,
                };
                o[ch] = i[ch] * (1.0 - mix) + y.tanh() * norm * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- multitap delay

/// The Multitap Delay: four echoes, each with its own time in
/// lines, level and pan, the longest fed back for more.
struct Multitap {
    line: DelayLine,
}

impl Multitap {
    fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 6.0) }
    }
}

impl Dsp for Multitap {
    fn reset(&mut self) {
        self.line.clear();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let taps: [(usize, f32, f32, f32); 4] = std::array::from_fn(|t| {
            let d = (p[t * 3] * ctx.samples_per_line) as usize;
            let (l, r) = pan_gains(p[t * 3 + 2]);
            (d, p[t * 3 + 1], l, r)
        });
        let longest = taps.iter().map(|t| t.0).max().unwrap_or(1);
        let (fb, mix) = (p[12], p[13]);
        for (o, i) in out.iter_mut().zip(input) {
            let mut wet = [0.0; 2];
            for &(d, level, l, r) in &taps {
                let x = self.line.at(d);
                // Each tap is panned from the middle of what it hears.
                let m = (x[0] + x[1]) * 0.5 * level;
                wet = [wet[0] + m * l, wet[1] + m * r];
            }
            let back = self.line.at(longest);
            self.line.push([i[0] + back[0] * fb, i[1] + back[1] * fb]);
            *o = [i[0] + wet[0] * mix, i[1] + wet[1] * mix];
        }
    }
}

// ---------------------------------------------------------------- EQ 10

/// The EQ 10: a graphic EQ, ten peaks an octave apart from 31 Hz
/// to 16 kHz, each up or down by 12 dB.
#[derive(Default)]
struct Eq10 {
    bands: [Biquad; 10],
}

impl Dsp for Eq10 {
    fn reset(&mut self) {
        self.bands.iter_mut().for_each(Biquad::clear);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        for ((b, &freq), &gain) in self.bands.iter_mut().zip(&EQ10_FREQS).zip(p) {
            b.design(0, freq, gain, ctx.sr);
        }
        run_bands(&mut self.bands, input, out);
    }
}

// ---------------------------------------------------------------- cabinet simulator

/// Each cabinet's tone as (shelf or peak, frequency, gain) bands, as
/// `Biquad::design` takes them: a speaker's lows and highs rolled off and
/// its body and bite.
const CABINET_BANDS: [[(i32, f32, f32); 4]; 4] = [
    [(-1, 120.0, 0.1), (0, 400.0, 1.3), (0, 1800.0, 2.0), (1, 4500.0, 0.08)],
    [(-1, 80.0, 0.2), (0, 110.0, 1.6), (0, 2500.0, 1.8), (1, 5000.0, 0.05)],
    [(-1, 40.0, 0.3), (0, 80.0, 1.8), (0, 700.0, 1.2), (1, 2500.0, 0.05)],
    [(-1, 400.0, 0.05), (0, 1500.0, 2.5), (0, 2500.0, 1.2), (1, 3000.0, 0.03)],
];

/// The Cabinet Simulator: the sound driven a little, as an amp
/// would, and shaped as one of four speaker cabinets.
#[derive(Default)]
struct Cabinet {
    bands: [Biquad; 4],
}

impl Dsp for Cabinet {
    fn reset(&mut self) {
        self.bands.iter_mut().for_each(Biquad::clear);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let cabinet = &CABINET_BANDS[(p[0].round() as usize).min(CABINET_BANDS.len() - 1)];
        for (b, &(kind, freq, gain)) in self.bands.iter_mut().zip(cabinet) {
            b.design(kind, freq, gain, ctx.sr);
        }
        let (drive, mix) = (p[1], p[2]);
        let norm = 1.0 / drive.tanh();
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let y = through(&mut self.bands, ch, (i[ch] * drive).tanh() * norm);
                o[ch] = i[ch] * (1.0 - mix) + y * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- vibrato

/// The most the vibrato's delay swings, in seconds: about a quarter tone
/// at 5 Hz.
const VIBRATO_SWING: f32 = 0.004;

/// The Vibrato: the sound read back through a delay that swings
/// longer and shorter, which bends its pitch up and down. Stereo puts the
/// right channel's swing up to half a cycle behind.
struct Vibrato {
    line: DelayLine,
    phase: f32,
}

impl Vibrato {
    fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 2.0 * VIBRATO_SWING), phase: 0.0 }
    }
}

impl Dsp for Vibrato {
    fn reset(&mut self) {
        self.line.clear();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (inc, swing, stereo, mix) = (p[0] / ctx.sr, p[1] * VIBRATO_SWING * ctx.sr, p[2], p[3]);
        for (o, i) in out.iter_mut().zip(input) {
            self.line.push(*i);
            for ch in 0..2 {
                let ph = self.phase + ch as f32 * stereo;
                let wet = self.line.tap_ch(ch, 1.0 + swing * (1.0 + sine(ph)));
                o[ch] = i[ch] * (1.0 - mix) + wet * mix;
            }
            self.phase = fract(self.phase + inc);
        }
    }
}

// ---------------------------------------------------------------- input

/// Frames of input an Input keeps on hand, and lets wait at most.
const INPUT_HELD: usize = 2048;

/// The Input: the sound card's input, played into the graph. The UI
/// opens the input while the song has one and it arrives on a tape; at a
/// rate other than the output's it is resampled.
pub struct LiveInput {
    tape: Arc<Tape>,
    held: Vec<Frame>,
    /// Where in `held` the next output frame reads, between frames.
    pos: f64,
}

impl LiveInput {
    pub fn new(tape: Arc<Tape>) -> Self {
        Self { tape, held: Vec::with_capacity(INPUT_HELD), pos: 0.0 }
    }
}

impl Dsp for LiveInput {
    fn reset(&mut self) {
        self.held.clear();
        self.pos = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        let rate = self.tape.rate();
        let step = if rate == 0 { 1.0 } else { rate as f64 / ctx.sr as f64 };
        // Top up what is held from the tape, without growing past its room.
        let have = self.held.len();
        self.held.resize(INPUT_HELD, [0.0; 2]);
        let got = self.tape.read(&mut self.held[have..], INPUT_HELD / 2);
        self.held.truncate(have + got);
        let (vol, channels) = (p[0], p[1].round() as u32);
        for o in out.iter_mut() {
            let i = self.pos as usize;
            // Waiting for input plays silence rather than old sound.
            if i + 1 >= self.held.len() {
                *o = [0.0; 2];
                continue;
            }
            let (a, b, t) = (self.held[i], self.held[i + 1], self.pos.fract() as f32);
            let x = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let x = match channels {
                0 => x,
                1 => [x[0]; 2],
                2 => [x[1]; 2],
                _ => [(x[0] + x[1]) * 0.5; 2],
            };
            *o = [x[0] * vol, x[1] * vol];
            self.pos += step;
        }
        let used = (self.pos as usize).min(self.held.len());
        self.held.drain(..used);
        self.pos -= used as f64;
    }
}

// ---------------------------------------------------------------- waveshaper

/// Points on the WaveShaper's curve, evenly from -1 to 1.
const SHAPE_POINTS: usize = 9;

/// The WaveShaper: the sound bent through a curve, drawn here as nine
/// points from -1 to 1 with straight lines between. Symmetric mirrors the
/// right half onto the left, so only the points from 0 up count. A level
/// past ±1 follows the end of the curve.
#[derive(Default)]
struct WaveShaper;

/// `x` through the curve of `points`, evenly spaced from -1 to 1.
fn shape(points: &[f32], symmetric: bool, x: f32) -> f32 {
    if symmetric && x < 0.0 {
        return -shape(points, true, -x);
    }
    let at = (x.clamp(-1.0, 1.0) + 1.0) * 0.5 * (points.len() - 1) as f32;
    let i = (at as usize).min(points.len() - 2);
    let t = at - i as f32;
    points[i] + (points[i + 1] - points[i]) * t
}

impl Dsp for WaveShaper {
    fn process(&mut self, _: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (gain, symmetric, level, mix) = (p[0], p[1] >= 0.5, p[2], p[3]);
        let points = &p[4..4 + SHAPE_POINTS];
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let wet = shape(points, symmetric, i[ch] * gain) * level;
                o[ch] = i[ch] * (1.0 - mix) + wet * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- filter pro

/// The Filter Pro: eight filter types, each one to four biquads deep
/// for a slope of 12 to 48 dB an octave, with resonance on the first, and
/// gain for the peak and shelves.
#[derive(Default)]
struct FilterPro {
    stages: [Biquad; 4],
}

impl Dsp for FilterPro {
    fn reset(&mut self) {
        self.stages.iter_mut().for_each(Biquad::clear);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let shape = Shape::ALL[(p[0].round() as usize).min(Shape::ALL.len() - 1)];
        let stages = (p[4].round() as usize + 1).clamp(1, self.stages.len());
        // The gain is shared between the stages, so the whole adds up to it.
        let gain = p[3].powf(1.0 / stages as f32);
        // Only the first stage resonates, so stacking them steepens the
        // slope without multiplying the peak; a shelf past a Q of one
        // overshoots wildly, so it stops there.
        let shelf = matches!(shape, Shape::LowShelf | Shape::HighShelf);
        let q = if shelf { p[2].min(1.0) } else { p[2] };
        for (k, b) in self.stages[..stages].iter_mut().enumerate() {
            let q = if k == 0 { q } else { std::f32::consts::FRAC_1_SQRT_2.min(q) };
            b.rbj(shape, p[1], q, gain, ctx.sr);
        }
        let mix = p[5];
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let y = through(&mut self.stages[..stages], ch, i[ch]);
                o[ch] = i[ch] * (1.0 - mix) + y * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- chorus

/// How far the chorus's voices sweep at full depth, in seconds.
const CHORUS_SWEEP: f32 = 0.008;

/// The Chorus: up to four copies of the sound, each read through a
/// delay that sweeps at the same rate but its own point in the cycle, so
/// they drift around each other; the right channel's cycle runs Stereo
/// behind the left's.
struct Chorus {
    line: DelayLine,
    phase: f32,
}

impl Chorus {
    fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 0.04 + 2.0 * CHORUS_SWEEP), phase: 0.0 }
    }
}

impl Dsp for Chorus {
    fn reset(&mut self) {
        self.line.clear();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let voices = p[0].round().clamp(1.0, 4.0) as usize;
        let (inc, sweep, base) = (p[1] / ctx.sr, p[2] * CHORUS_SWEEP * ctx.sr, p[3] * ctx.sr);
        let (stereo, fb, mix) = (p[4], p[5], p[6]);
        let n = voices as f32;
        for (o, i) in out.iter_mut().zip(input) {
            let mut sum = [0.0; 2];
            for (ch, w) in sum.iter_mut().enumerate() {
                for v in 0..voices {
                    let ph = self.phase + v as f32 / n + ch as f32 * stereo;
                    *w += self.line.tap_ch(ch, base + sweep * (1.0 + sine(ph)));
                }
            }
            // The voices' average goes round again, so the loop stays under
            // the feedback however many there are; heard, they add up as
            // uncorrelated sounds do.
            let fb = fb / n;
            self.line.push([i[0] + sum[0] * fb, i[1] + sum[1] * fb]);
            let wet = sum.map(|x| x / n.sqrt());
            self.phase = fract(self.phase + inc);
            for ch in 0..2 {
                o[ch] = i[ch] * (1.0 - mix) + wet[ch] * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- echo

/// The Echo: a delay timed in seconds rather than lines, darker each
/// time round with Damping, the right channel's echoes up to half again as
/// late with Stereo.
struct Echo {
    line: DelayLine,
    lp: Frame,
}

impl Echo {
    fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 3.0), lp: [0.0; 2] }
    }
}

impl Dsp for Echo {
    fn reset(&mut self) {
        self.line.clear();
        self.lp = [0.0; 2];
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (fb, damp, mix) = (p[1], p[2] * 0.9, p[4]);
        let times = [p[0] * ctx.sr, p[0] * ctx.sr * (1.0 + 0.5 * p[3])];
        for (o, i) in out.iter_mut().zip(input) {
            let mut back = [0.0; 2];
            for ch in 0..2 {
                let echo = self.line.tap_ch(ch, times[ch]);
                self.lp[ch] = echo * (1.0 - damp) + self.lp[ch] * damp;
                back[ch] = i[ch] + self.lp[ch] * fb;
                o[ch] = i[ch] + echo * mix;
            }
            self.line.push(back);
        }
    }
}

// ---------------------------------------------------------------- analog filter

/// The Analog Filter, a Moog-style ladder: four one-pole stages in
/// a row with the last fed back against the input, which resonates and at
/// full resonance sings by itself; the input is driven into a soft clip,
/// as the transistors would. The types are mixes of the stages'
/// outputs, after the Oberheim Xpander.
#[derive(Default)]
struct AnalogFilter {
    stages: [[f32; 4]; 2],
}

impl Dsp for AnalogFilter {
    fn reset(&mut self) {
        self.stages = [[0.0; 4]; 2];
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let kind = p[0].round() as u32;
        // Zero-delay feedback, after Zavalishin: each stage is a
        // trapezoidal one-pole, and the loop is solved for this frame, so
        // full resonance (4) rings at any cutoff.
        let g = (PI * p[1].min(ctx.sr * 0.45) / ctx.sr).tan();
        let big = g / (1.0 + g);
        let k = 4.0 * p[2].clamp(0.0, 1.0);
        let (drive, mix) = (p[3], p[4]);
        // The feedback takes the level down; most of it is made up.
        let makeup = 1.0 + 0.5 * k;
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let s = &mut self.stages[ch];
                let past = (big.powi(3) * s[0] + big * big * s[1] + big * s[2] + s[3]) / (1.0 + g);
                let u = ((i[ch] * drive - k * past) / (1.0 + k * big.powi(4))).tanh();
                let mut y = [0.0; 4];
                let mut x = u;
                for (st, yk) in s.iter_mut().zip(&mut y) {
                    let v = (x - *st) * big;
                    *yk = v + *st;
                    *st = *yk + v;
                    x = *yk;
                }
                let wet = match kind {
                    0 => y[3],
                    1 => y[1],
                    2 => 2.0 * (y[0] - y[1]),
                    _ => u - 4.0 * y[0] + 6.0 * y[1] - 4.0 * y[2] + y[3],
                } * makeup;
                o[ch] = i[ch] * (1.0 - mix) + wet * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- plate reverb

/// A ring of one channel's last samples.
struct Ring {
    buf: Vec<f32>,
    pos: usize,
}

impl Ring {
    fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(2)], pos: 0 }
    }

    /// The sample written `d` samples ago (1 is the last), between them.
    fn at(&self, d: f32) -> f32 {
        let len = self.buf.len();
        let d = d.clamp(1.0, (len - 2) as f32);
        let i = d as usize;
        let wrap = |i: usize| if i >= len { i - len } else { i };
        let (a, b) = (self.buf[wrap(self.pos + len - i)], self.buf[wrap(self.pos + len - i - 1)]);
        a + (b - a) * fract(d)
    }

    fn push(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) % self.buf.len();
    }
}

/// An allpass diffuser over a ring of `len` samples with gain `g`, read
/// `delay` back (`len` less a sweep, if it is modulated).
fn diffuse(ring: &mut Ring, delay: f32, g: f32, x: f32) -> f32 {
    let back = ring.at(delay);
    let v = x - g * back;
    ring.push(v);
    back + g * v
}

/// The plate's delays at the 29761 Hz they were tuned at.
const PLATE_RATE: f32 = 29761.0;
const PLATE_DIFFUSERS: [(f32, f32); 4] = [(142.0, 0.75), (107.0, 0.75), (379.0, 0.625), (277.0, 0.625)];
/// Each half of the tank: its swept allpass, delay, allpass and delay.
const PLATE_TANK: [[f32; 4]; 2] = [[672.0, 4453.0, 1800.0, 3720.0], [908.0, 4217.0, 2656.0, 3163.0]];
/// How far the swept allpasses move, in samples at the tuning rate.
const PLATE_SWEEP: f32 = 16.0;

/// A plate reverb after Dattorro's: the sound diffused by four allpasses,
/// then round a figure-eight tank of two halves, each a swept allpass, a
/// delay, damping, an allpass and a delay, feeding the other; each side
/// listens at seven points of the tank.
struct PlateReverb {
    scale: f32,
    predelay: Ring,
    input_lp: f32,
    diffusers: Vec<Ring>,
    /// The two halves of the tank: swept allpass, delay, allpass, delay.
    tank: [[Ring; 4]; 2],
    damp: [f32; 2],
    phase: f32,
}

impl PlateReverb {
    fn new(sr: f32) -> Self {
        let scale = sr / PLATE_RATE;
        let ring = |n: f32| Ring::new((n * scale) as usize + (PLATE_SWEEP * scale) as usize + 4);
        Self {
            scale,
            predelay: Ring::new((sr * 0.21) as usize),
            input_lp: 0.0,
            diffusers: PLATE_DIFFUSERS.iter().map(|&(n, _)| ring(n)).collect(),
            tank: PLATE_TANK.map(|half| half.map(ring)),
            damp: [0.0; 2],
            phase: 0.0,
        }
    }
}

impl Dsp for PlateReverb {
    fn reset(&mut self) {
        for ring in std::iter::once(&mut self.predelay).chain(&mut self.diffusers).chain(self.tank.iter_mut().flatten())
        {
            ring.buf.fill(0.0);
        }
        self.input_lp = 0.0;
        self.damp = [0.0; 2];
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (decay, pre, damping, width, mix) = (p[0], p[1] * ctx.sr, p[2] * 0.9, p[3], p[4]);
        let k = self.scale;
        let diffusion2 = (decay + 0.15).clamp(0.25, 0.5);
        let inc = 1.0 / ctx.sr;
        // The seven points each side listens at: (half, ring, delay, sign).
        const TAPS: [[(usize, usize, f32, f32); 7]; 2] = [
            [
                (1, 1, 266.0, 1.0),
                (1, 1, 2974.0, 1.0),
                (1, 2, 1913.0, -1.0),
                (1, 3, 1996.0, 1.0),
                (0, 1, 1990.0, -1.0),
                (0, 2, 187.0, -1.0),
                (0, 3, 1066.0, -1.0),
            ],
            [
                (0, 1, 353.0, 1.0),
                (0, 1, 3627.0, 1.0),
                (0, 2, 1228.0, -1.0),
                (0, 3, 2673.0, 1.0),
                (1, 1, 2111.0, -1.0),
                (1, 2, 335.0, -1.0),
                (1, 3, 121.0, -1.0),
            ],
        ];
        for (o, i) in out.iter_mut().zip(input) {
            self.predelay.push((i[0] + i[1]) * 0.5);
            let mut x = self.predelay.at(pre.max(1.0));
            self.input_lp += 0.7 * (x - self.input_lp);
            x = self.input_lp;
            for (ring, &(n, g)) in self.diffusers.iter_mut().zip(&PLATE_DIFFUSERS) {
                x = diffuse(ring, n * k, g, x);
            }
            // Each half takes the diffused sound and the other half's end.
            let end = |h: usize| self.tank[h][3].at(PLATE_TANK[h][3] * k);
            let ends = [end(1), end(0)];
            for half in 0..2 {
                let sweep = PLATE_SWEEP * k * sine(self.phase + half as f32 * 0.25);
                let [n0, n1, n2, _] = PLATE_TANK[half];
                let rings = &mut self.tank[half];
                let mut y = x + decay * ends[half];
                y = diffuse(&mut rings[0], n0 * k + sweep, -0.7, y);
                rings[1].push(y);
                let y = rings[1].at(n1 * k);
                self.damp[half] += (1.0 - damping) * (y - self.damp[half]);
                let y = diffuse(&mut rings[2], n2 * k, diffusion2, self.damp[half] * decay);
                rings[3].push(y);
            }
            self.phase = fract(self.phase + inc);
            let side = |taps: &[(usize, usize, f32, f32); 7], tank: &[[Ring; 4]; 2]| {
                0.6 * taps.iter().map(|&(h, r, d, s)| s * tank[h][r].at(d * k)).sum::<f32>()
            };
            let (l, r) = (side(&TAPS[0], &self.tank), side(&TAPS[1], &self.tank));
            let (mid, wide) = ((l + r) * 0.5, (l - r) * 0.5 * width);
            o[0] = i[0] + (mid + wide) * mix;
            o[1] = i[1] + (mid - wide) * mix;
        }
    }
}

// ---------------------------------------------------------------- EQ 5

/// The EQ 5: a parametric EQ of a low shelf, three peaks and a high
/// shelf, each with its frequency, gain and width.
#[derive(Default)]
struct Eq5 {
    bands: [Biquad; 5],
}

impl Dsp for Eq5 {
    fn reset(&mut self) {
        self.bands.iter_mut().for_each(Biquad::clear);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        for (k, b) in self.bands.iter_mut().enumerate() {
            let shape = match k {
                0 => Shape::LowShelf,
                4 => Shape::HighShelf,
                _ => Shape::Peak,
            };
            let band = &p[k * 3..k * 3 + 3];
            // A shelf past a Q of one overshoots; see the Filter Pro.
            let q = if shape == Shape::Peak { band[2] } else { band[2].min(1.0) };
            b.rbj(shape, band[0], q, band[1], ctx.sr);
        }
        run_bands(&mut self.bands, input, out);
    }
}

// ---------------------------------------------------------------- compressor

/// A feed-forward compressor following the louder channel.
#[derive(Default)]
struct Compressor {
    env: f32,
}

impl Dsp for Compressor {
    fn reset(&mut self) {
        self.env = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (threshold, ratio, makeup, mix) = (p[0], p[1], p[4], p[5]);
        let coef = |t: f32| 1.0 - (-1.0 / (t.max(1e-5) * ctx.sr)).exp();
        let (att, rel) = (coef(p[2]), coef(p[3]));
        for (o, i) in out.iter_mut().zip(input) {
            let level = i[0].abs().max(i[1].abs());
            let c = if level > self.env { att } else { rel };
            self.env += c * (level - self.env);
            let gain = if self.env > threshold {
                // Above the threshold the level grows 1/ratio as fast.
                (threshold / self.env).powf(1.0 - 1.0 / ratio)
            } else {
                1.0
            } * makeup;
            for ch in 0..2 {
                o[ch] = i[ch] * (1.0 - mix) + i[ch] * gain * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- EQ

/// A biquad in transposed direct form II.
#[derive(Default, Clone, Copy)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [[f32; 2]; 2],
}

/// The shapes a `Biquad` takes, after the RBJ cookbook.
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Lowpass,
    Highpass,
    Bandpass,
    Notch,
    Allpass,
    Peak,
    LowShelf,
    HighShelf,
}

impl Shape {
    /// In the order of `FILTER_PRO_TYPES`.
    const ALL: [Shape; 8] = [
        Shape::Lowpass,
        Shape::Highpass,
        Shape::Bandpass,
        Shape::Notch,
        Shape::Allpass,
        Shape::Peak,
        Shape::LowShelf,
        Shape::HighShelf,
    ];
}

impl Biquad {
    /// The EQs' shelves (`kind` -1 low, 1 high) and peak (0).
    fn design(&mut self, kind: i32, freq: f32, gain: f32, sr: f32) {
        let (shape, q) = match kind {
            0 => (Shape::Peak, 0.9),
            k if k < 0 => (Shape::LowShelf, std::f32::consts::FRAC_1_SQRT_2),
            _ => (Shape::HighShelf, std::f32::consts::FRAC_1_SQRT_2),
        };
        self.rbj(shape, freq, q, gain, sr);
    }

    /// `shape` at `freq` with resonance `q`; `gain` is for the peak and
    /// shelves.
    fn rbj(&mut self, shape: Shape, freq: f32, q: f32, gain: f32, sr: f32) {
        let a = gain.max(1e-4).sqrt();
        let w = TAU * freq.clamp(10.0, sr * 0.45) / sr;
        let (sn, cs) = w.sin_cos();
        let alpha = sn / (2.0 * q.max(0.05));
        let (b, a2) = match shape {
            Shape::Lowpass => ([(1.0 - cs) / 2.0, 1.0 - cs, (1.0 - cs) / 2.0], [1.0 + alpha, -2.0 * cs, 1.0 - alpha]),
            Shape::Highpass => {
                ([(1.0 + cs) / 2.0, -(1.0 + cs), (1.0 + cs) / 2.0], [1.0 + alpha, -2.0 * cs, 1.0 - alpha])
            }
            Shape::Bandpass => ([alpha, 0.0, -alpha], [1.0 + alpha, -2.0 * cs, 1.0 - alpha]),
            Shape::Notch => ([1.0, -2.0 * cs, 1.0], [1.0 + alpha, -2.0 * cs, 1.0 - alpha]),
            Shape::Allpass => ([1.0 - alpha, -2.0 * cs, 1.0 + alpha], [1.0 + alpha, -2.0 * cs, 1.0 - alpha]),
            Shape::Peak => {
                ([1.0 + alpha * a, -2.0 * cs, 1.0 - alpha * a], [1.0 + alpha / a, -2.0 * cs, 1.0 - alpha / a])
            }
            Shape::LowShelf | Shape::HighShelf => {
                let s = 2.0 * a.sqrt() * alpha;
                let k = if shape == Shape::LowShelf { 1.0 } else { -1.0 };
                (
                    [
                        a * ((a + 1.0) - k * (a - 1.0) * cs + s),
                        2.0 * k * a * ((a - 1.0) - k * (a + 1.0) * cs),
                        a * ((a + 1.0) - k * (a - 1.0) * cs - s),
                    ],
                    [
                        (a + 1.0) + k * (a - 1.0) * cs + s,
                        -2.0 * k * ((a - 1.0) + k * (a + 1.0) * cs),
                        (a + 1.0) + k * (a - 1.0) * cs - s,
                    ],
                )
            }
        };
        self.b = [b[0] / a2[0], b[1] / a2[0], b[2] / a2[0]];
        self.a = [a2[1] / a2[0], a2[2] / a2[0]];
    }

    fn clear(&mut self) {
        self.z = [[0.0; 2]; 2];
    }

    fn tick(&mut self, ch: usize, x: f32) -> f32 {
        let z = &mut self.z[ch];
        let y = self.b[0] * x + z[0];
        z[0] = self.b[1] * x - self.a[0] * y + z[1];
        z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// A three-band EQ: low shelf, mid peak and high shelf.
#[derive(Default)]
struct Eq {
    bands: [Biquad; 3],
}

/// Channel `ch`'s `x` through each of `bands` in turn.
fn through(bands: &mut [Biquad], ch: usize, x: f32) -> f32 {
    bands.iter_mut().fold(x, |x, b| b.tick(ch, x))
}

/// Runs `input` through each of `bands` in turn, into `out`.
fn run_bands(bands: &mut [Biquad], input: &[Frame], out: &mut [Frame]) {
    for (o, i) in out.iter_mut().zip(input) {
        for ch in 0..2 {
            o[ch] = through(bands, ch, i[ch]);
        }
    }
}

impl Dsp for Eq {
    fn reset(&mut self) {
        self.bands.iter_mut().for_each(Biquad::clear);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        self.bands[0].design(-1, p[3], p[0], ctx.sr);
        self.bands[1].design(0, p[4], p[1], ctx.sr);
        self.bands[2].design(1, p[5], p[2], ctx.sr);
        run_bands(&mut self.bands, input, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 1000.0;

    /// A slot whose audio is a ramp from 0 to 1, so the output level tells
    /// where in the sample playback is.
    fn ramp_slot(len: usize) -> SampleSlot {
        let frames = (0..len).map(|i| [i as f32 / len as f32; 2]).collect();
        let sample = Sample { name: "ramp".into(), sample_rate: SR, channels: 1, frames };
        let mut slot = SampleSlot::new(sample, None);
        slot.base_note = 60;
        slot
    }

    fn render(s: &mut Sampler, frames: usize) -> Vec<f32> {
        let ctx = Ctx { sr: SR, samples_per_line: 100.0, song_line: None };
        // Volume, pan, transpose, attack, decay, sustain, release.
        let params = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let mut out = vec![[0.0; 2]; frames];
        s.process(&ctx, &params, &[], &mut out);
        // Undo the center pan gain.
        out.iter().map(|f| f[0] / pan_gains(0.0).0).collect()
    }

    #[test]
    fn numbers_too_small_to_matter_are_zero() {
        flush_denormals();
        let tiny = std::hint::black_box(f32::MIN_POSITIVE);
        assert_eq!(std::hint::black_box(tiny * 0.5), 0.0);
    }

    #[test]
    fn an_lfo_synced_to_lines_follows_the_song() {
        // Tremolo, a sine over 4 lines at full depth: silent a quarter of the
        // way round the other way, three lines into the song.
        let mut lfo = Lfo::default();
        let p = [0.0, 0.0, 1.0, 2.0, 1.0, 4.0];
        let input = [[1.0; 2]; 4];
        let mut out = [[0.0; 2]; 4];
        for (line, gain) in [(1.0, 1.0), (3.0, 0.0)] {
            let ctx = Ctx { sr: 1000.0, samples_per_line: 100.0, song_line: Some(line) };
            lfo.process(&ctx, &p, &input, &mut out);
            assert!((out[0][0] - gain).abs() < 1e-3, "at line {line}: {}", out[0][0]);
        }
    }

    #[test]
    fn keyzones_pick_samples() {
        let mut s = Sampler::new();
        let mut a = ramp_slot(100);
        let mut b = ramp_slot(100);
        a.keys = [48, 48];
        b.keys = [49, 49];
        b.volume = 0.5;
        s.set_samples(&[a, b]);
        s.note_on(0, 49.0, 1.0);
        assert_eq!(s.voices.iter().filter(|v| v.env.active()).count(), 1);
        assert_eq!(s.voices.iter().find(|v| v.env.active()).unwrap().zone, 1);
        s.note_on(1, 60.0, 1.0);
        assert_eq!(s.voices.iter().filter(|v| v.env.active()).count(), 1, "no zone holds C-5");
    }

    #[test]
    fn overlapping_zones_layer() {
        let mut s = Sampler::new();
        s.set_samples(&[ramp_slot(100), ramp_slot(100)]);
        s.note_on(0, 60.0, 1.0);
        assert_eq!(s.voices.iter().filter(|v| v.env.active()).count(), 2);
    }

    #[test]
    fn velocity_ranges() {
        let mut s = Sampler::new();
        let mut soft = ramp_slot(100);
        soft.velocities = [0, 63];
        s.set_samples(&[soft]);
        s.note_on(0, 60.0, 1.0);
        assert!(s.voices.iter().all(|v| !v.env.active()));
        s.note_on(0, 60.0, 0.25);
        assert!(s.voices.iter().any(|v| v.env.active()));
    }

    #[test]
    fn no_loop_stops_at_the_end() {
        let mut s = Sampler::new();
        s.set_samples(&[ramp_slot(100)]);
        s.note_on(0, 60.0, 1.0);
        let out = render(&mut s, 200);
        assert!(out[99] > 0.9);
        assert!(out[150..].iter().all(|&x| x == 0.0));
    }

    #[test]
    fn forward_loop_repeats() {
        let mut s = Sampler::new();
        let mut slot = ramp_slot(100);
        slot.loop_mode = 1;
        slot.loop_start = 50;
        slot.loop_end = 100;
        s.set_samples(&[slot]);
        s.note_on(0, 60.0, 1.0);
        let out = render(&mut s, 300);
        // After the first pass the ramp restarts from the loop start.
        assert!((out[100] - 0.5).abs() < 0.02, "{}", out[100]);
        assert!((out[249] - 0.99).abs() < 0.02, "{}", out[249]);
        assert!((out[250] - 0.5).abs() < 0.02, "{}", out[250]);
    }

    #[test]
    fn backward_loop_plays_the_loop_in_reverse() {
        let mut s = Sampler::new();
        let mut slot = ramp_slot(100);
        slot.loop_mode = 2;
        slot.loop_start = 50;
        slot.loop_end = 100;
        s.set_samples(&[slot]);
        s.note_on(0, 60.0, 1.0);
        let out = render(&mut s, 400);
        // Forward to the loop end, then falling repeatedly from the end.
        assert!(out[120] < out[110] && out[110] < out[101]);
        assert!(out[200..400].iter().all(|&x| x >= 0.48));
        let rising = out[200..400].windows(2).filter(|w| w[1] > w[0] + 0.1).count();
        assert!(rising >= 3, "jumps back to the loop end: {rising}");
    }

    #[test]
    fn ping_pong_loop_bounces() {
        let mut s = Sampler::new();
        let mut slot = ramp_slot(100);
        slot.loop_mode = 3;
        slot.loop_start = 50;
        slot.loop_end = 100;
        s.set_samples(&[slot]);
        s.note_on(0, 60.0, 1.0);
        let out = render(&mut s, 400);
        assert!(out[120] < out[110], "falling after the loop end");
        assert!(out[170] > out[160], "rising again after the loop start");
        assert!(out.iter().skip(50).all(|&x| x >= 0.48));
    }

    #[test]
    fn pitch_follows_base_note_and_transpose() {
        let mut s = Sampler::new();
        let mut slot = ramp_slot(200);
        slot.transpose = -12;
        s.set_samples(&[slot]);
        // C-6 with a -12 transpose plays at the original speed.
        s.note_on(0, 72.0, 1.0);
        let out = render(&mut s, 100);
        assert!((out[50] - 0.25).abs() < 0.02, "{}", out[50]);
    }

    #[test]
    fn replacing_the_audio_stops_its_voices() {
        let mut s = Sampler::new();
        let slot = ramp_slot(100);
        s.set_samples(std::slice::from_ref(&slot));
        s.note_on(0, 60.0, 1.0);
        // A settings change keeps the voice.
        let mut louder = slot.clone();
        louder.volume = 2.0;
        s.set_samples(&[louder]);
        assert!(s.voices.iter().any(|v| v.env.active()));
        // New audio stops it.
        s.set_samples(&[ramp_slot(100)]);
        assert!(s.voices.iter().all(|v| !v.env.active()));
    }

    #[test]
    fn mute_groups_cut_each_other() {
        let mut s = Sampler::new();
        let (mut open, mut closed, other) = (ramp_slot(1000), ramp_slot(1000), ramp_slot(1000));
        (open.keys, closed.keys) = ([48, 48], [49, 49]);
        (open.mute_group, closed.mute_group) = (1, 1);
        s.set_samples(&[open, closed, other]);
        s.note_on(0, 48.0, 1.0);
        s.note_on(1, 60.0, 1.0);
        render(&mut s, 10);
        s.note_on(2, 49.0, 1.0);
        render(&mut s, 10);
        let zones: Vec<usize> = s.voices.iter().filter(|v| v.env.active()).map(|v| v.zone).collect();
        assert!(!zones.contains(&0), "the open hat is cut: {zones:?}");
        assert!(zones.contains(&1) && zones.contains(&2), "others play on: {zones:?}");
    }

    #[test]
    fn oneshots_ignore_note_offs() {
        let mut s = Sampler::new();
        let mut slot = ramp_slot(100);
        slot.oneshot = true;
        s.set_samples(&[slot]);
        s.note_on(0, 60.0, 1.0);
        s.note_off(0);
        assert!(s.voices.iter().any(|v| v.env.active() && !v.slot.released));
    }

    #[test]
    fn beat_sync_fits_the_sample_to_lines() {
        let mut s = Sampler::new();
        let mut slot = ramp_slot(100);
        // Two lines of 100 frames: half speed, whatever the sample rate.
        slot.beat_sync = 2;
        s.set_samples(&[slot]);
        s.note_on(0, 60.0, 1.0);
        let out = render(&mut s, 150);
        assert!((out[100] - 0.5).abs() < 0.02, "{}", out[100]);
    }

    #[test]
    fn the_sine_table_is_a_sine() {
        for i in -2000..2000 {
            let x = i as f32 * 0.00377;
            assert!((sine(x) as f64 - (x as f64 * std::f64::consts::TAU).sin()).abs() < 2e-6, "{x}");
        }
    }

    #[test]
    fn reverse_plays_from_the_end_or_from_where_it_is() {
        let mut s = Sampler::new();
        s.set_samples(&[ramp_slot(100)]);
        s.note_on(0, 60.0, 1.0);
        s.reverse(0, true);
        let out = render(&mut s, 10);
        assert!((out[0] - 0.99).abs() < 0.02 && out[9] < out[0], "{out:?}");
        // Forwards again from there.
        s.reverse(0, false);
        let out = render(&mut s, 10);
        assert!(out[9] > out[0], "{out:?}");
        // Backwards past the start, it stops.
        s.reverse(0, true);
        let out = render(&mut s, 200);
        assert_eq!(out[199], 0.0);
    }

    #[test]
    fn a_slice_plays_at_the_pitch_of_the_note() {
        let mut s = Sampler::new();
        let mut slot = ramp_slot(100);
        slot.slices = vec![25, 50, 75];
        s.set_samples(&[slot]);
        // The whole sample on its base note, an octave up, switched to the
        // third slice.
        s.note_on(0, 72.0, 1.0);
        s.play_slice(0, 2);
        let out = render(&mut s, 10);
        assert!((out[0] - 0.5).abs() < 0.02, "{out:?}");
        assert!((out[1] - out[0] - 0.02).abs() < 0.002, "still an octave up: {out:?}");
        // On the sample's base note it plays at the sample's pitch.
        s.note_on(1, 60.0, 1.0);
        s.play_slice(1, 1);
        s.note_off(0);
        let low = render(&mut s, 3);
        assert!((low[0] - 0.25).abs() < 0.02 && (low[2] - low[1] - 0.01).abs() < 0.002, "{low:?}");
        // A slice that isn't there leaves the note alone.
        s.note_on(2, 60.0, 1.0);
        s.play_slice(2, 9);
        assert!(s.voices.iter().any(|v| v.slot.key == 2 && v.zone == 0));
    }

    #[test]
    fn sample_offset_starts_later() {
        let mut s = Sampler::new();
        s.set_samples(&[ramp_slot(100)]);
        s.note_on(0, 60.0, 1.0);
        s.sample_offset(0, 0.5);
        let out = render(&mut s, 10);
        assert!((out[0] - 0.5).abs() < 0.02, "{}", out[0]);
    }

    #[test]
    fn seeking_moves_through_the_loop_or_ends_the_sample() {
        let mut s = Sampler::new();
        let mut slot = ramp_slot(100);
        slot.autoseek = true;
        s.set_samples(&[slot.clone()]);
        s.note_on(0, 60.0, 1.0);
        s.seek(0, 40.0);
        assert!((render(&mut s, 1)[0] - 0.4).abs() < 0.02, "40 frames in");
        let mut s = Sampler::new();
        s.set_samples(&[slot.clone()]);
        s.note_on(0, 60.0, 1.0);
        s.seek(0, 150.0);
        assert!(render(&mut s, 4).iter().all(|&x| x == 0.0), "played out by then");
        // Looped over its second half, it is still going round.
        (slot.loop_mode, slot.loop_start, slot.loop_end) = (1, 50, 100);
        let mut s = Sampler::new();
        s.set_samples(&[slot.clone()]);
        s.note_on(0, 60.0, 1.0);
        s.seek(0, 130.0);
        assert!((render(&mut s, 1)[0] - 0.8).abs() < 0.02, "30 frames past the loop's start");
        // Without autoseek, the note stays silent.
        slot.autoseek = false;
        let mut s = Sampler::new();
        s.set_samples(&[slot]);
        s.note_on(0, 60.0, 1.0);
        s.seek(0, 10.0);
        assert!(render(&mut s, 4).iter().all(|&x| x == 0.0));
    }

    /// Runs `kind` with its default parameters, changed by `set`, on a sine
    /// of `freq` Hz at 48 kHz, and returns the peak of the second half.
    fn effect_peak(kind: ModuleKind, set: &[(usize, f32)], freq: f32, amp: f32) -> f32 {
        let sr = 48000.0;
        let ctx = Ctx { sr, samples_per_line: 6000.0, song_line: None };
        let mut params: Vec<f32> = kind.params().iter().map(|p| p.default).collect();
        for &(i, v) in set {
            params[i] = v;
        }
        let input: Vec<Frame> = (0..48000).map(|i| [amp * (TAU * freq * i as f32 / sr).sin(); 2]).collect();
        let mut out = vec![[0.0; 2]; input.len()];
        create(kind, sr).process(&ctx, &params, &input, &mut out);
        out[24000..].iter().fold(0.0f32, |m, f| m.max(f[0].abs()))
    }

    #[test]
    fn phaser_cuts_a_notch_where_its_stages_turn_the_phase_over() {
        // Two stages held at 1 kHz shift 1 kHz by half a cycle, so mixed
        // half and half with the input it cancels out; far below it doesn't.
        let still = [(1, 0.0), (2, 1000.0), (3, 1000.0), (4, 2.0), (5, 0.0), (6, 0.5)];
        let notch = effect_peak(ModuleKind::Phaser, &still, 1000.0, 0.5);
        assert!(notch < 0.02, "notch at 1 kHz: {notch}");
        let low = effect_peak(ModuleKind::Phaser, &still, 60.0, 0.5);
        assert!(low > 0.45, "60 Hz passes: {low}");
        let dry = effect_peak(ModuleKind::Phaser, &[(6, 0.0)], 1000.0, 0.5);
        assert!((dry - 0.5).abs() < 1e-4, "no mix, no change: {dry}");
        let swept = effect_peak(ModuleKind::Phaser, &[(5, 0.9)], 1000.0, 0.5);
        assert!(swept.is_finite() && swept < 2.0, "stays stable with feedback: {swept}");
    }

    #[test]
    fn vocal_filter_passes_its_vowels_formants() {
        let unity = [(4, 1.0)];
        // A's first formant is at 650 Hz, I's at 290 Hz and 1870 Hz.
        let a = effect_peak(ModuleKind::VocalFilter, &unity, 650.0, 0.5);
        assert!((a - 0.5).abs() < 0.05, "A passes 650 Hz: {a}");
        let i = effect_peak(ModuleKind::VocalFilter, &[(0, 2.0), (4, 1.0)], 650.0, 0.5);
        assert!(i < a * 0.3, "I doesn't: {i}");
        let high = effect_peak(ModuleKind::VocalFilter, &unity, 8000.0, 0.5);
        assert!(high < 0.05, "nor 8 kHz: {high}");
        // An octave up, A's first formant is at 1300 Hz.
        let up = effect_peak(ModuleKind::VocalFilter, &[(1, 12.0), (4, 1.0)], 1300.0, 0.5);
        assert!((up - 0.5).abs() < 0.05, "shifted: {up}");
    }

    #[test]
    fn repeater_loops_the_input_before_hold() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let mut r = create(ModuleKind::Repeater, ctx.sr);
        // 1/16 line is 375 frames; the input counts up.
        let input: Vec<Frame> = (0..4000).map(|i| [i as f32; 2]).collect();
        let mut out = vec![[0.0; 2]; input.len()];
        r.process(&ctx, &[0.0, 9.0, 1.0], &input, &mut out);
        assert_eq!(out, input, "off, it passes the input");
        let silence = vec![[0.0; 2]; 2000];
        let mut held = vec![[0.0; 2]; 2000];
        r.process(&ctx, &[1.0, 9.0, 1.0], &silence, &mut held);
        // Once faded in, the loop's middle repeats every 375 frames.
        for k in [400 + 100, 400 + 375 + 100, 400 + 750 + 100] {
            assert_eq!(held[k][0], (4000 - 375 + (k % 375)) as f32, "frame {k}");
        }
        let mut after = vec![[0.0; 2]; 2000];
        r.process(&ctx, &[0.0, 9.0, 1.0], &silence, &mut after);
        assert!(after[500..].iter().all(|f| f[0] == 0.0), "let go, it passes the input again");
    }

    #[test]
    fn ring_mod_moves_a_tone_to_the_sum_and_difference() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let mut r = create(ModuleKind::RingMod, ctx.sr);
        let n = 48000;
        let input: Vec<Frame> = (0..n).map(|i| [(TAU * 1000.0 * i as f32 / ctx.sr).sin(); 2]).collect();
        let mut out = vec![[0.0; 2]; n];
        r.process(&ctx, &[300.0, 0.0, 0.0, 1.0], &input, &mut out);
        let level = |f: f32| level_at(&out, f, ctx.sr);
        assert!(
            (level(700.0) - 0.5).abs() < 0.02 && (level(1300.0) - 0.5).abs() < 0.02,
            "{} {}",
            level(700.0),
            level(1300.0)
        );
        assert!(level(1000.0) < 0.01, "the tone itself is gone: {}", level(1000.0));
    }

    #[test]
    fn gate_passes_loud_sounds_and_silences_quiet_ones() {
        let loud = effect_peak(ModuleKind::Gate, &[], 100.0, 0.5);
        assert!((loud - 0.5).abs() < 0.01, "above the threshold it is open: {loud}");
        let quiet = effect_peak(ModuleKind::Gate, &[], 100.0, 0.02);
        assert!(quiet < 1e-4, "below it, closed: {quiet}");
        let floor = effect_peak(ModuleKind::Gate, &[(4, 0.5)], 100.0, 0.02);
        assert!((floor - 0.01).abs() < 1e-3, "closed, it lets the floor through: {floor}");
    }

    #[test]
    fn kicker_falls_to_the_note_and_dies_away() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let mut k = create(ModuleKind::Kicker, ctx.sr);
        let params: Vec<f32> = ModuleKind::Kicker.params().iter().map(|p| p.default).collect();
        k.note_on(0, 33.0, 1.0);
        let mut out = vec![[0.0; 2]; 48000];
        for chunk in out.chunks_mut(MAX_BLOCK) {
            k.process(&ctx, &params, &[[0.0; 2]; MAX_BLOCK][..chunk.len()], chunk);
        }
        // Rising zero crossings in a stretch, to count cycles.
        let cycles =
            |from: usize, to: usize| out[from..to].windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count();
        assert!(cycles(0, 480) >= 3, "it starts high: {} cycles in 10 ms", cycles(0, 480));
        let low = cycles(9600, 14400);
        assert!((5..=6).contains(&low), "then plays A1, 55 Hz: {low} cycles in 100 ms");
        assert!(out[40000..].iter().all(|f| f[0].abs() < 1e-3), "and is gone after the decay");
    }

    /// How much of channel 0 of `out` is at `f`, by correlating with it.
    fn level_at(out: &[Frame], f: f32, sr: f32) -> f32 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (k, o) in out.iter().enumerate() {
            let ph = (TAU * f * k as f32 / sr) as f64;
            re += o[0] as f64 * ph.cos();
            im += o[0] as f64 * ph.sin();
        }
        ((re * re + im * im).sqrt() * 2.0 / out.len() as f64) as f32
    }

    #[test]
    fn spectravoice_stacks_harmonics_at_their_slope() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let play = |set: &[(usize, f32)]| {
            let mut params: Vec<f32> = ModuleKind::SpectraVoice.params().iter().map(|p| p.default).collect();
            params[1] = 4.0;
            params[8] = 1.0;
            for &(i, v) in set {
                params[i] = v;
            }
            let mut s = create(ModuleKind::SpectraVoice, ctx.sr);
            s.note_on(0, 69.0, 1.0);
            let mut out = vec![[0.0; 2]; 24000];
            for chunk in out.chunks_mut(MAX_BLOCK) {
                s.process(&ctx, &params, &[[0.0; 2]; MAX_BLOCK][..chunk.len()], chunk);
            }
            out.split_off(12000)
        };
        let full = play(&[]);
        let (first, second) = (level_at(&full, 440.0, ctx.sr), level_at(&full, 880.0, ctx.sr));
        assert!(first > 0.05 && (second / first - 0.5).abs() < 0.02, "the second harmonic at half: {first} {second}");
        let hollow = play(&[(3, 0.0)]);
        let (first, second) = (level_at(&hollow, 440.0, ctx.sr), level_at(&hollow, 880.0, ctx.sr));
        assert!(second < first * 0.01, "no even harmonics: {first} {second}");
        assert!((level_at(&hollow, 1320.0, ctx.sr) / first - 1.0 / 3.0).abs() < 0.02, "the third at a third");
    }

    #[test]
    fn pitch_shifter_moves_a_tone_up_an_octave() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let mut s = create(ModuleKind::PitchShifter, ctx.sr);
        let params: Vec<f32> = ModuleKind::PitchShifter.params().iter().map(|p| p.default).collect();
        let input: Vec<Frame> = (0..48000).map(|i| [0.5 * (TAU * 500.0 * i as f32 / ctx.sr).sin(); 2]).collect();
        let mut out = vec![[0.0; 2]; input.len()];
        s.process(&ctx, &params, &input, &mut out);
        let out = &out[12000..];
        let (up, at) = (level_at(out, 1000.0, ctx.sr), level_at(out, 500.0, ctx.sr));
        assert!(up > 0.3 && at < 0.05, "an octave up: {up} at 1 kHz, {at} left at 500 Hz");
    }

    #[test]
    fn stereo_expander_scales_the_side_and_keeps_the_middle() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let run = |width: f32, mono_bass: f32, freq: f32| {
            let input: Vec<Frame> = (0..48000)
                .map(|i| {
                    let x = (TAU * freq * i as f32 / ctx.sr).sin();
                    [0.5 + 0.25 * x, 0.5 - 0.25 * x]
                })
                .collect();
            let mut out = vec![[0.0; 2]; input.len()];
            create(ModuleKind::StereoExpander, ctx.sr).process(&ctx, &[width, mono_bass], &input, &mut out);
            out[24000..]
                .iter()
                .fold((0.0f32, 0.0f32), |(m, s), f| (m.max((f[0] + f[1]) * 0.5), s.max((f[0] - f[1]) * 0.5)))
        };
        let (mid, side) = run(1.0, 0.0, 1000.0);
        assert!((mid - 0.5).abs() < 1e-4 && (side - 0.25).abs() < 1e-3, "unchanged at 100%: {mid} {side}");
        let (mid, side) = run(0.0, 0.0, 1000.0);
        assert!((mid - 0.5).abs() < 1e-4 && side < 1e-4, "mono at 0: {mid} {side}");
        let (_, side) = run(2.0, 0.0, 1000.0);
        assert!((side - 0.5).abs() < 1e-3, "twice as wide: {side}");
        let (_, side) = run(2.0, 400.0, 40.0);
        assert!(side < 0.1, "lows kept in the middle: {side}");
    }

    #[test]
    fn comb_filter_rings_at_its_note_and_harmonics() {
        let wet = [(3, 0.0), (4, 1.0)];
        let at = |f: f32| effect_peak(ModuleKind::CombFilter, &wet, f, 0.1);
        let (tooth, harmonic, between) = (at(220.0), at(440.0), at(330.0));
        assert!(tooth > 0.09 && harmonic > 0.09, "A-3 and its octave come through: {tooth} {harmonic}");
        assert!(between < tooth * 0.2, "between them is cut: {between}");
    }

    #[test]
    fn maximizer_boosts_up_to_the_ceiling_and_no_further() {
        let quiet = effect_peak(ModuleKind::Maximizer, &[], 100.0, 0.2);
        assert!((quiet - 0.4).abs() < 1e-3, "quiet sounds get the boost: {quiet}");
        let loud = effect_peak(ModuleKind::Maximizer, &[(0, 4.0), (1, 0.8)], 100.0, 0.5);
        assert!(loud <= 0.8 && loud > 0.75, "loud ones stop at the ceiling: {loud}");
    }

    #[test]
    fn exciter_adds_to_the_highs_and_leaves_the_lows() {
        let low = effect_peak(ModuleKind::Exciter, &[], 100.0, 0.3);
        assert!((low - 0.3).abs() < 0.01, "lows pass: {low}");
        let high = effect_peak(ModuleKind::Exciter, &[], 10000.0, 0.3);
        assert!(high > 0.45, "highs get louder: {high}");
    }

    #[test]
    fn dc_blocker_takes_away_an_offset() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let input: Vec<Frame> = (0..48000).map(|i| [0.5 + 0.3 * (TAU * 440.0 * i as f32 / ctx.sr).sin(); 2]).collect();
        let mut out = vec![[0.0; 2]; input.len()];
        create(ModuleKind::DcBlocker, ctx.sr).process(&ctx, &[10.0], &input, &mut out);
        let tail = &out[24000..];
        let mean = tail.iter().map(|f| f[0]).sum::<f32>() / tail.len() as f32;
        assert!(mean.abs() < 1e-3, "no offset left: {mean}");
        assert!((level_at(tail, 440.0, ctx.sr) - 0.3).abs() < 0.01, "the tone stays");
    }

    #[test]
    fn scream_filter_filters_and_stays_bounded_at_full_resonance() {
        let gentle = [(2, 0.0), (3, 1.0)];
        let low = effect_peak(ModuleKind::ScreamFilter, &gentle, 100.0, 0.3);
        let high = effect_peak(ModuleKind::ScreamFilter, &gentle, 12000.0, 0.3);
        assert!(low > 0.2 && high < low * 0.1, "a lowpass: {low} {high}");
        let wild = effect_peak(ModuleKind::ScreamFilter, &[(2, 1.0), (3, 20.0)], 1500.0, 1.0);
        assert!(wild.is_finite() && wild < 1.5, "screams but stays bounded: {wild}");
    }

    #[test]
    fn multitap_echoes_at_each_taps_time_level_and_pan() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 1000.0, song_line: None };
        let mut params: Vec<f32> = ModuleKind::Multitap.params().iter().map(|p| p.default).collect();
        params[12] = 0.0;
        params[13] = 1.0;
        let mut input = vec![[0.0; 2]; 6000];
        input[0] = [1.0; 2];
        let mut out = vec![[0.0; 2]; input.len()];
        create(ModuleKind::Multitap, ctx.sr).process(&ctx, &params, &input, &mut out);
        for (t, at) in [1000, 2000, 3000, 4000].into_iter().enumerate() {
            let (l, r) = pan_gains(params[t * 3 + 2]);
            let level = params[t * 3 + 1];
            assert!((out[at][0] - level * l).abs() < 1e-5 && (out[at][1] - level * r).abs() < 1e-5, "tap {t}");
        }
        assert!(out[4500..].iter().all(|f| f[0] == 0.0), "and no more without feedback");
    }

    #[test]
    fn eq10_bands_move_their_octave() {
        let flat = effect_peak(ModuleKind::Eq10, &[], 1000.0, 0.25);
        assert!((flat - 0.25).abs() < 0.01, "flat at the defaults: {flat}");
        let boosted = effect_peak(ModuleKind::Eq10, &[(5, 4.0)], 1000.0, 0.25);
        assert!(boosted > 0.8, "the 1 kHz band boosts 1 kHz by 12 dB: {boosted}");
        let far = effect_peak(ModuleKind::Eq10, &[(5, 4.0)], 62.5, 0.25);
        assert!(far < 0.3, "and leaves 63 Hz about alone: {far}");
    }

    #[test]
    fn cabinets_keep_the_middle_and_roll_off_the_ends() {
        for cabinet in 0..CABINET_BANDS.len() as u32 {
            let at = |f: f32| effect_peak(ModuleKind::Cabinet, &[(0, cabinet as f32), (1, 1.0)], f, 0.1);
            let (low, mid, high) = (at(30.0), at(1500.0), at(12000.0));
            assert!(mid > 0.05 && low < mid && high < mid * 0.3, "cabinet {cabinet}: {low} {mid} {high}");
        }
    }

    #[test]
    fn vibrato_bends_a_tone_up_and_down_around_it() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let input: Vec<Frame> = (0..48000).map(|i| [(TAU * 1000.0 * i as f32 / ctx.sr).sin(); 2]).collect();
        let run = |depth: f32| {
            let mut out = vec![[0.0; 2]; input.len()];
            create(ModuleKind::Vibrato, ctx.sr).process(&ctx, &[5.0, depth, 0.0, 1.0], &input, &mut out);
            level_at(&out[4800..], 1000.0, ctx.sr)
        };
        assert!(run(0.0) > 0.95, "without depth the tone stays");
        assert!(run(1.0) < 0.8, "with it, part of the tone moves off 1 kHz: {}", run(1.0));
    }

    #[test]
    fn every_module_stays_finite_at_the_ends_of_its_parameters() {
        let ctx = Ctx { sr: 44100.0, samples_per_line: 5512.0, song_line: None };
        let mut rng = Rng(0x2468_ace1);
        let noise: Vec<Frame> = (0..MAX_BLOCK).map(|_| [rng.next(), rng.next()]).collect();
        for kind in ModuleKind::ADDABLE {
            let specs = kind.params();
            let settings: [Vec<f32>; 3] = [
                specs.iter().map(|p| p.min).collect(),
                specs.iter().map(|p| p.max).collect(),
                specs.iter().map(|p| p.default).collect(),
            ];
            for params in &settings {
                let mut m = create(kind, ctx.sr);
                m.note_on(0, 60.0, 1.0);
                m.note_on(1, 24.0, 1.0);
                let mut peak = 0f32;
                for block in 0..700 {
                    if block == 350 {
                        m.note_off(0);
                        m.note_off(1);
                    }
                    let mut out = [[0.0; 2]; MAX_BLOCK];
                    m.process(&ctx, params, &noise, &mut out);
                    for f in out {
                        assert!(f[0].is_finite() && f[1].is_finite(), "{} at {params:?}", kind.name());
                        peak = peak.max(f[0].abs()).max(f[1].abs());
                    }
                }
                assert!(peak < 50.0, "{} blows up to {peak} at {params:?}", kind.name());
            }
        }
    }

    #[test]
    fn fmx_operators_modulate_by_their_algorithm() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let play = |set: &[(usize, f32)]| {
            let mut params: Vec<f32> = ModuleKind::Fmx.params().iter().map(|p| p.default).collect();
            // Sustained operators, so the tone holds still.
            for k in 0..OPS {
                params[3 + k * 6 + 4] = 1.0;
            }
            for &(i, v) in set {
                params[i] = v;
            }
            let mut s = create(ModuleKind::Fmx, ctx.sr);
            s.note_on(0, 69.0, 1.0);
            let mut out = vec![[0.0; 2]; 24000];
            for chunk in out.chunks_mut(MAX_BLOCK) {
                s.process(&ctx, &params, &[[0.0; 2]; MAX_BLOCK][..chunk.len()], chunk);
            }
            out.split_off(12000)
        };
        // Only operator 1 at its full level: a sine at the note.
        let only_one = [(9, 0.0), (15, 0.0), (21, 0.0)];
        let pure = play(&only_one);
        let (f0, f2) = (level_at(&pure, 440.0, ctx.sr), level_at(&pure, 880.0, ctx.sr));
        assert!(f0 > 0.3 && f2 < f0 * 0.01, "a pure tone: {f0} {f2}");
        // Operator 2 modulating it in algorithm 1 adds harmonics.
        let modulated = play(&[(15, 0.0), (21, 0.0)]);
        assert!(level_at(&modulated, 880.0, ctx.sr) > 0.05, "operator 2 brings harmonics");
        // In algorithm 8 operator 2 is heard beside 1 rather than bending it.
        let apart = play(&[(1, 7.0), (10, 2.0), (15, 0.0), (21, 0.0)]);
        let (one, two) = (level_at(&apart, 440.0, ctx.sr), level_at(&apart, 880.0, ctx.sr));
        assert!(one > 0.2 && two > 0.1 && level_at(&apart, 1320.0, ctx.sr) < 0.01, "two sines: {one} {two}");
    }

    #[test]
    fn input_plays_what_arrives_on_its_tape() {
        let ctx = Ctx { sr: 1000.0, samples_per_line: 100.0, song_line: None };
        let tape = Arc::new(Tape::default());
        tape.start(4096);
        let mut input = LiveInput::new(tape.clone());
        let frames: Vec<Frame> = (0..100).map(|i| [i as f32 / 100.0, -(i as f32) / 100.0]).collect();
        tape.push(&frames);
        let mut out = [[0.0; 2]; 50];
        input.process(&ctx, &[2.0, 0.0], &[], &mut out);
        assert_eq!(out[10], [0.2, -0.2], "at its volume");
        input.process(&ctx, &[1.0, 1.0], &[], &mut out);
        assert_eq!(out[0], [0.5, 0.5], "the left side on both, picking up where it was");
        // Twice the rate: every other frame.
        tape.set_rate(2000);
        tape.push(&frames);
        input.process(&ctx, &[1.0, 0.0], &[], &mut out);
        // The last frame of before is held over, so it joins on smoothly.
        assert!((out[2][0] - out[1][0] - 0.02).abs() < 1e-5, "{:?}", &out[..3]);
    }

    #[test]
    fn waveshaper_bends_the_sound_through_its_curve() {
        let defaults: Vec<f32> = ModuleKind::WaveShaper.params().iter().map(|p| p.default).collect();
        let points = &defaults[4..];
        for x in [-0.9, -0.3, 0.0, 0.4, 1.0] {
            assert!((shape(points, true, x) - x).abs() < 1e-6, "the default curve is a straight line at {x}");
        }
        // A curve flat from +50% clips there, on both sides when symmetric.
        let mut clip = points.to_vec();
        clip[7] = 0.5;
        clip[8] = 0.5;
        assert_eq!(shape(&clip, true, 0.9), 0.5);
        assert_eq!(shape(&clip, true, -0.9), -0.5);
        assert_eq!(shape(&clip, false, -0.9), -0.9, "not symmetric: the left half is its own");
        let peak = effect_peak(ModuleKind::WaveShaper, &[(11, 0.5), (12, 0.5)], 100.0, 0.9);
        assert!((peak - 0.5).abs() < 1e-3, "{peak}");
    }

    #[test]
    fn filter_pro_types_and_slopes() {
        let at = |set: &[(usize, f32)], f: f32| effect_peak(ModuleKind::FilterPro, set, f, 0.5);
        let (pass, cut) = (at(&[], 100.0), at(&[], 8000.0));
        assert!(pass > 0.45 && cut < 0.05, "a lowpass at 1 kHz: {pass} {cut}");
        let steep = at(&[(4, 3.0)], 8000.0);
        assert!(steep < cut * 0.1, "48 dB an octave cuts much more: {steep} against {cut}");
        let high = at(&[(0, 1.0)], 100.0);
        assert!(high < 0.05, "a highpass cuts the lows: {high}");
        let notch = at(&[(0, 3.0), (2, 2.0)], 1000.0);
        assert!(notch < 0.01, "a notch takes out its frequency: {notch}");
        let peak = at(&[(0, 5.0), (3, 4.0)], 1000.0);
        assert!((peak - 2.0).abs() < 0.05, "a peak of 12 dB: {peak}");
    }

    #[test]
    fn chorus_voices_spread_a_tone_around_itself() {
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let input: Vec<Frame> = (0..48000).map(|i| [(TAU * 1000.0 * i as f32 / ctx.sr).sin(); 2]).collect();
        let run = |set: &[(usize, f32)]| {
            let mut params: Vec<f32> = ModuleKind::Chorus.params().iter().map(|p| p.default).collect();
            params[6] = 1.0;
            for &(i, v) in set {
                params[i] = v;
            }
            let mut out = vec![[0.0; 2]; input.len()];
            create(ModuleKind::Chorus, ctx.sr).process(&ctx, &params, &input, &mut out);
            out.split_off(4800)
        };
        let still = run(&[(2, 0.0), (0, 1.0)]);
        assert!(level_at(&still, 1000.0, ctx.sr) > 0.95, "one unswept voice is the tone, delayed");
        let moving = run(&[]);
        let left_right = moving.iter().map(|f| (f[0] - f[1]).abs()).fold(0f32, f32::max);
        assert!(level_at(&moving, 1000.0, ctx.sr) < 0.9 && left_right > 0.1, "swept voices smear it, wider than mono");
    }

    #[test]
    fn echo_repeats_after_its_time_and_fades() {
        let ctx = Ctx { sr: 1000.0, samples_per_line: 100.0, song_line: None };
        let mut params: Vec<f32> = ModuleKind::Echo.params().iter().map(|p| p.default).collect();
        params[2] = 0.0;
        params[3] = 0.0;
        params[4] = 1.0;
        let mut input = vec![[0.0; 2]; 1000];
        input[0] = [1.0; 2];
        let mut out = vec![[0.0; 2]; 1000];
        create(ModuleKind::Echo, ctx.sr).process(&ctx, &params, &input, &mut out);
        assert!(
            (out[300][0] - 1.0).abs() < 1e-4 && (out[600][0] - 0.5).abs() < 1e-4,
            "{} {}",
            out[300][0],
            out[600][0]
        );
        assert!(out[150][0].abs() < 1e-6, "nothing between");
    }

    #[test]
    fn analog_filter_cuts_and_sings() {
        let at = |set: &[(usize, f32)], f: f32| effect_peak(ModuleKind::AnalogFilter, set, f, 0.3);
        let dry = [(2, 0.0)];
        assert!(at(&dry, 100.0) > 0.25 && at(&dry, 8000.0) < 0.01, "a 24 dB lowpass");
        assert!(at(&[(0, 3.0), (2, 0.0)], 100.0) < 0.05, "the highpass cuts the lows");
        // At full resonance an impulse leaves it ringing at the cutoff.
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let mut input = vec![[0.0; 2]; 24000];
        input[0] = [0.5; 2];
        let mut out = vec![[0.0; 2]; input.len()];
        create(ModuleKind::AnalogFilter, ctx.sr).process(&ctx, &[0.0, 1000.0, 1.0, 1.0, 1.0], &input, &mut out);
        let late = out[20000..].iter().fold(0f32, |m, f| m.max(f[0].abs()));
        assert!(late > 0.01 && late < 2.0, "it sings by itself, and stays bounded: {late}");
    }

    #[test]
    fn plate_reverb_rings_on_longer_with_more_decay() {
        let ctx = Ctx { sr: 44100.0, samples_per_line: 5512.0, song_line: None };
        let tail = |decay: f32| {
            let mut input = vec![[0.0; 2]; 88200];
            input[0] = [1.0; 2];
            let mut out = vec![[0.0; 2]; input.len()];
            create(ModuleKind::PlateReverb, ctx.sr).process(&ctx, &[decay, 0.0, 0.3, 1.0, 1.0], &input, &mut out);
            let energy = |r: std::ops::Range<usize>| out[r].iter().map(|f| f[0] * f[0] + f[1] * f[1]).sum::<f32>();
            (energy(4410..22050), energy(44100..88200), out[1..].iter().any(|f| (f[0] - f[1]).abs() > 1e-4))
        };
        let (short_early, short_late, _) = tail(0.2);
        let (long_early, long_late, wide) = tail(0.9);
        assert!(short_early > 0.0 && long_early > 0.0, "it rings");
        assert!(long_late > short_late * 100.0, "more decay rings on: {long_late} {short_late}");
        assert!(wide, "the sides differ");
    }

    #[test]
    fn eq5_bands_move_their_frequencies() {
        let flat = effect_peak(ModuleKind::Eq5, &[], 1200.0, 0.25);
        assert!((flat - 0.25).abs() < 0.01, "flat at the defaults: {flat}");
        let mid = effect_peak(ModuleKind::Eq5, &[(7, 4.0)], 1200.0, 0.25);
        assert!((mid - 1.0).abs() < 0.05, "the mid band boosts 1.2 kHz by 12 dB: {mid}");
        let narrow = effect_peak(ModuleKind::Eq5, &[(7, 4.0), (8, 10.0)], 600.0, 0.25);
        assert!(narrow < 0.3, "a narrow one leaves an octave below alone: {narrow}");
    }

    #[test]
    fn eq_bands_boost_their_frequencies() {
        let flat = effect_peak(ModuleKind::Eq, &[], 100.0, 0.5);
        assert!((flat - 0.5).abs() < 0.01, "flat EQ changes nothing: {flat}");
        let low = effect_peak(ModuleKind::Eq, &[(0, 4.0)], 50.0, 0.5);
        assert!((low - 2.0).abs() < 0.1, "+12 dB low shelf: {low}");
        let high_at_low = effect_peak(ModuleKind::Eq, &[(2, 4.0)], 50.0, 0.5);
        assert!((high_at_low - 0.5).abs() < 0.05, "high shelf leaves 50 Hz: {high_at_low}");
        let mid = effect_peak(ModuleKind::Eq, &[(1, 0.25)], 1000.0, 0.5);
        assert!((mid - 0.125).abs() < 0.02, "-12 dB mid: {mid}");
    }

    #[test]
    fn compressor_turns_down_loud_input() {
        // Threshold 0.25, ratio 4: a peak of 1 (12 dB over) comes out 3 dB over.
        let loud = effect_peak(ModuleKind::Compressor, &[], 100.0, 1.0);
        assert!(loud < 0.5 && loud > 0.3, "{loud}");
        let quiet = effect_peak(ModuleKind::Compressor, &[], 100.0, 0.1);
        assert!((quiet - 0.1).abs() < 0.01, "{quiet}");
    }

    #[test]
    fn distortion_crushes_bits() {
        let sr = 48000.0;
        let ctx = Ctx { sr, samples_per_line: 6000.0, song_line: None };
        // Drive 1 and full tone keep the shape; 2 bits leave 2 levels a side.
        let params = [1.0, 1.0, 1.0, 1.0, 2.0, 1.0];
        let input: Vec<Frame> = (0..1000).map(|i| [(i as f32 / 1000.0) * 2.0 - 1.0; 2]).collect();
        let mut out = vec![[0.0; 2]; input.len()];
        let mut d = Distortion { lp: [0.0; 2], ..Default::default() };
        // Tone at 1 makes the smoothing filter pass everything at once.
        d.process(&ctx, &params, &input, &mut out);
        let mut levels: Vec<i32> = out.iter().map(|f| (f[0] / 0.7 * 100.0).round() as i32).collect();
        levels.dedup();
        assert_eq!(levels, vec![-100, -50, 0, 50, 100]);
    }

    #[test]
    fn lfo_tremolo_and_sync() {
        // Full depth: the volume dips to 0 and back once per period.
        let ctx = Ctx { sr: 1000.0, samples_per_line: 10.0, song_line: None };
        // Tremolo, sine, depth 1, rate ignored, synced to 10 lines.
        let params = [0.0, 0.0, 1.0, 2.0, 1.0, 10.0];
        let input = vec![[1.0; 2]; 100];
        let mut out = vec![[0.0; 2]; 100];
        Lfo::default().process(&ctx, &params, &input, &mut out);
        assert!((out[25][0] - 1.0).abs() < 1e-3, "top at a quarter: {}", out[25][0]);
        assert!(out[75][0].abs() < 1e-3, "silent at three quarters: {}", out[75][0]);
    }

    #[test]
    fn flanger_passes_signal() {
        let peak = effect_peak(ModuleKind::Flanger, &[(5, 0.0)], 100.0, 0.5);
        assert!((peak - 0.5).abs() < 1e-3, "dry: {peak}");
        let chorus = effect_peak(ModuleKind::Flanger, &[(0, 1.0)], 100.0, 0.5);
        assert!(chorus > 0.3 && chorus < 1.0, "{chorus}");
    }

    #[test]
    fn pitch_envelope_bends_and_holds_at_sustain() {
        let mut s = Sampler::new();
        s.set_samples(&[ramp_slot(100_000)]);
        // Up an octave at once, back to the note by 0.2 s, holding at the
        // octave (point 1) while the key is down.
        let pitch = VoiceEnvelope {
            on: true,
            points: vec![(0.0, 1.0), (0.1, 1.0), (0.2, 0.5)],
            sustain: Some(1),
            curve: false,
            amount: 12.0,
        };
        let m = Modulation { pitch, ..Modulation::default() };
        s.set_modulation(&m);
        s.note_on(0, 60.0, 1.0);
        render(&mut s, 1000);
        let v = s.voices.iter().find(|v| v.env.active()).unwrap();
        assert!((v.pos - 2000.0).abs() < 70.0, "twice as fast for a second: {}", v.pos);
        assert!((v.md.pitch_t - 0.1).abs() < 1e-6, "held at the sustain point");
        s.note_off(0);
        // A second's release keeps the voice going.
        let ctx = Ctx { sr: SR, samples_per_line: 100.0, song_line: None };
        let mut out = vec![[0.0; 2]; 300];
        s.process(&ctx, &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0], &[], &mut out);
        let v = s.voices.iter().find(|v| v.env.active()).unwrap();
        assert!(v.md.pitch_t > 0.2, "on past it once released: {}", v.md.pitch_t);
    }

    #[test]
    fn voice_filter_and_tremolo() {
        // A sample at the highest frequency there is.
        let frames: Vec<Frame> = (0..4000).map(|i| [if i % 2 == 0 { 0.5 } else { -0.5 }; 2]).collect();
        let sample = Sample { name: "nyquist".into(), sample_rate: SR, channels: 1, frames };
        let mut slot = SampleSlot::new(sample, None);
        slot.base_note = 60;
        let peak = |m: &Modulation| {
            let mut s = Sampler::new();
            s.set_samples(std::slice::from_ref(&slot));
            s.set_modulation(m);
            s.note_on(0, 60.0, 1.0);
            render(&mut s, 2000)[1000..].iter().fold(0f32, |a, x| a.max(x.abs()))
        };
        let open = peak(&Modulation::default());
        assert!(open > 0.4, "{open}");
        let low = Modulation { filter: true, cutoff: 20.0, ..Modulation::default() };
        assert!(peak(&low) < open * 0.05, "a 20 Hz lowpass takes it away: {}", peak(&low));

        // Full tremolo dips to silence once a cycle.
        let trem = Modulation {
            tremolo: VoiceLfo { on: true, shape: 0, rate: 4.0, depth: 1.0, delay: 0.0 },
            ..Modulation::default()
        };
        let mut s = Sampler::new();
        s.set_samples(std::slice::from_ref(&slot));
        s.set_modulation(&trem);
        s.note_on(0, 60.0, 1.0);
        let mut out = Vec::new();
        for _ in 0..16 {
            out.extend(render(&mut s, 32));
        }
        let quietest = out[100..].chunks(8).map(|c| c.iter().fold(0f32, |a, x| a.max(x.abs()))).fold(1f32, f32::min);
        assert!(quietest < 0.1, "{quietest}");
    }

    /// Plays A-4 on a fresh `kind` with `m` and counts the zero crossings
    /// and the peak of a second of the left channel, past the attack.
    fn synth_with(kind: ModuleKind, m: &Modulation) -> (usize, f32) {
        let sr = 48000.0;
        let ctx = Ctx { sr, samples_per_line: 6000.0, song_line: None };
        let mut params: Vec<f32> = kind.params().iter().map(|p| p.default).collect();
        if kind == ModuleKind::Generator {
            params[1] = 3.0; // sine
        }
        let mut dsp = create(kind, sr);
        dsp.set_modulation(m);
        dsp.note_on(0, 69.0, 1.0);
        let mut out = vec![[0.0; 2]; 64];
        let mut left = Vec::new();
        for _ in 0..(sr as usize / 64) {
            dsp.process(&ctx, &params, &[], &mut out);
            left.extend(out.iter().map(|f| f[0]));
        }
        let tail = &left[4800..];
        let crossings = tail.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        (crossings, tail.iter().fold(0f32, |a, x| a.max(x.abs())))
    }

    #[test]
    fn synths_take_modulation() {
        let plain = synth_with(ModuleKind::Generator, &Modulation::default());
        let pitch = VoiceEnvelope { on: true, points: vec![(0.0, 1.0)], sustain: None, curve: false, amount: 12.0 };
        let up = Modulation { pitch, ..Modulation::default() };
        let octave = synth_with(ModuleKind::Generator, &up);
        let ratio = octave.0 as f32 / plain.0 as f32;
        assert!((ratio - 2.0).abs() < 0.05, "an octave up: {ratio}");
        let shut = Modulation { filter: true, cutoff: 20.0, resonance: 0.0, ..Modulation::default() };
        let filtered = synth_with(ModuleKind::Generator, &shut);
        assert!(filtered.1 < plain.1 * 0.05, "{} {}", filtered.1, plain.1);
        let fm = synth_with(ModuleKind::Fm, &Modulation::default());
        let fm_up = synth_with(ModuleKind::Fm, &up);
        assert!(fm_up.0 as f32 > fm.0 as f32 * 1.5, "{} {}", fm_up.0, fm.0);
    }

    #[test]
    fn slices_play_on_the_notes_after_the_base_note() {
        let mut s = Sampler::new();
        let mut slot = ramp_slot(100);
        slot.slices = vec![25, 50];
        s.set_samples(&[slot]);
        // Slice 2 (frames 25..50) is on D-5, two notes above C-5.
        s.note_on(0, 62.0, 1.0);
        let out = render(&mut s, 40);
        assert!((out[0] - 0.25).abs() < 0.02, "starts at its marker: {}", out[0]);
        assert!(out[30].abs() < 1e-6, "stops at the next one: {}", out[30]);
        assert_eq!(s.voices.iter().filter(|v| v.env.active()).count(), 0);
        // The whole sample is on the base note only.
        s.note_on(1, 60.0, 1.0);
        let out = render(&mut s, 80);
        assert!((out[70] - 0.7).abs() < 0.02, "{}", out[70]);
        s.note_on(2, 70.0, 1.0);
        assert!(s.voices.iter().filter(|v| v.env.active()).count() <= 1, "no zone above the slices");
        let mut heads = Vec::new();
        s.playheads(&mut heads);
        assert!(heads.iter().all(|p| p.slot == 0), "playheads name the slot");
    }

    #[test]
    fn slices_have_settings_of_their_own() {
        let mut s = Sampler::new();
        let mut slot = ramp_slot(100);
        slot.slices = vec![50];
        // The second slice is at half volume and loops.
        *slot.slice_mut(1) = crate::project::SliceSettings { volume: 0.5, loop_mode: 1, ..Default::default() };
        s.set_samples(&[slot]);
        s.note_on(0, 62.0, 1.0);
        let out = render(&mut s, 120);
        assert!((out[0] - 0.25).abs() < 0.02, "half of 0.5: {}", out[0]);
        assert!((out[60] - 0.3).abs() < 0.02, "looped back to its start: {}", out[60]);
    }
}
