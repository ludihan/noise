//! Sounds the demo songs render for themselves, so their Samplers have
//! samples without audio files: drum hits, a felt piano, and the noise and
//! levels they are made with.

use crate::dsp::Frame;
use crate::sample::Sample;
use std::f32::consts::TAU;

pub(super) use crate::rng::Rng;

/// Scales `frames` so their peak is `peak`.
pub(super) fn normalize(frames: &mut [Frame], peak: f32) {
    let max = frames.iter().fold(0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()));
    if max > 0.0 {
        frames.iter_mut().for_each(|f| *f = [f[0] * peak / max, f[1] * peak / max]);
    }
}

/// A felt piano's C-4 (MIDI 48): two strings a hair apart, each a stack
/// of slightly stretched harmonics that die away faster the higher they
/// are, with the soft knock of the hammer. `bright` is a harder strike.
pub(super) fn felt_piano(sr: f32, bright: bool) -> Sample {
    let f0 = 440.0 * 2f32.powf((48.0 - 69.0) / 12.0);
    let len = (sr * 3.5) as usize;
    let mut frames = vec![[0.0f32; 2]; len];
    let (tilt, damp) = if bright { (1.15, 0.55) } else { (1.9, 0.8) };
    for h in 1..=18 {
        let hf = h as f32;
        let f = hf * f0 * (1.0 + 0.0004 * hf * hf).sqrt();
        if f > sr * 0.45 {
            break;
        }
        let amp = hf.powf(-tilt) * if h > 9 { 0.5 } else { 1.0 };
        let decay = damp * (0.6 + 0.45 * hf);
        // The two strings beat slowly against each other, and sit a little
        // apart in the stereo field.
        for (string, cents, pan) in [(0, -0.9f32, 0.65f32), (1, 0.9, 0.35)] {
            let w = std::f32::consts::TAU * f * 2f32.powf(cents / 1200.0) / sr;
            let phase = (string * h) as f32 * 0.7;
            for (k, fr) in frames.iter_mut().enumerate() {
                let t = k as f32 / sr;
                let v = amp * (-decay * t).exp() * (w * k as f32 + phase).sin();
                fr[0] += v * (1.0 - pan);
                fr[1] += v * pan;
            }
        }
    }
    // The hammer: a short, muffled knock.
    let mut rng = Rng(0x9E37_79B9);
    let (mut lp, knock) = (0.0f32, if bright { 0.25 } else { 0.1 });
    for (k, fr) in frames.iter_mut().take((sr * 0.03) as usize).enumerate() {
        lp += 0.15 * (rng.next() - lp);
        let v = lp * knock * (-(k as f32) / (sr * 0.006)).exp();
        fr[0] += v;
        fr[1] += v;
    }
    // A soft onset, felt rather than struck, and a gentle end.
    let attack = (sr * if bright { 0.002 } else { 0.006 }) as usize;
    let fade = (sr * 0.3) as usize;
    for (k, f) in frames.iter_mut().take(attack).enumerate() {
        let g = k as f32 / attack as f32;
        *f = [f[0] * g, f[1] * g];
    }
    for (k, f) in frames.iter_mut().rev().take(fade).enumerate() {
        let g = k as f32 / fade as f32;
        *f = [f[0] * g, f[1] * g];
    }
    normalize(&mut frames, 0.8);
    let name = if bright { "Felt Piano (hard)" } else { "Felt Piano (soft)" };
    Sample { name: name.into(), sample_rate: sr, channels: 2, frames }
}

/// What a drum hit is.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Hit {
    Kick,
    Snare,
    Ghost,
    Rim,
    Hat,
    Open,
    Crash,
    Clap,
    Ride,
}

use Hit::*;

/// One hit, mono, until it dies away; `room` is how long a snare's room
/// rings on, in seconds.
pub(super) fn hit(sr: f32, kind: Hit, seed: u32, room: f32) -> Vec<f32> {
    let secs = match kind {
        Kick => 0.4,
        Snare | Ghost => 0.3 + 3.0 * room,
        Rim => 0.08,
        Hat => 0.15,
        Open => 0.6,
        Crash => 2.0,
        Clap => 0.5,
        Ride => 1.2,
    };
    let mut noise = Rng(seed);
    let (mut phase, mut lp, mut hp) = (0.0f32, 0.0f32, [0.0f32; 2]);
    (0..(sr * secs) as usize)
        .map(|k| {
            let t = k as f32 / sr;
            let w = noise.next();
            match kind {
                Kick => {
                    phase += (50.0 + 140.0 * (-t / 0.03).exp()) / sr;
                    (TAU * phase).sin() * (-t / 0.16).exp() + w * 0.35 * (-t / 0.0015).exp()
                }
                Snare | Ghost => {
                    // A drum's ring, the snares' rattle (the noise less its
                    // lows) and the room (its lows).
                    lp += 0.4 * (w - lp);
                    let (snap, body) = if kind == Ghost { (0.05, 0.5) } else { (0.12, 1.0) };
                    let tone = (TAU * 185.0 * t).sin() * (-t / 0.05).exp() * 0.6
                        + (TAU * 330.0 * t).sin() * (-t / 0.03).exp() * 0.25;
                    tone * body + (w - lp) * (-t / snap).exp() + lp * 0.35 * (-t / room).exp()
                }
                Rim => {
                    (TAU * 820.0 * t).sin() * (-t / 0.012).exp()
                        + (TAU * 1650.0 * t).sin() * 0.5 * (-t / 0.007).exp()
                        + w * 0.5 * (-t / 0.003).exp()
                }
                Hat | Open | Crash | Ride => {
                    // Noise through two highpasses, thin and metallic; the
                    // crash rings with a few partials besides, and the
                    // ride's bell louder.
                    let h1 = w - hp[0];
                    hp[0] += 0.6 * h1;
                    let h2 = h1 - hp[1];
                    hp[1] += 0.6 * h2;
                    let decay = match kind {
                        Hat => 0.022,
                        Open => 0.17,
                        Ride => 0.35,
                        _ => 0.7,
                    };
                    let partials = || [3150.0, 4423.0, 5210.0, 6830.0].iter().map(|f| (TAU * f * t).sin()).sum::<f32>();
                    let ring = match kind {
                        Crash => partials() * 0.08,
                        Ride => partials() * 0.2,
                        _ => 0.0,
                    };
                    (h2 * 0.9 + ring) * (-t / decay).exp()
                }
                Clap => {
                    // Three claps a few milliseconds apart, then the room:
                    // noise between a highpass and a lowpass.
                    let h = w - hp[0];
                    hp[0] += 0.25 * h;
                    lp += 0.55 * (h - lp);
                    let claps: f32 =
                        [0.0, 0.011, 0.023].iter().filter(|&&s| t >= s).map(|&s| (-(t - s) / 0.005).exp()).sum();
                    let tail = if t >= 0.023 { 0.45 * (-(t - 0.023) / 0.11).exp() } else { 0.0 };
                    lp * 1.6 * (claps * 0.7 + tail)
                }
            }
        })
        .collect()
}

