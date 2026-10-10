//! The Granular synth: a note plays its sample as a cloud of short grains,
//! each a moment of it read at the note's pitch and faded in and out,
//! started Density times a second around a point that Scan moves through
//! the sample, scattered by Spray.

use super::*;

/// Grains a voice plays at once, at most; more are skipped.
const MAX_GRAINS: usize = 32;

#[derive(Clone, Copy, Default)]
struct Grain {
    /// Where it reads in the sample, and how far it moves a sample.
    pos: f64,
    step: f64,
    /// Samples played of its length; done once `age` reaches `len`.
    age: f32,
    len: f32,
    pl: f32,
    pr: f32,
}

#[derive(Clone, Copy, Default)]
struct GrainVoice {
    slot: VoiceSlot,
    env: Adsr,
    /// The sample it plays, as an index into `Granular::sources`.
    source: usize,
    /// The point grains start around, in sample frames.
    head: f64,
    /// Output samples until the next grain.
    wait: f32,
    grains: [Grain; MAX_GRAINS],
}

/// A sample slot's audio with the settings grains use.
struct Source {
    data: Arc<Sample>,
    /// Semitones from the note to the sample's own pitch.
    tune: f32,
    gain: f32,
    pan: f32,
    keys: [u8; 2],
    velocities: [u8; 2],
}

pub(super) struct Granular {
    sources: Vec<Source>,
    voices: [GrainVoice; MAX_VOICES],
    clock: u64,
    rng: Rng,
    tap: TrackTap,
}

impl Granular {
    pub(super) fn new() -> Self {
        Self {
            sources: Vec::new(),
            voices: [GrainVoice::default(); MAX_VOICES],
            clock: 0,
            rng: Rng(0x6b43_a9b5),
            tap: TrackTap::new(),
        }
    }
}

