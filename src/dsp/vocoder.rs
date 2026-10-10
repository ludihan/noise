//! The Vocoder: the key input (a voice, drums) split into bands, and how
//! loud each band is shapes the same band of the module's own input (a
//! synth), so the synth speaks with the key's voice.

use super::*;

const MAX_BANDS: usize = 32;

#[derive(Clone, Copy, Default)]
struct Band {
    /// The key's band and how loud it is.
    key: Svf,
    level: f32,
    /// The input's band, a channel each.
    carrier: [Svf; 2],
}

pub(super) struct Vocoder {
    bands: [Band; MAX_BANDS],
    rng: Rng,
}

impl Vocoder {
    pub(super) fn new() -> Self {
        Self { bands: [Band::default(); MAX_BANDS], rng: Rng(0x0c0d_e123) }
    }
}

impl Dsp for Vocoder {
    fn reset(&mut self) {
        self.bands = [Band::default(); MAX_BANDS];
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        self.process_keyed(ctx, p, input, input, out);
    }

    fn process_keyed(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], key: &[Frame], out: &mut [Frame]) {
        let n = (p[0].round() as usize).clamp(1, MAX_BANDS);
        let (low, high) = (p[1], p[2].max(p[1] * 1.01).min(ctx.sr * 0.45));
        let k = 1.0 / p[3].max(0.5);
        let coef = |t: f32| 1.0 - (-1.0 / (t.max(1e-5) * ctx.sr)).exp();
        let (att, rel) = (coef(p[4]), coef(p[5]));
        let (noise, gain, mix) = (p[6], p[7], p[8]);
        // The bands spread evenly in octaves from Low to High.
        let mut g = [0.0; MAX_BANDS];
        for (b, g) in g.iter_mut().enumerate().take(n) {
            let at = if n > 1 { b as f32 / (n - 1) as f32 } else { 0.5 };
            *g = (PI * low * (high / low).powf(at) / ctx.sr).tan();
        }
        // Each band's peak is 1/k; the bands' sum is about as loud as the
        // input over every band.
        let norm = gain * k * (n as f32).sqrt();
        for ((o, i), kf) in out.iter_mut().zip(input).zip(key) {
            let m = 0.5 * (kf[0] + kf[1]);
            // Noise in the carrier lets the key's hiss and consonants through.
            let hiss = noise * self.rng.next();
            let mut wet = [0.0; 2];
            for (band, &g) in self.bands.iter_mut().zip(&g).take(n) {
                let (_, _, kb) = band.key.tick(m, g, k);
                let a = (kb * k).abs();
                band.level += if a > band.level { att } else { rel } * (a - band.level);
                for ch in 0..2 {
                    let (_, _, cb) = band.carrier[ch].tick(i[ch] + hiss, g, k);
                    wet[ch] += cb * band.level;
                }
            }
            for ch in 0..2 {
                o[ch] = i[ch] * (1.0 - mix) + wet[ch] * norm * mix;
            }
        }
    }
}
