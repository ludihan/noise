//! The Analog Synth: two oscillators, a sub and noise into a ladder filter
//! on every voice, each with its own filter and amp envelopes and an LFO,
//! played as chords, or one note at a time with glide.

use super::*;

/// Where the Analog Synth's parameters are.
mod at {
    pub const VOLUME: usize = 0;
    pub const OSC1: usize = 1;
    pub const OSC2: usize = 2;
    pub const OSC2_PITCH: usize = 3;
    pub const OSC2_DETUNE: usize = 4;
    pub const MIX: usize = 5;
    pub const SUB: usize = 6;
    pub const NOISE: usize = 7;
    pub const WIDTH: usize = 8;
    pub const FILTER: usize = 9;
    pub const CUTOFF: usize = 10;
    pub const RESONANCE: usize = 11;
    pub const ENV_AMOUNT: usize = 12;
    pub const KEY_TRACK: usize = 13;
    pub const VELOCITY: usize = 14;
    pub const FILTER_ENV: usize = 15;
    pub const AMP_ENV: usize = 19;
    pub const LFO_SHAPE: usize = 23;
    pub const LFO_RATE: usize = 24;
    pub const LFO_PITCH: usize = 25;
    pub const LFO_CUTOFF: usize = 26;
    pub const LFO_WIDTH: usize = 27;
    pub const ENV_PITCH: usize = 28;
    pub const VOICES: usize = 29;
    pub const GLIDE: usize = 30;
    pub const UNISON: usize = 31;
    pub const SPREAD: usize = 32;
    pub const PAN: usize = 33;
}

const MAX_UNISON: usize = 4;

#[derive(Clone, Copy, Default)]
pub(super) struct AnalogVoice {
    pub(super) slot: VoiceSlot,
    /// The amp envelope; `voice_controls!` releases it.
    pub(super) env: Adsr,
    pub(super) filter_env: Adsr,
    /// The note sounding, on its way to `slot.note` over the glide.
    pub(super) pitch: f32,
    /// Each unison voice's two oscillators and their triangles' states.
    pub(super) phase: [[f32; 2]; MAX_UNISON],
    pub(super) tri: [[f32; 2]; MAX_UNISON],
    pub(super) sub: f32,
    pub(super) lfo: f32,
    pub(super) ladder: Ladder,
}

pub(super) struct Analog {
    pub(super) voices: [AnalogVoice; MAX_VOICES],
    pub(super) clock: u64,
    pub(super) rng: Rng,
    /// The last note played, which the next one glides from.
    pub(super) last: Option<f32>,
    /// The voice mode and glide time of the last block, which `note_on`
    /// has no parameters for.
    pub(super) mode: u32,
    pub(super) glide: f32,
    pub(super) tap: TrackTap,
}

impl Analog {
    pub(super) fn new() -> Self {
        let mut voices = [AnalogVoice::default(); MAX_VOICES];
        // Spread the phases so unison voices don't start in lockstep, and
        // keep each pair close, so the same wave twice doesn't cancel.
        for v in &mut voices {
            v.phase = [[0.0, 0.1], [0.31, 0.43], [0.67, 0.79], [0.13, 0.23]];
        }
        Self { voices, clock: 0, rng: Rng(0x9e37_79b9), last: None, mode: 0, glide: 0.0, tap: TrackTap::new() }
    }
}

