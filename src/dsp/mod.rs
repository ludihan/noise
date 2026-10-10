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
    /// `process`, listening to `key` (another module's sound) instead of
    /// `input` where the module takes a key input.
    fn process_keyed(&mut self, ctx: &Ctx, params: &[f32], input: &[Frame], _key: &[Frame], out: &mut [Frame]) {
        self.process(ctx, params, input, out);
    }
}

pub fn create(kind: ModuleKind, sr: f32) -> Box<dyn Dsp> {
    match kind {
        ModuleKind::Output => Box::new(Output),
        ModuleKind::Generator => Box::new(Generator::new()),
        ModuleKind::Wavetable => Box::new(wavetable::Wavetable::new()),
        ModuleKind::Analog => Box::new(analog::Analog::new()),
        ModuleKind::PluckedString => Box::new(string::PluckedString::new()),
        ModuleKind::Vocoder => Box::new(vocoder::Vocoder::new()),
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
        ModuleKind::Convolver => Box::new(convolver::Convolver::new()),
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
mod analog;
pub mod convolver;
mod delays;
mod distortion;
mod drums;
mod dynamics;
mod eq;
mod filters;
mod fm;
mod generator;
pub mod granular;
mod modulated;
mod modulation;
mod reverb;
mod sampler;
mod spectravoice;
mod string;
mod utility;
mod vocoder;
pub mod wavetable;
pub use modulated::lfo_shape;
pub use utility::LiveInput;
use {
    delays::*, distortion::*, drums::*, dynamics::*, eq::*, filters::*, fm::*, generator::*, modulated::*,
    modulation::*, reverb::*, sampler::*, spectravoice::*, utility::*,
};

#[cfg(test)]
mod tests;
