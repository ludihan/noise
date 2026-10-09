//! A radix-2 FFT with its twiddle factors worked out once, for the
//! spectrum view, the wavetables and the convolver.

use std::f32::consts::TAU;

/// An FFT of one power-of-two length.
pub struct Fft {
    /// The cosine and sine of each turn the butterflies use.
    twiddles: Vec<(f32, f32)>,
    /// Where each index goes in bit-reversed order.
    reversed: Vec<u32>,
}

impl Fft {
    pub fn new(n: usize) -> Fft {
        assert!(n.is_power_of_two(), "{n}");
        let bits = n.trailing_zeros();
        let reversed = (0..n as u32).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (32 - bits) }).collect();
        let twiddles = (0..n / 2).map(|k| (-TAU * k as f32 / n as f32).sin_cos()).map(|(s, c)| (c, s)).collect();
        Fft { twiddles, reversed }
    }

    pub fn len(&self) -> usize {
        self.reversed.len()
    }

    /// Transforms `re` and `im` in place, from time to frequency.
    pub fn forward(&self, re: &mut [f32], im: &mut [f32]) {
        let n = self.len();
        for i in 0..n {
            let j = self.reversed[i] as usize;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let (half, stride) = (len / 2, n / len);
            for start in (0..n).step_by(len) {
                for k in 0..half {
                    let (wr, wi) = self.twiddles[k * stride];
                    let (a, b) = (start + k, start + k + half);
                    let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
                    (re[b], im[b]) = (re[a] - tr, im[a] - ti);
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len <<= 1;
        }
    }

    /// Transforms back, from frequency to time, scaled so `forward` then
    /// `inverse` gives the input again.
    pub fn inverse(&self, re: &mut [f32], im: &mut [f32]) {
        // The inverse is the forward transform of the conjugate, conjugated.
        im.iter_mut().for_each(|x| *x = -*x);
        self.forward(re, im);
        let scale = 1.0 / self.len() as f32;
        re.iter_mut().for_each(|x| *x *= scale);
        im.iter_mut().for_each(|x| *x *= -scale);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sine_lands_in_its_bin_and_comes_back() {
        let n = 64;
        let fft = Fft::new(n);
        let x: Vec<f32> = (0..n).map(|i| (TAU * 5.0 * i as f32 / n as f32).cos()).collect();
        let (mut re, mut im) = (x.clone(), vec![0.0; n]);
        fft.forward(&mut re, &mut im);
        let mags: Vec<f32> = re.iter().zip(&im).map(|(r, i)| r.hypot(*i)).collect();
        assert!((mags[5] - 32.0).abs() < 1e-3 && (mags[59] - 32.0).abs() < 1e-3, "{mags:?}");
        assert!(mags.iter().enumerate().filter(|(k, _)| ![5, 59].contains(k)).all(|(_, m)| *m < 1e-3));
        fft.inverse(&mut re, &mut im);
        assert!(re.iter().zip(&x).all(|(a, b)| (a - b).abs() < 1e-5) && im.iter().all(|v| v.abs() < 1e-5));
    }
}
