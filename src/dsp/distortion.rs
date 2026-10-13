//! The effects that bend the waveform: Distortion, WaveShaper, Cabinet Simulator and Exciter.

use super::*;

// ---------------------------------------------------------------- oversampling

/// The resonances of four biquads that make an eighth-order Butterworth
/// lowpass.
const BUTTERWORTH_8: [f32; 4] = [0.5098, 0.6013, 0.9000, 2.5629];

/// Runs a curve at a multiple of the sample rate: the overtones it makes
/// past the top of hearing are filtered off there, rather than folding
/// back down as aliasing. The sound is stuffed with zeros up to the
/// higher rate and smoothed by a lowpass, bent, and lowpassed again
/// before every so many samples are kept. The sound left unbent goes
/// through filters of its own, so that it is as late as the bent sound
/// it is mixed with, rather than cancelling it in the highs.
#[derive(Default)]
pub(super) struct Oversampler {
    /// The lowpass up, then the one down, of the bent sound and of the
    /// sound as it was.
    wet: [Biquad; 8],
    dry: [Biquad; 8],
    /// The factor and sample rate the filters are made for.
    made: (usize, f32),
}

/// `x` stuffed with zeros up to `factor` times the rate, through the
/// first half of `filters`, `curve` and the second half, every `factor`th
/// sample kept.
fn oversample(filters: &mut [Biquad; 8], factor: usize, ch: usize, x: f32, curve: impl Fn(f32) -> f32) -> f32 {
    if factor <= 1 {
        return curve(x);
    }
    let (up, down) = filters.split_at_mut(4);
    let mut y = 0.0;
    for k in 0..factor {
        let stuffed = if k == 0 { x * factor as f32 } else { 0.0 };
        y = through(down, ch, curve(through(up, ch, stuffed)));
    }
    y
}

impl Oversampler {
    /// The factor that `OVERSAMPLE` choice `p` stands for.
    pub(super) fn factor(p: f32) -> usize {
        1 << (p.round() as usize).min(3)
    }

    pub(super) fn reset(&mut self) {
        self.wet.iter_mut().chain(&mut self.dry).for_each(Biquad::clear);
    }

    /// Makes the filters for running `factor` times sample rate `sr`.
    pub(super) fn prepare(&mut self, factor: usize, sr: f32) {
        if self.made == (factor, sr) {
            return;
        }
        self.made = (factor, sr);
        for (b, &q) in self.wet.iter_mut().chain(&mut self.dry).zip(BUTTERWORTH_8.iter().cycle()) {
            b.rbj(Shape::Lowpass, sr * 0.42, q, 1.0, sr * factor as f32);
        }
        self.reset();
    }

    /// Channel `ch`'s `x` through `curve`, at the factor `prepare` made
    /// the filters for.
    pub(super) fn run(&mut self, ch: usize, x: f32, curve: impl Fn(f32) -> f32) -> f32 {
        oversample(&mut self.wet, self.made.0, ch, x, curve)
    }

    /// Channel `ch`'s `x` unbent, as late as `run` makes what it bends,
    /// to mix with it.
    pub(super) fn dry(&mut self, ch: usize, x: f32) -> f32 {
        oversample(&mut self.dry, self.made.0, ch, x, |x| x)
    }
}

// ---------------------------------------------------------------- distortion

#[derive(Default)]
pub(super) struct Distortion {
    pub(super) lp: [f32; 2],
    /// The held input of the sample-rate reducer and how long it is held.
    pub(super) hold: Frame,
    pub(super) held: f32,
    pub(super) oversampler: Oversampler,
}

