//! The Convolver: the sound played through an impulse response, the echo
//! of a click in a room, a hall, a plate, a spring or a speaker cabinet,
//! or any sample loaded as one.
//!
//! The first `PART` frames of the impulse are summed directly, frame by
//! frame; the rest in blocks of `PART` through an FFT (uniformly
//! partitioned convolution), from input blocks already complete, so the
//! sound comes out with no delay.

use super::fft::Fft;
use super::*;

/// Frames in each block of the impulse.
const PART: usize = 512;
/// The FFT length: two blocks, so a block's convolution fits.
const SPAN: usize = 2 * PART;
/// Frequency bins kept: the rest mirror them, the signals being real.
const BINS: usize = SPAN / 2 + 1;
/// The longest impulse, in seconds.
const MAX_SECONDS: f32 = 6.0;
/// Blocks of a new impulse made ready each time sound is processed, so a
/// long one comes in over a few blocks rather than holding the sound up.
const PARTS_AT_ONCE: usize = 16;

/// One channel's spectrum, bins `0..BINS`.
#[derive(Clone)]
struct Spectrum {
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Spectrum {
    fn zero() -> Self {
        Spectrum { re: vec![0.0; BINS], im: vec![0.0; BINS] }
    }
}

/// An impulse made ready: its head reversed for summing directly, and
/// each later block's spectrum, for each channel, as far as they are
/// ready; `rest` is the impulse they are still to come from.
struct Prepared {
    head: [Vec<f32>; 2],
    parts: Vec<[Spectrum; 2]>,
    rest: [Vec<f32>; 2],
}

/// What the impulse was made from, to tell when it changes.
#[derive(Clone, Copy, PartialEq)]
enum Source {
    Builtin(usize, u32),
    Sample(usize, u32),
    None,
}

pub(super) struct Convolver {
    fft: Fft,
    source: Source,
    sample: Option<Arc<Sample>>,
    ir: Prepared,
    /// Each channel's last `PART` inputs, twice over, so the newest
    /// `PART` read as one slice ending at `pos + PART`.
    history: [Vec<f32>; 2],
    pos: usize,
    /// The block filling, and the one before it.
    block: [Vec<f32>; 2],
    last_block: [Vec<f32>; 2],
    /// The spectra of past input blocks, newest first.
    past: Vec<[Spectrum; 2]>,
    /// The tail's output for the block filling.
    tail: [Vec<f32>; 2],
    /// Scratch for the FFT.
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Convolver {
    pub(super) fn new() -> Self {
        Convolver {
            fft: Fft::new(SPAN),
            source: Source::None,
            sample: None,
            ir: Prepared { head: [vec![0.0; PART], vec![0.0; PART]], parts: Vec::new(), rest: Default::default() },
            history: [vec![0.0; 2 * PART], vec![0.0; 2 * PART]],
            pos: 0,
            block: [vec![0.0; PART], vec![0.0; PART]],
            last_block: [vec![0.0; PART], vec![0.0; PART]],
            past: Vec::new(),
            tail: [vec![0.0; PART], vec![0.0; PART]],
            re: vec![0.0; SPAN],
            im: vec![0.0; SPAN],
        }
    }

    /// Starts on `ir` (left and right): its head at once, its blocks a
    /// few at a time by `prepare_some`; the sound starts over.
    fn prepare(&mut self, ir: [Vec<f32>; 2]) {
        let len = ir[0].len().max(ir[1].len());
        let parts = len.saturating_sub(PART).div_ceil(PART);
        let mut head = [vec![0.0; PART], vec![0.0; PART]];
        for (h, x) in head.iter_mut().zip(&ir) {
            for (k, v) in x.iter().take(PART).enumerate() {
                h[PART - 1 - k] = *v;
            }
        }
        self.ir = Prepared { head, parts: Vec::with_capacity(parts), rest: ir };
        self.past = vec![[Spectrum::zero(), Spectrum::zero()]; parts];
        self.clear();
    }

    /// Makes up to `most` more of the impulse's blocks ready.
    fn prepare_some(&mut self, most: usize) {
        let total = self.past.len();
        for _ in 0..most {
            let p = self.ir.parts.len() + 1;
            if p > total {
                self.ir.rest = Default::default();
                return;
            }
            let spec: [Spectrum; 2] = std::array::from_fn(|ch| {
                self.re.fill(0.0);
                self.im.fill(0.0);
                for (k, v) in self.ir.rest[ch].iter().skip(p * PART).take(PART).enumerate() {
                    self.re[k] = *v;
                }
                self.fft.forward(&mut self.re, &mut self.im);
                Spectrum { re: self.re[..BINS].to_vec(), im: self.im[..BINS].to_vec() }
            });
            self.ir.parts.push(spec);
        }
    }

    fn clear(&mut self) {
        for ch in 0..2 {
            self.history[ch].fill(0.0);
            self.block[ch].fill(0.0);
            self.last_block[ch].fill(0.0);
            self.tail[ch].fill(0.0);
        }
        for p in &mut self.past {
            for s in p.iter_mut() {
                s.re.fill(0.0);
                s.im.fill(0.0);
            }
        }
        self.pos = 0;
    }

    /// A block has filled: its spectrum joins the past ones, and the tail
    /// for the next block is summed from them.
    fn next_block(&mut self) {
        if self.ir.parts.is_empty() {
            std::mem::swap(&mut self.block, &mut self.last_block);
            return;
        }
        // Both channels in one transform: left real, right imaginary.
        for k in 0..PART {
            (self.re[k], self.im[k]) = (self.last_block[0][k], self.last_block[1][k]);
            (self.re[PART + k], self.im[PART + k]) = (self.block[0][k], self.block[1][k]);
        }
        self.fft.forward(&mut self.re, &mut self.im);
        // The oldest spectrum makes room for the newest, then each channel
        // is parted from the other by the symmetry of a real signal's.
        self.past.rotate_right(1);
        let newest = &mut self.past[0];
        for m in 0..BINS {
            let n = (SPAN - m) % SPAN;
            let (zr, zi, cr, ci) = (self.re[m], self.im[m], self.re[n], -self.im[n]);
            (newest[0].re[m], newest[0].im[m]) = (0.5 * (zr + cr), 0.5 * (zi + ci));
            (newest[1].re[m], newest[1].im[m]) = (0.5 * (zi - ci), -0.5 * (zr - cr));
        }
        // The tail: each past block through the impulse's block as far
        // back, summed for each channel.
        let (mut yl, mut yr) = (Spectrum::zero(), Spectrum::zero());
        for (x, h) in self.past.iter().zip(&self.ir.parts) {
            for (y, ch) in [(&mut yl, 0), (&mut yr, 1)] {
                // Zipped rather than indexed, so it runs four or eight
                // bins at a time.
                let bins = y.re.iter_mut().zip(y.im.iter_mut());
                let x = x[ch].re.iter().zip(&x[ch].im);
                let h = h[ch].re.iter().zip(&h[ch].im);
                for ((yr, yi), ((xr, xi), (hr, hi))) in bins.zip(x.zip(h)) {
                    *yr += xr * hr - xi * hi;
                    *yi += xr * hi + xi * hr;
                }
            }
        }
        // Back together, left real and right imaginary, and back in time.
        for m in 0..BINS {
            (self.re[m], self.im[m]) = (yl.re[m] - yr.im[m], yl.im[m] + yr.re[m]);
            if m > 0 && m < SPAN / 2 {
                let n = SPAN - m;
                (self.re[n], self.im[n]) = (yl.re[m] + yr.im[m], -yl.im[m] + yr.re[m]);
            }
        }
        self.fft.inverse(&mut self.re, &mut self.im);
        self.tail[0].copy_from_slice(&self.re[PART..]);
        self.tail[1].copy_from_slice(&self.im[PART..]);
        std::mem::swap(&mut self.block, &mut self.last_block);
    }

    /// The impulse `source` asks for, at `sr`.
    fn impulse(&self, source: Source, sr: f32) -> [Vec<f32>; 2] {
        match source {
            Source::Builtin(kind, _) => builtin(kind, sr),
            Source::Sample(..) => match &self.sample {
                Some(s) => from_sample(s, sr),
                None => [Vec::new(), Vec::new()],
            },
            Source::None => [Vec::new(), Vec::new()],
        }
    }
}

/// The sum of `a` times `b`, in eight lanes so it vectorizes.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut lanes = [0.0f32; 8];
    for (x, y) in a.as_chunks::<8>().0.iter().zip(b.as_chunks::<8>().0) {
        for k in 0..8 {
            lanes[k] += x[k] * y[k];
        }
    }
    lanes.iter().sum()
}

/// A sample as an impulse at `sr`: both channels, read between frames at
/// the output's rate, at most `MAX_SECONDS` long.
fn from_sample(s: &Sample, sr: f32) -> [Vec<f32>; 2] {
    let step = (s.sample_rate / sr) as f64;
    let len = ((s.frames.len() as f64 / step) as usize).min((MAX_SECONDS * sr) as usize);
    let ir = std::array::from_fn(|ch| {
        (0..len)
            .map(|i| {
                let at = i as f64 * step;
                let k = at as usize;
                let (a, b) = (s.frames[k][ch], s.frames[(k + 1).min(s.frames.len() - 1)][ch]);
                a + (b - a) * (at - k as f64) as f32
            })
            .collect()
    });
    normalized(ir)
}

/// `ir` scaled so its louder channel carries as much energy as a click:
/// white noise through it comes out as loud as it went in.
fn normalized(mut ir: [Vec<f32>; 2]) -> [Vec<f32>; 2] {
    let energy = ir.iter().map(|c| c.iter().map(|v| v * v).sum::<f32>()).fold(0.0, f32::max);
    if energy > 0.0 {
        let scale = energy.sqrt().recip();
        ir.iter_mut().flatten().for_each(|v| *v *= scale);
    }
    ir
}

/// The built-in impulses, in the order of `project::IMPULSES`.
fn builtin(kind: usize, sr: f32) -> [Vec<f32>; 2] {
    let mut rng = Rng(0x9e37_79b9 ^ kind as u32);
    let secs = |s: f32| (s * sr) as usize;
    // Noise dying away by 60 dB over `t60` seconds, darker as it goes,
    // each channel its own, after `pre` seconds of silence.
    let tail = |len: f32, t60: f32, pre: f32, bright: f32, dark: f32, rng: &mut Rng| -> [Vec<f32>; 2] {
        std::array::from_fn(|_| {
            let (mut lp, mut a, mut fall) = (0.0, 0.0, 0.0);
            (0..secs(len))
                .map(|i| {
                    let t = i as f32 / sr - pre;
                    if t < 0.0 {
                        return 0.0;
                    }
                    // The lowpass closes from `bright` to `dark` as it
                    // decays: worked out every 32 samples, as is the level.
                    if i % 32 == 0 {
                        let cutoff = bright * (dark / bright).powf((t / t60).min(1.0));
                        a = (-TAU * cutoff / sr).exp();
                        fall = 10f32.powf(-3.0 * t / t60);
                    }
                    lp = rng.next() * (1.0 - a) + lp * a;
                    lp * fall
                })
                .collect()
        })
    };
    let ir = match kind {
        // A room: early reflections off near walls, then a short tail.
        0 => {
            let mut ir = tail(0.9, 0.7, 0.012, 9000.0, 2500.0, &mut rng);
            for (k, ms) in [3.1f32, 7.3, 11.9, 17.2, 23.5, 31.0].iter().enumerate() {
                let g = 0.9 / (1.0 + k as f32 * 0.6);
                ir[k % 2][secs(ms / 1000.0)] += g;
                ir[(k + 1) % 2][secs(ms * 1.07 / 1000.0)] += g * 0.8;
            }
            ir
        }
        // A hall: a later start, a long, dark tail.
        1 => tail(3.0, 2.6, 0.025, 7000.0, 1200.0, &mut rng),
        // A plate: dense at once, bright, a medium tail.
        2 => tail(2.2, 1.8, 0.0, 14000.0, 5000.0, &mut rng),
        // A spring: chirps falling in pitch, again and again, fading.
        3 => {
            let len = secs(1.6);
            let mut ir = [vec![0.0; len], vec![0.0; len]];
            for echo in 0..40 {
                let start = secs(0.004 + echo as f32 * 0.037);
                let gain = 0.8f32.powi(echo);
                for (ch, chan) in ir.iter_mut().enumerate() {
                    let mut phase = ch as f32 * 0.25;
                    for i in 0..secs(0.03) {
                        let t = i as f32 / secs(0.03) as f32;
                        // From 4 kHz down to 400 Hz over 30 ms.
                        phase += (4000.0 * (0.1f32).powf(t)) / sr;
                        if let Some(v) = chan.get_mut(start + i) {
                            *v += sine(phase) * gain * (1.0 - t) * t.sqrt();
                        }
                    }
                }
            }
            ir
        }
        // A cabinet: a click through a speaker's ring and its roll-offs.
        _ => {
            let len = secs(0.04);
            std::array::from_fn(|ch| {
                let mut ir = vec![0.0; len];
                // Two resonances, low and high mid, and a lowpass past 5 kHz.
                let mut res = [(110.0, 0.0, 0.0, 0.9), (2400.0 + ch as f32 * 150.0, 0.0, 0.0, 0.8)];
                let mut lp = 0.0;
                let a = (-TAU * 5000.0 / sr).exp();
                for (i, v) in ir.iter_mut().enumerate() {
                    let x = if i == 0 { 1.0 } else { 0.0 };
                    let mut y = x * 0.3;
                    for (f, s1, s2, g) in &mut res {
                        let w = TAU * *f / sr;
                        let r = 0.995f32;
                        let out = x + 2.0 * r * w.cos() * *s1 - r * r * *s2;
                        (*s2, *s1) = (*s1, out);
                        y += out * *g * w.sin();
                    }
                    lp = y * (1.0 - a) + lp * a;
                    *v = lp;
                }
                ir
            })
        }
    };
    normalized(ir)
}

impl Dsp for Convolver {
    fn reset(&mut self) {
        self.clear();
    }

