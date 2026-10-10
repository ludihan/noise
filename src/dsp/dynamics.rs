//! The dynamics: Gate, Maximizer and Compressor.

use super::*;

// ---------------------------------------------------------------- gate

/// The Gate: lets the sound through while it is louder than the
/// threshold, and for the hold time after, and turns it down to the floor
/// otherwise, opening over the attack and closing over the release.
#[derive(Default)]
pub(super) struct Gate {
    /// How far open the gate is, from 0 (at the floor) to 1.
    pub(super) open: f32,
    /// Frames left before the gate starts closing.
    pub(super) hold: f32,
}

impl Dsp for Gate {
    fn reset(&mut self) {
        self.open = 0.0;
        self.hold = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        self.process_keyed(ctx, p, input, input, out);
    }

    fn process_keyed(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], key: &[Frame], out: &mut [Frame]) {
        let (threshold, floor) = (p[0], p[4]);
        let coef = |t: f32| 1.0 - (-1.0 / (t.max(1e-5) * ctx.sr)).exp();
        let (att, rel) = (coef(p[1]), coef(p[3]));
        for ((o, i), k) in out.iter_mut().zip(input).zip(key) {
            if k[0].abs().max(k[1].abs()) > threshold {
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

// ---------------------------------------------------------------- maximizer

/// How far ahead the Maximizer looks, in seconds.
pub(super) const LOOKAHEAD: f32 = 0.0015;

/// The Maximizer: boosts the sound and keeps its peaks under the
/// ceiling. It looks a moment ahead, so the gain is already down when a
/// peak arrives, and lets the gain back up over the release.
pub(super) struct Maximizer {
    /// The boosted input, waiting out the lookahead.
    pub(super) delay: Vec<Frame>,
    /// The gain each frame in `delay` needs to stay under the ceiling.
    pub(super) needs: Vec<f32>,
    pub(super) pos: usize,
    pub(super) gain: f32,
}

impl Maximizer {
    pub(super) fn new(sr: f32) -> Self {
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

// ---------------------------------------------------------------- compressor

/// A feed-forward compressor following the louder channel of its input,
/// or of its key input.
#[derive(Default)]
pub(super) struct Compressor {
    pub(super) env: f32,
}

impl Dsp for Compressor {
    fn reset(&mut self) {
        self.env = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        self.process_keyed(ctx, p, input, input, out);
    }

    fn process_keyed(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], key: &[Frame], out: &mut [Frame]) {
        let (threshold, ratio, makeup, mix) = (p[0], p[1], p[4], p[5]);
        let coef = |t: f32| 1.0 - (-1.0 / (t.max(1e-5) * ctx.sr)).exp();
        let (att, rel) = (coef(p[2]), coef(p[3]));
        for ((o, i), k) in out.iter_mut().zip(input).zip(key) {
            let level = k[0].abs().max(k[1].abs());
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
