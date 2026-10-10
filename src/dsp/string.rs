//! The Plucked String: a burst of noise sent round a delay as long as one
//! cycle of the note, losing a little and its highs on every trip, as a
//! plucked string does (after Karplus and Strong). Up to three strings a
//! voice, detuned against each other as a twelve-string's courses are.

use super::*;

/// The longest delay a string has, in frames: about 12 Hz at 48 kHz.
const LINE: usize = 4096;
const MAX_STRINGS: usize = 3;

/// One string of a voice: its delay, where it writes, and its damping
/// filter's state.
struct Line {
    buf: Box<[f32]>,
    at: usize,
    damp: f32,
}

pub(super) struct StringVoice {
    pub(super) slot: VoiceSlot,
    /// The gate: open while the note is held, closing over the release.
    pub(super) env: Adsr,
    lines: [Line; MAX_STRINGS],
}

pub(super) struct PluckedString {
    pub(super) voices: [StringVoice; MAX_VOICES],
    pub(super) clock: u64,
    pub(super) rng: Rng,
    /// The sample rate, pluck position, brightness, strings and detune of
    /// the last block, for plucking: `note_on` has no parameters.
    pub(super) sr: f32,
    pub(super) pluck: [f32; 4],
    pub(super) tap: TrackTap,
}

impl PluckedString {
    pub(super) fn new() -> Self {
        let line = || Line { buf: vec![0.0; LINE].into_boxed_slice(), at: 0, damp: 0.0 };
        Self {
            voices: std::array::from_fn(|_| StringVoice {
                slot: VoiceSlot::default(),
                env: Adsr::default(),
                lines: std::array::from_fn(|_| line()),
            }),
            clock: 0,
            rng: Rng(0x5eed_1234),
            sr: 48000.0,
            pluck: [0.15, 0.7, 1.0, 6.0],
            tap: TrackTap::new(),
        }
    }
}

/// String `s` of `n`'s pitch around the note, `detune` cents at most
/// either side.
fn spread(s: usize, n: usize, detune: f32) -> f32 {
    if n > 1 { (s as f32 / (n - 1) as f32 - 0.5) * 2.0 * detune / 100.0 } else { 0.0 }
}

impl Dsp for PluckedString {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        let v = &mut self.voices[i];
        v.slot = VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false };
        v.env.trigger();
        let [position, brightness, strings, detune] = self.pluck;
        let n = (strings.round() as usize).clamp(1, MAX_STRINGS);
        // Softer notes are plucked darker as well as quieter.
        let smooth = (0.03 + 0.97 * brightness * (0.4 + 0.6 * vel)).clamp(0.01, 1.0);
        for (s, line) in v.lines.iter_mut().enumerate().take(n) {
            let period = (self.sr / note_to_freq(note + spread(s, n, detune))).clamp(2.0, (LINE - 2) as f32) as usize;
            // The cycle behind where the string writes next, in place.
            let start = line.at + LINE - period;
            let at = |k: usize| (start + k) % LINE;
            // Noise, smoothed by the brightness, then less the same a
            // pluck's width later: plucking away from the end of a string
            // leaves out the harmonics with a node there.
            let mut lp = 0.0;
            for k in 0..period {
                lp += smooth * (self.rng.next() - lp);
                line.buf[at(k)] = lp;
            }
            let gap = ((position * period as f32) as usize).max(1);
            for k in (gap..period).rev() {
                line.buf[at(k)] -= line.buf[at(k - gap)];
            }
            // No offset, and as loud whatever the brightness.
            let mean = (0..period).map(|k| line.buf[at(k)]).sum::<f32>() / period as f32;
            let peak = (0..period).fold(0f32, |m, k| m.max((line.buf[at(k)] - mean).abs())).max(1e-6);
            for k in 0..period {
                line.buf[at(k)] = (line.buf[at(k)] - mean) / peak;
            }
            line.damp = 0.0;
        }
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let sr = ctx.sr;
        let (vol, decay, release, detune) = (p[0], p[3], p[5], p[7]);
        let n = (p[6].round() as usize).clamp(1, MAX_STRINGS);
        (self.sr, self.pluck) = (sr, [p[1], p[2], p[6], detune]);
        // The damping filter, and the frames of delay it adds at low notes,
        // which the delay leaves out so the string stays in tune.
        let a = p[4];
        let lag = a / (1.0 - a);
        let norm = 1.0 / (n as f32).sqrt();
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let (pl, pr) = pan_gains(p[8] + v.slot.pan);
            let mut strings = [(0.0, 1.0); MAX_STRINGS];
            for (s, st) in strings.iter_mut().enumerate().take(n) {
                let f = note_to_freq(v.slot.note + spread(s, n, detune));
                // Each trip round loses what fades it by 60 dB over the decay.
                *st = (((sr / f) - lag).clamp(1.0, (LINE - 2) as f32), 0.001f32.powf(1.0 / (decay * f)));
            }
            let mut loudest = 0f32;
            for (i, o) in out.iter_mut().enumerate() {
                let mut x = 0.0;
                for (line, &(delay, gain)) in v.lines.iter_mut().zip(&strings).take(n) {
                    let back = line.at as f32 + LINE as f32 - delay;
                    let k = back as usize;
                    let frac = back - k as f32;
                    let (y0, y1) = (line.buf[k % LINE], line.buf[(k + 1) % LINE]);
                    let y = y0 + (y1 - y0) * frac;
                    line.damp = (1.0 - a) * y + a * line.damp;
                    line.buf[line.at] = line.damp * gain;
                    line.at = (line.at + 1) % LINE;
                    x += y;
                }
                let g = v.env.next(sr, 0.0, 0.0, 1.0, release) * v.slot.vel * vol * norm;
                let y = [x * g * pl, x * g * pr];
                loudest = loudest.max(x.abs());
                o[0] += y[0];
                o[1] += y[1];
                self.tap.add(v.slot.key, i, y);
            }
            // A string that has rung out frees its voice.
            if loudest < 1e-4 {
                v.env = Adsr::default();
            }
        }
    }
}