impl Dsp for Analog {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let from = if self.glide > 0.0 { self.last.unwrap_or(note) } else { note };
        self.last = Some(note);
        let slot = VoiceSlot::new(key, note, vel, self.clock);
        if self.mode > 0 {
            // One voice: the note takes it over where it is. Legato keeps
            // its envelopes going while a note is held.
            let v = &mut self.voices[0];
            let held = v.env.active() && !v.slot.released;
            if !v.env.active() {
                v.pitch = from;
            }
            v.slot = slot;
            if !(held && self.mode == 2) {
                v.env.trigger();
                v.filter_env.trigger();
            }
            return;
        }
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        let v = &mut self.voices[i];
        v.slot = slot;
        v.pitch = from;
        v.lfo = 0.0;
        v.env.trigger();
        v.filter_env.trigger();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let sr = ctx.sr;
        let mode = p[at::VOICES].round() as u32;
        if mode > 0 {
            // One voice: the newest note held is it, and the others let go,
            // as when notes came before the mode was known.
            let held = self.voices.iter().enumerate().filter(|(_, v)| v.env.active() && !v.slot.released);
            if let Some((newest, _)) = held.max_by_key(|(_, v)| v.slot.age) {
                self.voices.swap(0, newest);
            }
            for v in self.voices[1..].iter_mut().filter(|v| !v.slot.released) {
                v.slot.released = true;
                v.env.release();
            }
        }
        (self.mode, self.glide) = (mode, p[at::GLIDE]);
        let waves = [p[at::OSC1].round() as u32, p[at::OSC2].round() as u32];
        let osc2 = p[at::OSC2_PITCH].round() + p[at::OSC2_DETUNE] / 100.0;
        let (mix, sub, noise, width) = (p[at::MIX], p[at::SUB], p[at::NOISE], p[at::WIDTH]);
        let (kind, cutoff, k) = (p[at::FILTER].round() as u32, p[at::CUTOFF], 4.0 * p[at::RESONANCE]);
        let fe = &p[at::FILTER_ENV..at::FILTER_ENV + 4];
        let ae = &p[at::AMP_ENV..at::AMP_ENV + 4];
        let unison = (p[at::UNISON].round() as usize).clamp(1, MAX_UNISON);
        let norm = 1.0 / (unison as f32).sqrt();
        let n = out.len();
        // How far a gliding note gets towards where it is going this block.
        let glide = if self.glide > 0.0 { 1.0 - (-(n as f32) / (self.glide * sr)).exp() } else { 1.0 };

        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            // Letting go of a note releases its filter envelope too.
            if v.slot.released {
                v.filter_env.release();
            }
            v.pitch += (v.slot.note - v.pitch) * glide;
            let lfo = lfo_shape(p[at::LFO_SHAPE].round() as u32, v.lfo);
            v.lfo = fract(v.lfo + p[at::LFO_RATE] * n as f32 / sr);
            // The filter envelope bends the pitch too, at the block's start.
            let bend = lfo * p[at::LFO_PITCH] + v.filter_env.level * p[at::ENV_PITCH];
            let mut dts = [[0.0; 2]; MAX_UNISON];
            for (u, dt) in dts.iter_mut().enumerate().take(unison) {
                // Unison voices fan out by up to the spread either side, in cents.
                let spread =
                    if unison > 1 { (u as f32 / (unison - 1) as f32 - 0.5) * 2.0 * p[at::SPREAD] / 100.0 } else { 0.0 };
                let note = v.pitch + bend + spread;
                *dt = [note, note + osc2].map(|n| (note_to_freq(n) / sr).min(0.49));
            }
            let sub_dt = (note_to_freq(v.pitch + bend - 12.0) / sr).min(0.49);
            let pw = (width + lfo * p[at::LFO_WIDTH]).clamp(0.05, 0.95);
            // The cutoff before the envelope: the knob, the key, how hard the
            // note was played and the LFO, in octaves.
            let octaves = p[at::KEY_TRACK] * (v.slot.note - 48.0) / 12.0
                - p[at::VELOCITY] * (1.0 - v.slot.vel.clamp(0.0, 1.0))
                + lfo * p[at::LFO_CUTOFF];
            let (pl, pr) = pan_gains(p[at::PAN] + v.slot.pan);
            let gain = v.slot.vel * p[at::VOLUME];
            for (i, o) in out.iter_mut().enumerate() {
                let mut x = 0.0;
                for ((phase, tri), dt) in v.phase.iter_mut().zip(&mut v.tri).zip(&dts).take(unison) {
                    for (o, &wave) in waves.iter().enumerate() {
                        let y = blep_wave(wave, phase[o], dt[o], pw, &mut tri[o]);
                        phase[o] = fract(phase[o] + dt[o]);
                        x += y * if o == 0 { 1.0 - mix } else { mix };
                    }
                }
                x *= norm;
                x += sub * if v.sub < 0.5 { 1.0 } else { -1.0 } + noise * self.rng.next();
                v.sub = fract(v.sub + sub_dt);
                let env = v.filter_env.next(sr, fe[0], fe[1], fe[2], fe[3]);
                let fc = (cutoff * 2f32.powf(octaves + env * p[at::ENV_AMOUNT])).clamp(20.0, sr * 0.45);
                let y = v.ladder.tick(x * 0.5, (PI * fc / sr).tan(), k, kind);
                let a = v.env.next(sr, ae[0], ae[1], ae[2], ae[3]);
                let y = [y * a * gain * pl, y * a * gain * pr];
                o[0] += y[0];
                o[1] += y[1];
                self.tap.add(v.slot.key, i, y);
            }
            if !v.env.active() {
                v.ladder = Ladder::default();
            }
        }
    }
}
