//! The effects moved by an LFO: LFO, Flanger, Phaser, Chorus, Vibrato and Ring Mod.

use super::*;

// ---------------------------------------------------------------- LFO

/// The LFO: moves the volume (tremolo) or the panning of its input.
#[derive(Default)]
pub(super) struct Lfo {
    pub(super) phase: f32,
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
pub(super) struct Flanger {
    pub(super) line: DelayLine,
    pub(super) phase: f32,
}

impl Flanger {
    pub(super) fn new(sr: f32) -> Self {
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
pub(super) const MAX_STAGES: usize = 12;

/// A phaser: a chain of first-order allpass filters swept
/// between two frequencies by an LFO, mixed back with the input so their
/// phase shifts cut notches. The right channel runs a quarter cycle
/// behind.
#[derive(Default)]
pub(super) struct Phaser {
    /// Each channel's stages, as (last input, last output).
    pub(super) stages: [[(f32, f32); MAX_STAGES]; 2],
    pub(super) last: [f32; 2],
    pub(super) phase: f32,
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

// ---------------------------------------------------------------- chorus

/// How far the chorus's voices sweep at full depth, in seconds.
pub(super) const CHORUS_SWEEP: f32 = 0.008;

/// The Chorus: up to four copies of the sound, each read through a
/// delay that sweeps at the same rate but its own point in the cycle, so
/// they drift around each other; the right channel's cycle runs Stereo
/// behind the left's.
pub(super) struct Chorus {
    pub(super) line: DelayLine,
    pub(super) phase: f32,
}

impl Chorus {
    pub(super) fn new(sr: f32) -> Self {
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

// ---------------------------------------------------------------- vibrato

/// The most the vibrato's delay swings, in seconds: about a quarter tone
/// at 5 Hz.
pub(super) const VIBRATO_SWING: f32 = 0.004;

/// The Vibrato: the sound read back through a delay that swings
/// longer and shorter, which bends its pitch up and down. Stereo puts the
/// right channel's swing up to half a cycle behind.
pub(super) struct Vibrato {
    pub(super) line: DelayLine,
    pub(super) phase: f32,
}

impl Vibrato {
    pub(super) fn new(sr: f32) -> Self {
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

// ---------------------------------------------------------------- ring modulator

/// The Ring Mod: the input times a carrier wave, which turns each of
/// its frequencies into their sum and difference with the carrier's.
/// Stereo puts the right channel's carrier up to half a cycle behind.
#[derive(Default)]
pub(super) struct RingMod {
    pub(super) phase: f32,
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
