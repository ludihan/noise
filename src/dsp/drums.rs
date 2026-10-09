//! The drum synths: Drums, a kit of synthesized hits, and Kicker, a kick drum.

use super::*;

// ---------------------------------------------------------------- drums

#[derive(Clone, Copy, Default, PartialEq)]
pub(super) enum DrumKind {
    #[default]
    Kick,
    Snare,
    ClosedHat,
    OpenHat,
    Tom,
}

#[derive(Clone, Copy, Default)]
pub(super) struct DrumVoice {
    pub(super) slot: VoiceSlot,
    pub(super) kind: DrumKind,
    pub(super) active: bool,
    pub(super) t: f32,
    pub(super) phase: f32,
    pub(super) hp: [f32; 2],
}

pub(super) struct Drums {
    pub(super) voices: [DrumVoice; MAX_VOICES],
    pub(super) clock: u64,
    pub(super) rng: Rng,
    pub(super) tap: TrackTap,
}

impl Drums {
    pub(super) fn new() -> Self {
        Self { voices: [DrumVoice::default(); MAX_VOICES], clock: 0, rng: Rng(0x9e37_79b9), tap: TrackTap::new() }
    }
}

impl Dsp for Drums {
    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        // Pitch class picks the drum: C kick, D snare, F# closed hat,
        // A# open hat, anything else a tom tuned to the note.
        let kind = match (note.round() as i32).rem_euclid(12) {
            0 | 1 => DrumKind::Kick,
            2..=4 => DrumKind::Snare,
            6 | 8 => DrumKind::ClosedHat,
            10 | 11 => DrumKind::OpenHat,
            _ => DrumKind::Tom,
        };
        if kind == DrumKind::ClosedHat {
            // Closed hat chokes the open one.
            for v in self.voices.iter_mut().filter(|v| v.kind == DrumKind::OpenHat) {
                v.active = false;
            }
        }
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.active));
        self.voices[i] = DrumVoice {
            slot: VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false },
            kind,
            active: true,
            ..Default::default()
        };
    }

    fn set_pan(&mut self, key: u32, pan: f32) {
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.active) {
            v.slot.pan = pan;
        }
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.active = false;
        }
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, kick_tone, kick_decay, snare_tone, hat_decay) = (p[0], p[1], p[2], p[3], p[4]);
        let dt = 1.0 / ctx.sr;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let (pl, pr) = pan_gains(v.slot.pan);
            for (i, o) in out.iter_mut().enumerate() {
                let t = v.t;
                let (x, done) = match v.kind {
                    DrumKind::Kick => {
                        let f = kick_tone + 350.0 * (-t * 40.0).exp();
                        v.phase = fract(v.phase + f * dt);
                        let env = (-t / kick_decay * 4.0).exp();
                        let click = if t < 0.002 { self.rng.next() * 0.3 } else { 0.0 };
                        ((v.phase * TAU).sin() * env + click, env < 0.001)
                    }
                    DrumKind::Snare => {
                        let f = 160.0 + 60.0 * snare_tone + 100.0 * (-t * 60.0).exp();
                        v.phase = fract(v.phase + f * dt);
                        let body = (v.phase * TAU).sin() * (-t * 25.0).exp() * snare_tone;
                        let n = self.rng.next();
                        // One-pole highpass on the noise.
                        let hp = n - v.hp[0];
                        v.hp[0] += 0.3 * hp;
                        let noise = hp * (-t * 14.0).exp() * (1.2 - snare_tone * 0.6);
                        let x = body + noise;
                        (x, t > 0.6)
                    }
                    DrumKind::ClosedHat | DrumKind::OpenHat => {
                        let decay = if v.kind == DrumKind::OpenHat { hat_decay * 6.0 } else { hat_decay };
                        let n = self.rng.next();
                        // Two cascaded highpasses for a thin metallic hiss.
                        let h1 = n - v.hp[0];
                        v.hp[0] += 0.6 * h1;
                        let h2 = h1 - v.hp[1];
                        v.hp[1] += 0.6 * h2;
                        let env = (-t / decay).exp();
                        (h2 * env * 0.8, env < 0.001)
                    }
                    DrumKind::Tom => {
                        let base = note_to_freq(v.slot.note);
                        let f = base * (1.0 + 0.6 * (-t * 20.0).exp());
                        v.phase = fract(v.phase + f * dt);
                        let env = (-t * 6.0).exp();
                        ((v.phase * TAU).sin() * env, env < 0.001)
                    }
                };
                let x = x * v.slot.vel * vol;
                o[0] += x * pl;
                o[1] += x * pr;
                self.tap.add(v.slot.key, i, [x * pl, x * pr]);
                v.t += dt;
                if done {
                    v.active = false;
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------- kicker

#[derive(Clone, Copy, Default)]
pub(super) struct KickVoice {
    pub(super) slot: VoiceSlot,
    pub(super) active: bool,
    /// Seconds since the note started.
    pub(super) t: f32,
    pub(super) phase: f32,
}

/// The Kicker: a wave that falls from octaves above the note to the
/// note, fading out over the decay, with a drive (Boost) that squares it
/// off. Notes play out in full; a note off doesn't stop them.
pub(super) struct Kicker {
    pub(super) voices: [KickVoice; MAX_VOICES],
    pub(super) clock: u64,
    pub(super) tap: TrackTap,
}

impl Kicker {
    pub(super) fn new() -> Self {
        Self { voices: [KickVoice::default(); MAX_VOICES], clock: 0, tap: TrackTap::new() }
    }
}

impl Dsp for Kicker {
    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.active));
        self.voices[i] = KickVoice {
            slot: VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false },
            active: true,
            ..Default::default()
        };
    }

    fn set_pitch(&mut self, key: u32, note: f32) {
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.active) {
            v.slot.note = note;
        }
    }

    fn set_pan(&mut self, key: u32, pan: f32) {
        for v in self.voices.iter_mut().filter(|v| v.slot.key == key && v.active) {
            v.slot.pan = pan;
        }
    }

    fn reset(&mut self) {
        for v in &mut self.voices {
            v.active = false;
        }
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, wave, drop, sweep, attack, decay) = (p[0], p[1].round() as u32, p[2], p[3], p[4], p[5]);
        let drive = 1.0 + p[6] * 15.0;
        let norm = 1.0 / drive.tanh();
        let dt = 1.0 / ctx.sr;
        for v in self.voices.iter_mut().filter(|v| v.active) {
            let (pl, pr) = pan_gains(p[7] + v.slot.pan);
            let base = note_to_freq(v.slot.note);
            for (i, o) in out.iter_mut().enumerate() {
                let t = v.t;
                let f = base * 2f32.powf(drop * (-t / sweep).exp());
                v.phase = fract(v.phase + f * dt);
                let x = match wave {
                    0 => sine(v.phase),
                    1 => 1.0 - 4.0 * (v.phase - 0.5).abs(),
                    _ => (v.phase * TAU).sin().signum(),
                };
                let env = (t / attack.max(1e-4)).min(1.0) * (-t / decay * 5.0).exp();
                let x = (x * drive).tanh() * norm * env * v.slot.vel * vol;
                o[0] += x * pl;
                o[1] += x * pr;
                self.tap.add(v.slot.key, i, [x * pl, x * pr]);
                v.t += dt;
                if env < 0.0005 && t > attack {
                    v.active = false;
                    break;
                }
            }
        }
    }
}
