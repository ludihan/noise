//! The output, the Amplifier, the Stereo Expander and the sound card Input.

use super::*;

// ---------------------------------------------------------------- output / amp

pub(super) struct Output;

impl Dsp for Output {
    fn process(&mut self, _: &Ctx, params: &[f32], input: &[Frame], out: &mut [Frame]) {
        let vol = params[0];
        for (o, i) in out.iter_mut().zip(input) {
            *o = [i[0] * vol, i[1] * vol];
        }
    }
}

/// Modules that make no sound of their own; the engine routes their notes.
pub(super) struct Silent;

impl Dsp for Silent {
    fn process(&mut self, _: &Ctx, _: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
    }
}

pub(super) struct Amplifier;

impl Dsp for Amplifier {
    fn process(&mut self, _: &Ctx, params: &[f32], input: &[Frame], out: &mut [Frame]) {
        let sign = if params[2] >= 0.5 { -1.0 } else { 1.0 };
        let (l, r) = pan_gains(params[1]);
        let vol = params[0] * sign;
        for (o, i) in out.iter_mut().zip(input) {
            *o = [i[0] * vol * l, i[1] * vol * r];
        }
    }
}

// ---------------------------------------------------------------- stereo expander

/// The Stereo Expander: the difference between the channels (the
/// side) scaled by Width, from mono at 0 through unchanged at 100% to
/// twice as wide, with the side below Mono bass taken out so the low end
/// stays centred.
#[derive(Default)]
pub(super) struct StereoExpander {
    /// The side's lows, for Mono bass.
    pub(super) low: f32,
}

impl Dsp for StereoExpander {
    fn reset(&mut self) {
        self.low = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let width = p[0];
        let coef = 1.0 - (-TAU * p[1] / ctx.sr).exp();
        for (o, i) in out.iter_mut().zip(input) {
            let mid = (i[0] + i[1]) * 0.5;
            let side = (i[0] - i[1]) * 0.5;
            self.low += coef * (side - self.low);
            let side = (side - self.low) * width;
            *o = [mid + side, mid - side];
        }
    }
}

// ---------------------------------------------------------------- input

/// Frames of input an Input keeps on hand, and lets wait at most.
pub(super) const INPUT_HELD: usize = 2048;

/// The Input: the sound card's input, played into the graph. The UI
/// opens the input while the song has one and it arrives on a tape; at a
/// rate other than the output's it is resampled.
pub struct LiveInput {
    tape: Arc<Tape>,
    held: Vec<Frame>,
    /// Where in `held` the next output frame reads, between frames.
    pos: f64,
}

impl LiveInput {
    pub fn new(tape: Arc<Tape>) -> Self {
        Self { tape, held: Vec::with_capacity(INPUT_HELD), pos: 0.0 }
    }
}

impl Dsp for LiveInput {
    fn reset(&mut self) {
        self.held.clear();
        self.pos = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        let rate = self.tape.rate();
        let step = if rate == 0 { 1.0 } else { rate as f64 / ctx.sr as f64 };
        // Top up what is held from the tape, without growing past its room.
        let have = self.held.len();
        self.held.resize(INPUT_HELD, [0.0; 2]);
        let got = self.tape.read(&mut self.held[have..], INPUT_HELD / 2);
        self.held.truncate(have + got);
        let (vol, channels) = (p[0], p[1].round() as u32);
        for o in out.iter_mut() {
            let i = self.pos as usize;
            // Waiting for input plays silence rather than old sound.
            if i + 1 >= self.held.len() {
                *o = [0.0; 2];
                continue;
            }
            let (a, b, t) = (self.held[i], self.held[i + 1], self.pos.fract() as f32);
            let x = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let x = match channels {
                0 => x,
                1 => [x[0]; 2],
                2 => [x[1]; 2],
                _ => [(x[0] + x[1]) * 0.5; 2],
            };
            *o = [x[0] * vol, x[1] * vol];
            self.pos += step;
        }
        let used = (self.pos as usize).min(self.held.len());
        self.held.drain(..used);
        self.pos -= used as f64;
    }
}
