//! The effects built on delay lines: Delay, Echo, Multitap Delay, Repeater and Pitch Shifter.

use super::*;

// ---------------------------------------------------------------- delay lines

/// A ring of the last frames written, for the modules built on delays.
pub(super) struct DelayLine {
    pub(super) buf: Vec<Frame>,
    pub(super) pos: usize,
}

impl DelayLine {
    /// Room for `seconds` of sound, and a few frames more.
    pub(super) fn new(sr: f32, seconds: f32) -> Self {
        Self { buf: vec![[0.0; 2]; (sr * seconds) as usize + 4], pos: 0 }
    }

    pub(super) fn len(&self) -> usize {
        self.buf.len()
    }

    pub(super) fn clear(&mut self) {
        self.buf.fill([0.0; 2]);
    }

    /// The frame written `d` frames ago: 1 is the last one.
    pub(super) fn at(&self, d: usize) -> Frame {
        let len = self.buf.len();
        // Wrapped with a comparison: a division would cost more than the
        // rest of a tap.
        let i = self.pos + len - d.clamp(1, len - 1);
        self.buf[if i >= len { i - len } else { i }]
    }

    /// Channel `ch` `d` frames back, between frames.
    pub(super) fn tap_ch(&self, ch: usize, d: f32) -> f32 {
        let d = d.clamp(1.0, (self.buf.len() - 2) as f32);
        let back = d as usize;
        let (a, b) = (self.at(back)[ch], self.at(back + 1)[ch]);
        a + (b - a) * fract(d)
    }

    /// Both channels `d` frames back, between frames.
    pub(super) fn tap(&self, d: f32) -> Frame {
        [self.tap_ch(0, d), self.tap_ch(1, d)]
    }

    pub(super) fn push(&mut self, x: Frame) {
        self.buf[self.pos] = x;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
    }
}

pub(super) struct Delay {
    pub(super) line: DelayLine,
}

impl Delay {
    pub(super) fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 6.0) }
    }
}

impl Dsp for Delay {
    fn reset(&mut self) {
        self.line.clear();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (time, fb, mix, cross) = (p[0], p[1], p[2], p[3]);
        let d = (time * ctx.samples_per_line) as usize;
        for (o, i) in out.iter_mut().zip(input) {
            let rd = self.line.at(d);
            let wl = i[0] + fb * ((1.0 - cross) * rd[0] + cross * rd[1]);
            let wr = i[1] + fb * ((1.0 - cross) * rd[1] + cross * rd[0]);
            self.line.push([wl, wr]);
            *o = [i[0] + rd[0] * mix, i[1] + rd[1] * mix];
        }
    }
}

// ---------------------------------------------------------------- repeater

/// How long the Repeater takes to switch between its input and its loop,
/// and to blend the loop's end into its start, in seconds.
pub(super) const REPEATER_FADE: f32 = 0.003;

/// The Repeater: while Hold is on it plays the last
/// Length of its input over and over, a stutter that follows the tempo.
pub(super) struct Repeater {
    pub(super) buf: Vec<Frame>,
    pub(super) pos: usize,
    /// Where the held loop starts in `buf`, and how far into it the
    /// playback is; `None` while not holding.
    pub(super) held: Option<(usize, usize)>,
    /// How much of the loop is heard, moving towards Hold.
    pub(super) wet: f32,
}

impl Repeater {
    pub(super) fn new(sr: f32) -> Self {
        Self { buf: vec![[0.0; 2]; (sr * 8.0) as usize], pos: 0, held: None, wet: 0.0 }
    }
}

impl Dsp for Repeater {
    fn reset(&mut self) {
        self.buf.fill([0.0; 2]);
        self.held = None;
        self.wet = 0.0;
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let hold = p[0] >= 0.5;
        let lines =
            crate::project::REPEATER_LINES[(p[1].round() as usize).min(crate::project::REPEATER_LINES.len() - 1)];
        let mix = p[2];
        let size = self.buf.len();
        let len = ((lines * ctx.samples_per_line) as usize).clamp(16, size / 2);
        let fade = ((REPEATER_FADE * ctx.sr) as usize).clamp(1, len / 4);
        let step = 1.0 / fade as f32;
        if hold && self.held.is_none() {
            // The loop is the stretch of input just before Hold came on.
            self.held = Some(((self.pos + size - len) % size, 0));
        }
        for (o, i) in out.iter_mut().zip(input) {
            let target = if hold { 1.0 } else { 0.0 };
            self.wet = if self.wet < target { (self.wet + step).min(target) } else { (self.wet - step).max(target) };
            let mut looped = [0.0; 2];
            if let Some((start, at)) = &mut self.held {
                let at_ = *at % len;
                looped = self.buf[(*start + at_) % size];
                // Near its end the loop fades into the audio before its
                // start, so it wraps without a click.
                if at_ + fade >= len {
                    let t = (len - at_) as f32 * step;
                    let before = self.buf[(*start + size - (len - at_)) % size];
                    for ch in 0..2 {
                        looped[ch] = looped[ch] * t + before[ch] * (1.0 - t);
                    }
                }
                *at = at_ + 1;
            } else {
                self.buf[self.pos] = *i;
                self.pos = (self.pos + 1) % size;
            }
            if !hold && self.wet == 0.0 {
                self.held = None;
            }
            let w = self.wet * mix;
            for ch in 0..2 {
                o[ch] = i[ch] * (1.0 - w) + looped[ch] * w;
            }
        }
    }
}