    fn set_samples(&mut self, slots: &[SampleSlot]) {
        let new = slots.first().and_then(|s| s.data.clone());
        let same = match (&new, &self.sample) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.sample = new;
            // A different sample: make it ready on the next block.
            if let Source::Sample(..) = self.source {
                self.source = Source::None;
            }
        }
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let kind = (p[0].round() as usize).min(5);
        let (mix, gain) = (p[1], p[2]);
        let sr = ctx.sr as u32;
        let source = if kind == 5 {
            Source::Sample(self.sample.as_ref().map_or(0, |s| Arc::as_ptr(s) as usize), sr)
        } else {
            Source::Builtin(kind, sr)
        };
        if source != self.source {
            let ir = self.impulse(source, ctx.sr);
            self.prepare(ir);
            self.source = source;
        }
        if self.ir.parts.len() < self.past.len() {
            self.prepare_some(PARTS_AT_ONCE);
        }
        for (o, i) in out.iter_mut().zip(input) {
            let k = self.pos;
            let mut wet = [0.0; 2];
            for ch in 0..2 {
                let h = &mut self.history[ch];
                // Written twice, so the newest PART frames are one slice.
                h[k] = i[ch];
                h[k + PART] = i[ch];
                self.block[ch][k] = i[ch];
                wet[ch] = dot(&h[k + 1..k + 1 + PART], &self.ir.head[ch]) + self.tail[ch][k];
            }
            for ch in 0..2 {
                o[ch] = i[ch] * (1.0 - mix) + wet[ch] * gain * mix;
            }
            self.pos += 1;
            if self.pos == PART {
                self.pos = 0;
                self.next_block();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Convolves `x` with `ir` the slow way.
    fn direct(x: &[f32], ir: &[f32]) -> Vec<f32> {
        (0..x.len()).map(|n| (0..ir.len().min(n + 1)).map(|k| x[n - k] * ir[k]).sum()).collect()
    }

    #[test]
    fn it_matches_convolving_directly_with_no_delay() {
        // A made-up impulse three blocks and a bit long, different in each
        // channel, against noise, in uneven blocks.
        let mut rng = Rng(7);
        let ir: [Vec<f32>; 2] = std::array::from_fn(|ch| {
            (0..3 * PART + 77).map(|i| rng.next() * 0.9f32.powi((i / 40 + ch) as i32)).collect()
        });
        let x: Vec<Frame> = (0..6000).map(|_| [rng.next(), rng.next()]).collect();
        let mut c = Convolver::new();
        c.prepare(ir.clone());
        c.prepare_some(usize::MAX);
        c.source = Source::Builtin(0, 48000);
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let params = [0.0, 1.0, 1.0];
        let mut out = vec![[0.0; 2]; x.len()];
        let mut at = 0;
        for n in [100, 512, 3, 900, 2000, 1].iter().cycle() {
            if at >= x.len() {
                break;
            }
            let end = (at + n).min(x.len());
            c.process(&ctx, &params, &x[at..end], &mut out[at..end]);
            at = end;
        }
        for ch in 0..2 {
            let xs: Vec<f32> = x.iter().map(|f| f[ch]).collect();
            let want = direct(&xs, &ir[ch]);
            let worst = out.iter().zip(&want).map(|(o, w)| (o[ch] - w).abs()).fold(0.0, f32::max);
            assert!(worst < 1e-3, "channel {ch}: off by {worst}");
        }
    }

    #[test]
    fn the_built_in_impulses_ring_as_long_as_they_should() {
        for (kind, (shortest, longest)) in
            [(0.3, 1.0), (1.5, 3.1), (1.0, 2.3), (0.5, 1.7), (0.0, 0.05)].iter().enumerate()
        {
            let ir = builtin(kind, 48000.0);
            let energy: f32 = ir.iter().flatten().map(|v| v * v).sum();
            assert!(energy > 0.5 && energy <= 2.0 + 1e-3, "{kind}: energy {energy}");
            // Where it falls below -60 dB of its peak, for good.
            let peak = ir[0].iter().fold(0f32, |m, v| m.max(v.abs()));
            let end = ir[0].iter().rposition(|v| v.abs() > peak * 1e-3).unwrap_or(0) as f32 / 48000.0;
            assert!(end >= *shortest && end <= *longest, "{kind}: rings for {end} s");
        }
    }
}