impl Dsp for Distortion {
    fn reset(&mut self) {
        self.lp = [0.0; 2];
        self.hold = [0.0; 2];
        self.held = 0.0;
        self.oversampler.reset();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (drive, tone, mix) = (p[0], p[1], p[2]);
        let kind = p[3].round() as u32;
        let steps = 2f32.powf(p[4].round() - 1.0);
        let crush = p[4] < 15.5;
        let downsample = p[5].round().max(1.0);
        let norm = 1.0 / drive.tanh();
        let coef = 0.05 + 0.95 * tone * tone;
        self.oversampler.prepare(Oversampler::factor(p[6]), ctx.sr);
        for (o, i) in out.iter_mut().zip(input) {
            // Sample-rate reduction: hold each input for `downsample` frames.
            self.held -= 1.0;
            if self.held <= 0.0 {
                self.hold = *i;
                self.held += downsample;
            }
            for ch in 0..2 {
                let mut wet = self.oversampler.run(ch, self.hold[ch] * drive, |x| match kind {
                    0 => x.tanh() * norm,
                    1 => x.clamp(-1.0, 1.0),
                    // Folds back from ±1 instead of flattening.
                    _ => {
                        let t = (x + 1.0).rem_euclid(4.0);
                        if t < 2.0 { t - 1.0 } else { 3.0 - t }
                    }
                });
                if crush {
                    wet = (wet * steps).round() / steps;
                }
                self.lp[ch] += coef * (wet - self.lp[ch]);
                o[ch] = self.oversampler.dry(ch, i[ch]) * (1.0 - mix) + self.lp[ch] * mix * 0.7;
            }
        }
    }
}

// ---------------------------------------------------------------- waveshaper

/// Points on the WaveShaper's curve, evenly from -1 to 1.
pub(super) const SHAPE_POINTS: usize = 9;

/// The WaveShaper: the sound bent through a curve, drawn here as nine
/// points from -1 to 1 with straight lines between. Symmetric mirrors the
/// right half onto the left, so only the points from 0 up count. A level
/// past ±1 follows the end of the curve.
#[derive(Default)]
pub(super) struct WaveShaper {
    pub(super) oversampler: Oversampler,
}

/// `x` through the curve of `points`, evenly spaced from -1 to 1.
pub(super) fn shape(points: &[f32], symmetric: bool, x: f32) -> f32 {
    if symmetric && x < 0.0 {
        return -shape(points, true, -x);
    }
    let at = (x.clamp(-1.0, 1.0) + 1.0) * 0.5 * (points.len() - 1) as f32;
    let i = (at as usize).min(points.len() - 2);
    let t = at - i as f32;
    points[i] + (points[i + 1] - points[i]) * t
}

impl Dsp for WaveShaper {
    fn reset(&mut self) {
        self.oversampler.reset();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (gain, symmetric, level, mix) = (p[0], p[1] >= 0.5, p[2], p[3]);
        let points = &p[4..4 + SHAPE_POINTS];
        self.oversampler.prepare(Oversampler::factor(p[4 + SHAPE_POINTS]), ctx.sr);
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let wet = self.oversampler.run(ch, i[ch] * gain, |x| shape(points, symmetric, x)) * level;
                o[ch] = self.oversampler.dry(ch, i[ch]) * (1.0 - mix) + wet * mix;
            }
        }
    }
}

// ---------------------------------------------------------------- cabinet simulator

/// Each cabinet's tone as (shelf or peak, frequency, gain) bands, as
/// `Biquad::design` takes them: a speaker's lows and highs rolled off and
/// its body and bite.
pub(super) const CABINET_BANDS: [[(i32, f32, f32); 4]; 4] = [
    [(-1, 120.0, 0.1), (0, 400.0, 1.3), (0, 1800.0, 2.0), (1, 4500.0, 0.08)],
    [(-1, 80.0, 0.2), (0, 110.0, 1.6), (0, 2500.0, 1.8), (1, 5000.0, 0.05)],
    [(-1, 40.0, 0.3), (0, 80.0, 1.8), (0, 700.0, 1.2), (1, 2500.0, 0.05)],
    [(-1, 400.0, 0.05), (0, 1500.0, 2.5), (0, 2500.0, 1.2), (1, 3000.0, 0.03)],
];

/// The Cabinet Simulator: the sound driven a little, as an amp
/// would, and shaped as one of four speaker cabinets.
#[derive(Default)]
pub(super) struct Cabinet {
    pub(super) bands: [Biquad; 4],
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

// ---------------------------------------------------------------- exciter

/// The Exciter: the highs above Frequency, driven into a soft clip
/// that gives them new overtones, added back to the sound for air and
/// presence.
#[derive(Default)]
pub(super) struct Exciter {
    /// Two one-pole lowpasses per channel; the input less them is the highs.
    pub(super) lp: [[f32; 2]; 2],
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