impl Dsp for Granular {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn set_samples(&mut self, slots: &[SampleSlot]) {
        self.sources = slots
            .iter()
            .filter_map(|s| {
                Some(Source {
                    data: s.data.clone()?,
                    tune: s.transpose as f32 + s.finetune as f32 / 100.0 - s.base_note as f32,
                    gain: s.volume,
                    pan: s.panning,
                    keys: s.keys,
                    velocities: s.velocities,
                })
            })
            .collect();
        // A voice whose sample went stops.
        let n = self.sources.len();
        for v in self.voices.iter_mut().filter(|v| v.source >= n) {
            v.env = Adsr::default();
        }
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        // The first sample whose keyzone holds the note, or the first.
        let (n, v) = (note.round().clamp(0.0, 127.0) as u8, (vel * 127.0).round().clamp(0.0, 127.0) as u8);
        let fits =
            |s: &Source| (s.keys[0]..=s.keys[1]).contains(&n) && (s.velocities[0]..=s.velocities[1]).contains(&v);
        let Some(source) = self.sources.iter().position(fits).or((!self.sources.is_empty()).then_some(0)) else {
            return;
        };
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        self.voices[i] =
            GrainVoice { slot: VoiceSlot::new(key, note, vel, self.clock), source, head: -1.0, ..Default::default() };
        self.voices[i].env.trigger();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, position, scan, size, density) = (p[0], p[1], p[2], p[3], p[4].max(0.1));
        let (spray, cents, stereo, reverse) = (p[5], p[6], p[7], p[8]);
        let (attack, release) = (p[9], p[10]);
        // Grains overlap Density × Size deep; heard as uncorrelated sounds.
        let norm = 1.0 / (density * size).max(1.0).sqrt();
        let len = (size * ctx.sr).max(16.0);
        let rng = &mut self.rng;
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let Some(src) = self.sources.get(v.source) else { continue };
            let frames = &src.data.frames[..];
            let span = frames.len() as f64;
            if span < 4.0 {
                continue;
            }
            if v.head < 0.0 {
                v.head = position as f64 * (span - 1.0);
            }
            // The sample's rate against the output's, and the note's pitch.
            let rate = (src.data.sample_rate / ctx.sr) as f64;
            let pitch = 2f64.powf((v.slot.note + src.tune) as f64 / 12.0) * rate;
            let (pl, pr) = pan_gains(src.pan + v.slot.pan);
            let gain = vol * src.gain * v.slot.vel * norm;
            for (i, o) in out.iter_mut().enumerate() {
                let env = v.env.next(ctx.sr, attack, 0.0, 1.0, release);
                v.head = (v.head + scan as f64 * rate).rem_euclid(span);
                v.wait -= 1.0;
                if v.wait <= 0.0 {
                    // A little irregular, so grains don't buzz at Density.
                    v.wait += ctx.sr / density * (1.0 + 0.25 * rng.next());
                    if let Some(g) = v.grains.iter_mut().find(|g| g.age >= g.len) {
                        let at = v.head + (spray * src.data.sample_rate * rng.next()) as f64;
                        let step = pitch * 2f64.powf((cents * rng.next()) as f64 / 1200.0);
                        let back = (rng.next() * 0.5 + 0.5) < reverse;
                        let (gl, gr) = pan_gains(stereo * rng.next());
                        *g = Grain {
                            pos: at.rem_euclid(span),
                            step: if back { -step } else { step },
                            age: 0.0,
                            len,
                            pl: gl,
                            pr: gr,
                        };
                    }
                }
                let mut y = [0.0f32; 2];
                for g in v.grains.iter_mut().filter(|g| g.age < g.len) {
                    // A Hann window: in and out without a click.
                    let w = 0.5 - 0.5 * sine(g.age / g.len + 0.25);
                    let x = hermite(frames, g.pos);
                    y[0] += x[0] * w * g.pl;
                    y[1] += x[1] * w * g.pr;
                    g.age += 1.0;
                    g.pos = (g.pos + g.step).rem_euclid(span - 1.0);
                }
                let g = env * gain;
                let y = [y[0] * g * pl, y[1] * g * pr];
                o[0] += y[0];
                o[1] += y[1];
                self.tap.add(v.slot.key, i, y);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second of a 440 Hz sine at 48 kHz on C-4.
    fn sine_slot() -> SampleSlot {
        let frames = (0..48000).map(|i| [(TAU * 440.0 * i as f32 / 48000.0).sin() * 0.5; 2]).collect();
        let sample = Sample { name: "sine".into(), sample_rate: 48000.0, channels: 1, frames };
        let mut slot = SampleSlot::new(sample, None);
        slot.base_note = 48;
        slot
    }

    fn params(scan: f32, density: f32) -> [f32; 11] {
        [1.0, 0.2, scan, 0.05, density, 0.0, 0.0, 0.0, 0.0, 0.005, 0.1]
    }

    /// The strongest frequency in `out`, by counting rising zero crossings.
    fn pitch(out: &[Frame]) -> f32 {
        let ups = out.windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count();
        ups as f32 * 48000.0 / out.len() as f32
    }

    #[test]
    fn grains_play_the_sample_at_the_note_s_pitch_and_keep_sounding() {
        let mut g = Granular::new();
        g.set_samples(&[sine_slot()]);
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        g.note_on(0, 60.0, 1.0);
        let mut out = vec![[0.0; 2]; 48000];
        for block in out.chunks_mut(512) {
            g.process(&ctx, &params(0.0, 40.0), &[], block);
        }
        // An octave up, and still sounding at the end of a second of a
        // frozen position, as a pad does.
        let hz = pitch(&out[4800..]);
        assert!((hz - 880.0).abs() < 30.0, "{hz}");
        let tail = out[40000..].iter().fold(0f32, |m, f| m.max(f[0].abs()));
        assert!(tail > 0.1 && tail < 2.0, "{tail}");
    }

    #[test]
    fn without_a_sample_it_is_silent() {
        let mut g = Granular::new();
        g.note_on(0, 60.0, 1.0);
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let mut out = vec![[1.0; 2]; 256];
        g.process(&ctx, &params(1.0, 40.0), &[], &mut out);
        assert!(out.iter().all(|f| f[0] == 0.0));
    }
}
