//! The Generator: band-limited saw, square and triangle, sine and noise, with unison.

use super::*;

pub(super) fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

/// One sample of a band-limited saw (0), pulse of width `pw` (1), triangle
/// (2) or sine (3) at `t` into its cycle, which moves `dt` a sample; the
/// triangle integrates a square in `tri`, kept between samples.
pub(super) fn blep_wave(wave: u32, t: f32, dt: f32, pw: f32, tri: &mut f32) -> f32 {
    match wave {
        0 => 2.0 * t - 1.0 - poly_blep(t, dt),
        1 => {
            let mut y = if t < pw { 1.0 } else { -1.0 };
            y += poly_blep(t, dt);
            y -= poly_blep(fract(t - pw + 1.0), dt);
            y
        }
        2 => {
            // Integrated band-limited square.
            let mut sq = if t < 0.5 { 1.0 } else { -1.0 };
            sq += poly_blep(t, dt);
            sq -= poly_blep(fract(t + 0.5), dt);
            *tri = dt * 4.0 * sq + (1.0 - dt * 0.5) * *tri;
            *tri
        }
        _ => sine(t),
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct GenVoice {
    pub(super) slot: VoiceSlot,
    pub(super) env: Adsr,
    pub(super) phase: [f32; 4],
    pub(super) tri: [f32; 4],
    pub(super) md: VoiceMod,
}

pub(super) struct Generator {
    pub(super) voices: [GenVoice; MAX_VOICES],
    pub(super) clock: u64,
    pub(super) rng: Rng,
    pub(super) mods: Modulation,
    pub(super) tap: TrackTap,
}

impl Generator {
    pub(super) fn new() -> Self {
        let mut voices = [GenVoice::default(); MAX_VOICES];
        // Spread initial phases so unison voices don't start in lockstep.
        for v in &mut voices {
            v.phase = [0.0, 0.31, 0.67, 0.13];
        }
        Self { voices, clock: 0, rng: Rng(0x1234_5678), mods: Modulation::default(), tap: TrackTap::new() }
    }
}

impl Dsp for Generator {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        let v = &mut self.voices[i];
        v.slot = VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false };
        v.md = VoiceMod::default();
        v.env.trigger();
    }

    fn set_modulation(&mut self, m: &Modulation) {
        copy_modulation(&mut self.mods, m);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, wave) = (p[0], p[1].round() as u32);
        let (a, d, s, r) = (p[2], p[3], p[4], p[5]);
        let detune = p[6];
        let unison = (p[7].round() as usize).clamp(1, 4);
        let pw = p[8];
        let norm = 1.0 / (unison as f32).sqrt();

        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let (pl, pr) = pan_gains(p[9] + v.slot.pan);
            let mut mb = v.md.block(&self.mods, &v.slot, out.len(), ctx.sr);
            let mut dts = [0.0; 4];
            for (u, dt) in dts.iter_mut().enumerate().take(unison) {
                // Unison voices fan out symmetrically around the note.
                let spread = if unison > 1 { (u as f32 / (unison - 1) as f32 - 0.5) * 2.0 } else { 0.0 };
                let cents = detune * spread;
                *dt = (note_to_freq(v.slot.note + mb.bend + cents / 100.0) / ctx.sr).min(0.49);
            }
            for (i, o) in out.iter_mut().enumerate() {
                let env = v.env.next(ctx.sr, a, d, s, r);
                let mut sl = 0.0;
                let mut sr_ = 0.0;
                for (u, &dt) in dts.iter().enumerate().take(unison) {
                    let t = v.phase[u];
                    let x = if wave < 4 { blep_wave(wave, t, dt, pw, &mut v.tri[u]) } else { self.rng.next() };
                    v.phase[u] = fract(t + dt);
                    // Alternate unison voices left/right for width.
                    if unison > 1 && u % 2 == 1 {
                        sr_ += x * 1.3;
                        sl += x * 0.7;
                    } else if unison > 1 {
                        sl += x * 1.3;
                        sr_ += x * 0.7;
                    } else {
                        sl += x;
                        sr_ += x;
                    }
                }
                let [sl, sr_] = v.md.apply(&mut mb, [sl, sr_]);
                let g = env * v.slot.vel * vol * norm;
                let y = [sl * g * pl, sr_ * g * pr];
                o[0] += y[0];
                o[1] += y[1];
                self.tap.add(v.slot.key, i, y);
            }
        }
    }
}
