//! The reverbs: Reverb, a Freeverb-style network, and Plate Reverb.

use super::*;

// ---------------------------------------------------------------- reverb

pub(super) struct Comb {
    pub(super) buf: Vec<f32>,
    pub(super) pos: usize,
    pub(super) store: f32,
}

impl Comb {
    pub(super) fn tick(&mut self, x: f32, fb: f32, damp: f32) -> f32 {
        let y = self.buf[self.pos];
        self.store = y * (1.0 - damp) + self.store * damp;
        self.buf[self.pos] = x + self.store * fb;
        self.pos = (self.pos + 1) % self.buf.len();
        y
    }
}

pub(super) struct Allpass {
    pub(super) buf: Vec<f32>,
    pub(super) pos: usize,
}

impl Allpass {
    pub(super) fn tick(&mut self, x: f32) -> f32 {
        let b = self.buf[self.pos];
        self.buf[self.pos] = x + b * 0.5;
        self.pos = (self.pos + 1) % self.buf.len();
        b - x
    }
}

/// Freeverb.
pub(super) struct Reverb {
    pub(super) combs: [Vec<Comb>; 2],
    pub(super) allpasses: [Vec<Allpass>; 2],
}

impl Reverb {
    pub(super) fn new(sr: f32) -> Self {
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

// ---------------------------------------------------------------- plate reverb

/// A ring of one channel's last samples.
pub(super) struct Ring {
    pub(super) buf: Vec<f32>,
    pub(super) pos: usize,
}

impl Ring {
    pub(super) fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len.max(2)], pos: 0 }
    }

    /// The sample written `d` samples ago (1 is the last), between them.
    pub(super) fn at(&self, d: f32) -> f32 {
        let len = self.buf.len();
        let d = d.clamp(1.0, (len - 2) as f32);
        let i = d as usize;
        let wrap = |i: usize| if i >= len { i - len } else { i };
        let (a, b) = (self.buf[wrap(self.pos + len - i)], self.buf[wrap(self.pos + len - i - 1)]);
        a + (b - a) * fract(d)
    }

    pub(super) fn push(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) % self.buf.len();
    }
}

/// An allpass diffuser over a ring of `len` samples with gain `g`, read
/// `delay` back (`len` less a sweep, if it is modulated).
pub(super) fn diffuse(ring: &mut Ring, delay: f32, g: f32, x: f32) -> f32 {
    let back = ring.at(delay);
    let v = x - g * back;
    ring.push(v);
    back + g * v
}

/// The plate's delays at the 29761 Hz they were tuned at.
pub(super) const PLATE_RATE: f32 = 29761.0;
pub(super) const PLATE_DIFFUSERS: [(f32, f32); 4] = [(142.0, 0.75), (107.0, 0.75), (379.0, 0.625), (277.0, 0.625)];
/// Each half of the tank: its swept allpass, delay, allpass and delay.
pub(super) const PLATE_TANK: [[f32; 4]; 2] = [[672.0, 4453.0, 1800.0, 3720.0], [908.0, 4217.0, 2656.0, 3163.0]];
/// How far the swept allpasses move, in samples at the tuning rate.
pub(super) const PLATE_SWEEP: f32 = 16.0;

/// A plate reverb after Dattorro's: the sound diffused by four allpasses,
/// then round a figure-eight tank of two halves, each a swept allpass, a
/// delay, damping, an allpass and a delay, feeding the other; each side
/// listens at seven points of the tank.
pub(super) struct PlateReverb {
    pub(super) scale: f32,
    pub(super) predelay: Ring,
    pub(super) input_lp: f32,
    pub(super) diffusers: Vec<Ring>,
    /// The two halves of the tank: swept allpass, delay, allpass, delay.
    pub(super) tank: [[Ring; 4]; 2],
    pub(super) damp: [f32; 2],
    pub(super) phase: f32,
}

impl PlateReverb {
    pub(super) fn new(sr: f32) -> Self {
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
