//! The Wavetable synth: a table of waves that Position moves through, each
//! read at a band limit that keeps the note's harmonics below half the
//! sample rate, with unison voices spread in pitch and across the stereo
//! field.

use super::fft::Fft;
use super::*;
use std::sync::LazyLock;

/// Points in each wave, and one more: the first again, so reading between
/// the last two needs no wrap.
const WAVE_LEN: usize = 2048;
/// Waves in a table, which Position moves through.
const FRAMES: usize = 16;
/// Copies of each wave with half as many harmonics each: the first keeps
/// 1024, the last 1.
const LEVELS: usize = 11;
/// The most unison voices a note plays.
const MAX_UNISON: usize = 7;

/// A table: `FRAMES` waves at `LEVELS` band limits, `WAVE_LEN + 1` points
/// each, frame by frame.
struct Table(Vec<f32>);

impl Table {
    fn wave(&self, frame: usize, level: usize) -> &[f32] {
        let at = (frame * LEVELS + level) * (WAVE_LEN + 1);
        &self.0[at..at + WAVE_LEN + 1]
    }
}

/// The tables, in the order of `project::WAVETABLES`, made the first time
/// they are needed: `warm_up` makes them before the sound starts.
static TABLES: LazyLock<Vec<Table>> = LazyLock::new(|| (0..5).map(build_table).collect());

/// Makes the tables now, off the audio thread.
pub fn warm_up() {
    LazyLock::force(&TABLES);
}

/// One cycle of frame `t` (0..1) of table `table`, at phase `x` (0..1).
fn shape(table: usize, t: f32, x: f32) -> f32 {
    let saw = 2.0 * x - 1.0;
    match table {
        // Sine, triangle, saw, square, one into the next.
        0 => {
            let shapes = [sine(x), 1.0 - 4.0 * (x - 0.5).abs(), saw, if x < 0.5 { 1.0 } else { -1.0 }];
            let at = t * 3.0;
            let i = (at as usize).min(2);
            let f = at - i as f32;
            shapes[i] * (1.0 - f) + shapes[i + 1] * f
        }
        // A pulse narrowing from square to a sliver.
        1 => {
            let width = 0.5 - 0.47 * t;
            if x < width { 1.0 } else { -1.0 }
        }
        // A saw synced to a cycle one to eight times its own.
        2 => 2.0 * fract(x * (1.0 + 7.0 * t)) - 1.0,
        // A sine folded back on itself, harder and harder.
        3 => sine(0.25 * (1.0 + 6.0 * t) * sine(x)),
        // Harmonics of 110 Hz through the formants of A, E, I, O and U.
        _ => {
            let at = t * 4.0;
            let (k, f) = ((at as usize).min(3), at - (at as usize).min(3) as f32);
            (1..=48)
                .map(|h| {
                    let hz = 110.0 * h as f32;
                    let level: f32 = (0..3)
                        .map(|n| {
                            let (a, b) = (VOWELS[k][n], VOWELS[k + 1][n]);
                            let (freq, db, bw) =
                                (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f, (a.2 + (b.2 - a.2) * f) * 1.5);
                            10f32.powf(db / 20.0) / (1.0 + ((hz - freq) / bw).powi(2))
                        })
                        .sum();
                    level * sine(h as f32 * x)
                })
                .sum()
        }
    }
}

