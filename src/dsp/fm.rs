//! The two FM synths: FM, a carrier and a decaying modulator, and FMX, four operators in eight algorithms.

use super::*;

// ---------------------------------------------------------------- FM

#[derive(Clone, Copy, Default)]
pub(super) struct FmVoice {
    pub(super) slot: VoiceSlot,
    pub(super) env: Adsr,
    pub(super) car: f32,
    pub(super) modu: f32,
    pub(super) mod_env: f32,
    pub(super) last: f32,
    pub(super) md: VoiceMod,
}

pub(super) struct Fm {
    pub(super) voices: [FmVoice; MAX_VOICES],
    pub(super) clock: u64,
    pub(super) mods: Modulation,
    pub(super) tap: TrackTap,
}

impl Fm {
    pub(super) fn new() -> Self {
        Self { voices: [FmVoice::default(); MAX_VOICES], clock: 0, mods: Modulation::default(), tap: TrackTap::new() }
    }
}

impl Dsp for Fm {
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
        v.mod_env = 1.0;
        v.md = VoiceMod::default();
    }

    fn set_modulation(&mut self, m: &Modulation) {
        copy_modulation(&mut self.mods, m);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, ratio, index, mod_decay, fb) = (p[0], p[1], p[2], p[3], p[4]);
        let (a, d, s, r) = (p[5], p[6], p[7], p[8]);
        let mod_mul = (-1.0 / (mod_decay.max(0.001) * ctx.sr)).exp();
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let (pl, pr) = pan_gains(v.slot.pan);
            let mut mb = v.md.block(&self.mods, !v.slot.released, out.len(), ctx.sr);
            let f = note_to_freq(v.slot.note + mb.bend);
            let dc = f / ctx.sr;
            let dm = f * ratio / ctx.sr;
            for (i, o) in out.iter_mut().enumerate() {
                let env = v.env.next(ctx.sr, a, d, s, r);
                let m = sine(v.modu + v.last * fb * 0.5);
                v.last = m;
                let idx = index * (0.15 + 0.85 * v.mod_env);
                let x = sine(v.car + m * idx / TAU) * env * v.slot.vel * vol;
                v.car = fract(v.car + dc);
                v.modu = fract(v.modu + dm);
                v.mod_env *= mod_mul;
                let [l, r] = v.md.apply(&mut mb, [x, x]);
                o[0] += l * pl;
                o[1] += r * pr;
                self.tap.add(v.slot.key, i, [l * pl, r * pr]);
            }
        }
    }
}

// ---------------------------------------------------------------- FMX

/// Operators of an FMX voice.
pub(super) const OPS: usize = 4;

/// For each FMX algorithm, which operators each one modulates, as a bit
/// per operator (bit 0 is operator 1). Operators only modulate lower ones,
/// so running them from 4 down to 1 has each one's input ready.
pub(super) const FMX_ROUTES: [[u8; OPS]; 8] = [
    [0, 0b0001, 0b0010, 0b0100],
    [0, 0b0001, 0b0010, 0b0010],
    [0, 0b0001, 0b0010, 0b0001],
    [0, 0b0001, 0b0001, 0b0100],
    [0, 0b0001, 0, 0b0100],
    [0, 0, 0, 0b0111],
    [0, 0, 0, 0b0100],
    [0, 0, 0, 0],
];

/// How far a modulator at full level pushes its targets' phase, in cycles.
pub(super) const FMX_DEPTH: f32 = 1.5;

/// An FMX voice's envelopes, one per operator.
#[derive(Clone, Copy, Default)]
pub(super) struct OpEnvs(pub(super) [Adsr; OPS]);

impl OpEnvs {
    pub(super) fn release(&mut self) {
        self.0.iter_mut().for_each(Adsr::release);
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct FmxVoice {
    pub(super) slot: VoiceSlot,
    pub(super) env: OpEnvs,
    pub(super) phase: [f32; OPS],
    /// Operator 4's last two outputs, for its feedback.
    pub(super) fb: [f32; 2],
}

/// The FMX, a four-operator FM synth: each operator a sine at its
/// ratio of the note with its own level and envelope, wired by one of
/// eight algorithms, with feedback on operator 4. A voice sounds while an
/// operator that is heard does.
pub(super) struct Fmx {
    pub(super) voices: [FmxVoice; MAX_VOICES],
    pub(super) clock: u64,
    pub(super) tap: TrackTap,
}

impl Fmx {
    pub(super) fn new() -> Self {
        Self { voices: [FmxVoice::default(); MAX_VOICES], clock: 0, tap: TrackTap::new() }
    }
}

/// The operators that modulate none, which are heard, in `routes`.
pub(super) fn carriers(routes: &[u8; OPS]) -> impl Iterator<Item = usize> + '_ {
    (0..OPS).filter(|&k| routes[k] == 0)
}

impl Dsp for Fmx {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.0.iter().any(Adsr::active)));
        let v = &mut self.voices[i];
        *v = FmxVoice::default();
        v.slot = VoiceSlot { key, note, vel, pan: 0.0, age: self.clock, released: false };
        v.env.0.iter_mut().for_each(Adsr::trigger);
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, fb) = (p[0], p[2]);
        let routes = &FMX_ROUTES[(p[1].round() as usize).min(FMX_ROUTES.len() - 1)];
        let op = |k: usize| &p[3 + k * 6..9 + k * 6];
        let heard = carriers(routes).count() as f32;
        for v in self.voices.iter_mut().filter(|v| carriers(routes).any(|k| v.env.0[k].active())) {
            let (pl, pr) = pan_gains(v.slot.pan);
            let f = note_to_freq(v.slot.note);
            let incs: [f32; OPS] = std::array::from_fn(|k| (f * op(k)[1] / ctx.sr).min(0.49));
            for (i, o) in out.iter_mut().enumerate() {
                let mut mods = [0.0; OPS];
                let mut x = 0.0;
                for k in (0..OPS).rev() {
                    let q = op(k);
                    let env = v.env.0[k].next(ctx.sr, q[2], q[3], q[4], q[5]);
                    let mut m = mods[k];
                    if k == OPS - 1 {
                        m += fb * 0.5 * (v.fb[0] + v.fb[1]);
                    }
                    let y = sine(v.phase[k] + m) * env * q[0];
                    v.phase[k] = fract(v.phase[k] + incs[k]);
                    if k == OPS - 1 {
                        v.fb = [y, v.fb[0]];
                    }
                    if routes[k] == 0 {
                        x += y;
                    }
                    for (t, to) in mods.iter_mut().enumerate().take(k) {
                        if routes[k] >> t & 1 == 1 {
                            *to += y * FMX_DEPTH;
                        }
                    }
                }
                let x = x / heard.sqrt() * v.slot.vel * vol;
                o[0] += x * pl;
                o[1] += x * pr;
                self.tap.add(v.slot.key, i, [x * pl, x * pr]);
            }
        }
    }
}
