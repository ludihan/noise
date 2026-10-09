//! The equalizers, EQ, EQ 5 and EQ 10, on the biquads they share, and the DC Blocker.

use super::*;

// ---------------------------------------------------------------- EQ 10

/// The EQ 10: a graphic EQ, ten peaks an octave apart from 31 Hz
/// to 16 kHz, each up or down by 12 dB.
#[derive(Default)]
pub(super) struct Eq10 {
    pub(super) bands: [Biquad; 10],
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

// ---------------------------------------------------------------- EQ 5

/// The EQ 5: a parametric EQ of a low shelf, three peaks and a high
/// shelf, each with its frequency, gain and width.
#[derive(Default)]
pub(super) struct Eq5 {
    pub(super) bands: [Biquad; 5],
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

// ---------------------------------------------------------------- EQ

/// A biquad in transposed direct form II.
#[derive(Default, Clone, Copy)]
pub(super) struct Biquad {
    pub(super) b: [f32; 3],
    pub(super) a: [f32; 2],
    pub(super) z: [[f32; 2]; 2],
}

/// The shapes a `Biquad` takes, after the RBJ cookbook.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Shape {
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
    pub(super) const ALL: [Shape; 8] = [
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
    pub(super) fn design(&mut self, kind: i32, freq: f32, gain: f32, sr: f32) {
        let (shape, q) = match kind {
            0 => (Shape::Peak, 0.9),
            k if k < 0 => (Shape::LowShelf, std::f32::consts::FRAC_1_SQRT_2),
            _ => (Shape::HighShelf, std::f32::consts::FRAC_1_SQRT_2),
        };
        self.rbj(shape, freq, q, gain, sr);
    }

    /// `shape` at `freq` with resonance `q`; `gain` is for the peak and
    /// shelves.
    pub(super) fn rbj(&mut self, shape: Shape, freq: f32, q: f32, gain: f32, sr: f32) {
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

    pub(super) fn clear(&mut self) {
        self.z = [[0.0; 2]; 2];
    }

    pub(super) fn tick(&mut self, ch: usize, x: f32) -> f32 {
        let z = &mut self.z[ch];
        let y = self.b[0] * x + z[0];
        z[0] = self.b[1] * x - self.a[0] * y + z[1];
        z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// A three-band EQ: low shelf, mid peak and high shelf.
#[derive(Default)]
pub(super) struct Eq {
    pub(super) bands: [Biquad; 3],
}

/// Channel `ch`'s `x` through each of `bands` in turn.
pub(super) fn through(bands: &mut [Biquad], ch: usize, x: f32) -> f32 {
    bands.iter_mut().fold(x, |x, b| b.tick(ch, x))
}

/// Runs `input` through each of `bands` in turn, into `out`.
pub(super) fn run_bands(bands: &mut [Biquad], input: &[Frame], out: &mut [Frame]) {
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

// ---------------------------------------------------------------- DC blocker

/// The DC Blocker: a highpass far below hearing that takes away an
/// offset (from distortion, ring modulation or uneven waves) so it doesn't
/// eat into the headroom.
#[derive(Default)]
pub(super) struct DcBlocker {
    /// The last input and output of each channel.
    pub(super) last: [Frame; 2],
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
