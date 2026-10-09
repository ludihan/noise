//! The effects that bend the waveform: Distortion, WaveShaper, Cabinet Simulator and Exciter.

use super::*;

// ---------------------------------------------------------------- distortion

#[derive(Default)]
pub(super) struct Distortion {
    pub(super) lp: [f32; 2],
    /// The held input of the sample-rate reducer and how long it is held.
    pub(super) hold: Frame,
    pub(super) held: f32,
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

// ---------------------------------------------------------------- waveshaper

/// Points on the WaveShaper's curve, evenly from -1 to 1.
pub(super) const SHAPE_POINTS: usize = 9;

/// The WaveShaper: the sound bent through a curve, drawn here as nine
/// points from -1 to 1 with straight lines between. Symmetric mirrors the
/// right half onto the left, so only the points from 0 up count. A level
/// past ±1 follows the end of the curve.
#[derive(Default)]
pub(super) struct WaveShaper;

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