/// Table `table`: each frame drawn, then taken to its harmonics and back
/// once for each band limit, and scaled so its fullest version peaks at 1.
fn build_table(table: usize) -> Table {
    let fft = Fft::new(WAVE_LEN);
    let mut data = vec![0.0; FRAMES * LEVELS * (WAVE_LEN + 1)];
    let (mut re, mut im) = (vec![0.0; WAVE_LEN], vec![0.0; WAVE_LEN]);
    for frame in 0..FRAMES {
        let t = frame as f32 / (FRAMES - 1) as f32;
        let wave: Vec<f32> = (0..WAVE_LEN).map(|i| shape(table, t, i as f32 / WAVE_LEN as f32)).collect();
        let (mut spec_re, mut spec_im) = (wave, vec![0.0; WAVE_LEN]);
        fft.forward(&mut spec_re, &mut spec_im);
        // No offset: a wave centered on zero.
        (spec_re[0], spec_im[0]) = (0.0, 0.0);
        let mut scale = 1.0;
        for level in 0..LEVELS {
            let harmonics = (WAVE_LEN / 2) >> level;
            for k in 0..WAVE_LEN {
                let h = k.min(WAVE_LEN - k);
                let keep = h <= harmonics && h < WAVE_LEN / 2;
                re[k] = if keep { spec_re[k] } else { 0.0 };
                im[k] = if keep { spec_im[k] } else { 0.0 };
            }
            fft.inverse(&mut re, &mut im);
            if level == 0 {
                scale = 1.0 / re.iter().fold(1e-6f32, |m, x| m.max(x.abs()));
            }
            let at = (frame * LEVELS + level) * (WAVE_LEN + 1);
            for (d, x) in data[at..at + WAVE_LEN].iter_mut().zip(&re) {
                *d = x * scale;
            }
            data[at + WAVE_LEN] = data[at];
        }
    }
    Table(data)
}

/// `wave` at phase `x` (0..1), between its points.
#[inline]
fn read(wave: &[f32], x: f32) -> f32 {
    let at = x * WAVE_LEN as f32;
    let i = (at as usize).min(WAVE_LEN - 1);
    wave[i] + (wave[i + 1] - wave[i]) * (at - i as f32)
}

#[derive(Clone, Copy, Default)]
struct WtVoice {
    slot: VoiceSlot,
    env: Adsr,
    phase: [f32; MAX_UNISON],
    /// Seconds since the note started, for Sweep.
    age: f32,
    md: VoiceMod,
}

pub(super) struct Wavetable {
    voices: [WtVoice; MAX_VOICES],
    clock: u64,
    rng: Rng,
    mods: Modulation,
    tap: TrackTap,
}

impl Wavetable {
    pub(super) fn new() -> Self {
        Self {
            voices: [WtVoice::default(); MAX_VOICES],
            clock: 0,
            rng: Rng(0x2545_f491),
            mods: Modulation::default(),
            tap: TrackTap::new(),
        }
    }
}

