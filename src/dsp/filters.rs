//! The filters: Filter, Filter Pro, Analog Filter, Vocal Filter, Comb Filter and Scream Filter.

use super::*;

// ---------------------------------------------------------------- filter

/// Topology-preserving state variable filter (Simper).
#[derive(Default, Clone, Copy)]
pub(super) struct Svf {
    pub(super) ic1: f32,
    pub(super) ic2: f32,
}

impl Svf {
    pub(super) fn tick(&mut self, x: f32, g: f32, k: f32) -> (f32, f32, f32) {
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
pub(super) struct Filter {
    pub(super) svf: [Svf; 2],
    pub(super) lfo: f32,
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

// ---------------------------------------------------------------- vocal filter

/// The formants of a tenor singing A, E, I, O and U (Csound's table): the
/// frequency in Hz, the level in dB and the bandwidth in Hz of each.
pub(super) const VOWELS: [[(f32, f32, f32); 5]; 5] = [
    [(650.0, 0.0, 80.0), (1080.0, -6.0, 90.0), (2650.0, -7.0, 120.0), (2900.0, -8.0, 130.0), (3250.0, -22.0, 140.0)],
    [(400.0, 0.0, 70.0), (1700.0, -14.0, 80.0), (2600.0, -12.0, 100.0), (3200.0, -14.0, 120.0), (3580.0, -20.0, 120.0)],
    [(290.0, 0.0, 40.0), (1870.0, -15.0, 90.0), (2800.0, -18.0, 100.0), (3250.0, -20.0, 120.0), (3540.0, -30.0, 120.0)],
    [(400.0, 0.0, 40.0), (800.0, -10.0, 80.0), (2600.0, -12.0, 100.0), (2800.0, -12.0, 120.0), (3000.0, -26.0, 120.0)],
    [(350.0, 0.0, 40.0), (600.0, -20.0, 60.0), (2700.0, -17.0, 100.0), (2900.0, -14.0, 120.0), (3300.0, -26.0, 120.0)],
];

/// The Vocal Filter: band-pass filters on the formants of a vowel,
/// morphing from one vowel to the next, which make any sound say it.
#[derive(Default)]
pub(super) struct VocalFilter {
    pub(super) bands: [[Svf; 2]; 5],
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

// ---------------------------------------------------------------- comb filter

/// The Comb Filter: a delay one cycle of a note long fed back into
/// itself, which rings at the note and its harmonics (or, with negative
/// feedback, an octave down and its odd harmonics), with damping that
/// dulls the higher ones.
pub(super) struct CombFilter {
    pub(super) line: DelayLine,
    pub(super) lp: Frame,
}

impl CombFilter {
    pub(super) fn new(sr: f32) -> Self {
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

// ---------------------------------------------------------------- scream filter

/// The Scream Filter: a resonant filter driven hard, with the
/// distortion inside its loop, so the resonance growls and can scream at
/// full without running away.
#[derive(Default)]
pub(super) struct ScreamFilter {
    pub(super) svf: [Svf; 2],
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

// ---------------------------------------------------------------- filter pro

/// The Filter Pro: eight filter types, each one to four biquads deep
/// for a slope of 12 to 48 dB an octave, with resonance on the first, and
/// gain for the peak and shelves.
#[derive(Default)]
pub(super) struct FilterPro {
    pub(super) stages: [Biquad; 4],
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

// ---------------------------------------------------------------- analog filter

/// The Analog Filter, a Moog-style ladder: four one-pole stages in
/// a row with the last fed back against the input, which resonates and at
/// full resonance sings by itself; the input is driven into a soft clip,
/// as the transistors would. The types are mixes of the stages'
/// outputs, after the Oberheim Xpander.
#[derive(Default)]
pub(super) struct AnalogFilter {
    pub(super) ladders: [Ladder; 2],
}

/// One channel of the ladder: its four stages' states.
#[derive(Clone, Copy, Default)]
pub(super) struct Ladder {
    pub(super) stages: [f32; 4],
}

impl Ladder {
    /// Takes `x` through the ladder for one frame, at `g`, the cutoff as
    /// `tan(PI * cutoff / sr)`, and resonance `k`, 0..4, mixed to the
    /// `LADDER_TYPES` type `kind`.
    pub(super) fn tick(&mut self, x: f32, g: f32, k: f32, kind: u32) -> f32 {
        // Zero-delay feedback, after Zavalishin: each stage is a
        // trapezoidal one-pole, and the loop is solved for this frame, so
        // full resonance (4) rings at any cutoff.
        let big = g / (1.0 + g);
        let s = &mut self.stages;
        let past = (big.powi(3) * s[0] + big * big * s[1] + big * s[2] + s[3]) / (1.0 + g);
        let u = ((x - k * past) / (1.0 + k * big.powi(4))).tanh();
        let mut y = [0.0; 4];
        let mut x = u;
        for (st, yk) in s.iter_mut().zip(&mut y) {
            let v = (x - *st) * big;
            *yk = v + *st;
            *st = *yk + v;
            x = *yk;
        }
        // The feedback takes the level down; most of it is made up.
        (match kind {
            0 => y[3],
            1 => y[1],
            2 => 2.0 * (y[0] - y[1]),
            _ => u - 4.0 * y[0] + 6.0 * y[1] - 4.0 * y[2] + y[3],
        }) * (1.0 + 0.5 * k)
    }
}

impl Dsp for AnalogFilter {
    fn reset(&mut self) {
        self.ladders = Default::default();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let kind = p[0].round() as u32;
        let g = (PI * p[1].min(ctx.sr * 0.45) / ctx.sr).tan();
        let k = 4.0 * p[2].clamp(0.0, 1.0);
        let (drive, mix) = (p[3], p[4]);
        for (o, i) in out.iter_mut().zip(input) {
            for ch in 0..2 {
                let wet = self.ladders[ch].tick(i[ch] * drive, g, k, kind);
                o[ch] = i[ch] * (1.0 - mix) + wet * mix;
            }
        }
    }
}