// ---------------------------------------------------------------- multitap delay

/// The Multitap Delay: four echoes, each with its own time in
/// lines, level and pan, the longest fed back for more.
pub(super) struct Multitap {
    pub(super) line: DelayLine,
}

impl Multitap {
    pub(super) fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 6.0) }
    }
}

impl Dsp for Multitap {
    fn reset(&mut self) {
        self.line.clear();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let taps: [(usize, f32, f32, f32); 4] = std::array::from_fn(|t| {
            let d = (p[t * 3] * ctx.samples_per_line) as usize;
            let (l, r) = pan_gains(p[t * 3 + 2]);
            (d, p[t * 3 + 1], l, r)
        });
        let longest = taps.iter().map(|t| t.0).max().unwrap_or(1);
        let (fb, mix) = (p[12], p[13]);
        for (o, i) in out.iter_mut().zip(input) {
            let mut wet = [0.0; 2];
            for &(d, level, l, r) in &taps {
                let x = self.line.at(d);
                // Each tap is panned from the middle of what it hears.
                let m = (x[0] + x[1]) * 0.5 * level;
                wet = [wet[0] + m * l, wet[1] + m * r];
            }
            let back = self.line.at(longest);
            self.line.push([i[0] + back[0] * fb, i[1] + back[1] * fb]);
            *o = [i[0] + wet[0] * mix, i[1] + wet[1] * mix];
        }
    }
}

// ---------------------------------------------------------------- echo

/// The Echo: a delay timed in seconds rather than lines, darker each
/// time round with Damping, the right channel's echoes up to half again as
/// late with Stereo.
pub(super) struct Echo {
    pub(super) line: DelayLine,
    pub(super) lp: Frame,
}

impl Echo {
    pub(super) fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 3.0), lp: [0.0; 2] }
    }
}

impl Dsp for Echo {
    fn reset(&mut self) {
        self.line.clear();
        self.lp = [0.0; 2];
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let (fb, damp, mix) = (p[1], p[2] * 0.9, p[4]);
        let times = [p[0] * ctx.sr, p[0] * ctx.sr * (1.0 + 0.5 * p[3])];
        for (o, i) in out.iter_mut().zip(input) {
            let mut back = [0.0; 2];
            for ch in 0..2 {
                let echo = self.line.tap_ch(ch, times[ch]);
                self.lp[ch] = echo * (1.0 - damp) + self.lp[ch] * damp;
                back[ch] = i[ch] + self.lp[ch] * fb;
                o[ch] = i[ch] + echo * mix;
            }
            self.line.push(back);
        }
    }
}

// ---------------------------------------------------------------- pitch shifter

/// The Pitch Shifter: two read heads sweep through a short delay
/// faster or slower than it fills, each fading in and out over a grain so
/// one is always loud while the other jumps back. Feedback sends the
/// shifted sound round again, for rising or falling cascades.
pub(super) struct PitchShifter {
    pub(super) line: DelayLine,
    /// Where the first head is in its grain, 0..1; the second is half a
    /// grain on.
    pub(super) phase: f32,
}

impl PitchShifter {
    pub(super) fn new(sr: f32) -> Self {
        Self { line: DelayLine::new(sr, 0.25), phase: 0.0 }
    }
}

impl Dsp for PitchShifter {
    fn reset(&mut self) {
        self.line.clear();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], input: &[Frame], out: &mut [Frame]) {
        let ratio = 2f32.powf((p[0] + p[1] / 100.0) / 12.0);
        let grain = (p[2] * ctx.sr).min(self.line.len() as f32 - 4.0);
        let (fb, mix) = (p[3], p[4]);
        // The delay shrinks by ratio - 1 frames a frame, so the heads read
        // `ratio` frames a frame.
        let step = (1.0 - ratio) / grain;
        for (o, i) in out.iter_mut().zip(input) {
            let mut wet = [0.0; 2];
            for head in [0.0, 0.5] {
                let ph = fract(self.phase + head);
                // sin² windows half a grain apart add up to one.
                let w = (PI * ph).sin().powi(2);
                let x = self.line.tap(1.0 + ph * grain);
                wet = [wet[0] + x[0] * w, wet[1] + x[1] * w];
            }
            self.line.push([i[0] + wet[0] * fb, i[1] + wet[1] * fb]);
            self.phase = (self.phase + step).rem_euclid(1.0);
            for ch in 0..2 {
                o[ch] = i[ch] * (1.0 - mix) + wet[ch] * mix;
            }
        }
    }
}