impl Dsp for Wavetable {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        let rng = &mut self.rng;
        let v = &mut self.voices[i];
        v.slot = VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false };
        // Unison voices start apart, so they don't sound as one at first.
        v.phase = std::array::from_fn(|u| if u == 0 { 0.0 } else { 0.5 + 0.5 * rng.next() });
        v.age = 0.0;
        v.md = VoiceMod::default();
        v.env.trigger();
    }

    fn set_modulation(&mut self, m: &Modulation) {
        copy_modulation(&mut self.mods, m);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let table = &TABLES[(p[1].round() as usize).min(TABLES.len() - 1)];
        let (vol, position, sweep) = (p[0], p[2], p[3]);
        let unison = (p[4].round() as usize).clamp(1, MAX_UNISON);
        let (detune, stereo) = (p[5], p[6]);
        let (a, d, s, r) = (p[7], p[8], p[9], p[10]);
        let pan = p[11];
        let norm = 1.0 / (unison as f32).sqrt();
        let block = out.len() as f32 / ctx.sr;
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let mut mb = v.md.block(&self.mods, &v.slot, out.len(), ctx.sr);
            // Where in the table this block reads: two frames and between.
            let at = (position + sweep * v.age).clamp(0.0, 1.0) * (FRAMES - 1) as f32;
            let frame = (at as usize).min(FRAMES - 2);
            let mix = at - frame as f32;
            v.age += block;
            // Each unison voice: its turn per sample, band limit and panning.
            let mut voices = [(0.0f32, 0usize, 1.0f32, 1.0f32); MAX_UNISON];
            for (u, uv) in voices.iter_mut().enumerate().take(unison) {
                let spread = if unison > 1 { u as f32 / (unison - 1) as f32 * 2.0 - 1.0 } else { 0.0 };
                let freq = note_to_freq(v.slot.note + mb.bend + detune * spread / 100.0);
                let dt = (freq / ctx.sr).min(0.49);
                // The fewest halvings that keep the harmonics under Nyquist.
                let room = 0.5 / dt;
                let level = ((WAVE_LEN / 2) as f32 / room).log2().ceil().clamp(0.0, (LEVELS - 1) as f32) as usize;
                let (pl, pr) = pan_gains(pan + v.slot.pan + stereo * spread);
                *uv = (dt, level, pl, pr);
            }
            for (i, o) in out.iter_mut().enumerate() {
                let env = v.env.next(ctx.sr, a, d, s, r);
                let mut y = [0.0f32; 2];
                for (u, &(dt, level, pl, pr)) in voices.iter().enumerate().take(unison) {
                    let x = v.phase[u];
                    let a = read(table.wave(frame, level), x);
                    let b = read(table.wave(frame + 1, level), x);
                    let w = a + (b - a) * mix;
                    y[0] += w * pl;
                    y[1] += w * pr;
                    v.phase[u] = fract(x + dt);
                }
                let y = v.md.apply(&mut mb, y);
                let g = env * v.slot.vel * vol * norm;
                let y = [y[0] * g, y[1] * g];
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

    #[test]
    fn high_notes_read_waves_without_harmonics_past_nyquist() {
        // A saw at its fullest has every harmonic; the level for a note
        // near 5 kHz at 48 kHz keeps under 4.8 of them.
        let basic = &TABLES[0];
        let fft = Fft::new(WAVE_LEN);
        let energy_above = |wave: &[f32], h: usize| {
            let (mut re, mut im) = (wave[..WAVE_LEN].to_vec(), vec![0.0; WAVE_LEN]);
            fft.forward(&mut re, &mut im);
            (h + 1..WAVE_LEN / 2).map(|k| re[k].hypot(im[k])).sum::<f32>()
        };
        let saw = 10; // Two thirds of the way: the saw.
        assert!(energy_above(basic.wave(saw, 0), 8) > 1.0, "the full saw is bright");
        let level = ((WAVE_LEN / 2) as f32 / 4.8).log2().ceil() as usize;
        // What is left past them is rounding, a million times weaker.
        let (kept, above) = (energy_above(basic.wave(saw, level), 0), energy_above(basic.wave(saw, level), 4));
        assert!(above < kept * 1e-5, "level {level} stops at 4 harmonics: {above} of {kept}");
        // Every wave peaks near 1.
        for t in 0..5 {
            let peak = TABLES[t].wave(FRAMES / 2, 0).iter().fold(0f32, |m, x| m.max(x.abs()));
            assert!((0.5..=1.01).contains(&peak), "table {t}: {peak}");
        }
    }

    #[test]
    fn position_moves_through_the_table() {
        // The first frame of Basic is a sine: one harmonic; the last a
        // square: many.
        let mut w = Wavetable::new();
        let ctx = Ctx { sr: 48000.0, samples_per_line: 6000.0, song_line: None };
        let params = |pos: f32| [1.0, 0.0, pos, 0.0, 1.0, 0.0, 0.0, 0.0, 0.1, 1.0, 0.1, 0.0];
        let mut brightness = |pos: f32| {
            w.reset();
            w.note_on(0, 45.0, 1.0);
            let mut out = vec![[0.0; 2]; 2048];
            w.process(&ctx, &params(pos), &[], &mut out);
            // The energy of the change from one sample to the next, which
            // edges are full of, over the energy of the sound.
            out.windows(2).map(|f| (f[1][0] - f[0][0]).powi(2)).sum::<f32>()
                / out.iter().map(|f| f[0].powi(2)).sum::<f32>().max(1e-6)
        };
        let (soft, hard) = (brightness(0.0), brightness(1.0));
        assert!(hard > soft * 10.0, "a square is brighter than a sine: {soft} {hard}");
    }
}
