//! SpectraVoice: additive synthesis from up to 32 harmonics.

use super::*;

pub(super) const MAX_PARTIALS: usize = 32;

#[derive(Clone, Copy)]
pub(super) struct SpectraVoiceVoice {
    pub(super) slot: VoiceSlot,
    pub(super) env: Adsr,
    /// Each partial's phase as a point on the unit circle, turned a step
    /// each frame: cheaper than a sine per partial per frame.
    pub(super) z: [[f32; 2]; MAX_PARTIALS],
    /// Seconds since the note started, for the shimmer.
    pub(super) t: f32,
}

impl Default for SpectraVoiceVoice {
    fn default() -> Self {
        Self { slot: VoiceSlot::default(), env: Adsr::default(), z: [[1.0, 0.0]; MAX_PARTIALS], t: 0.0 }
    }
}

/// The SpectraVoice: additive synthesis from up to 32
/// harmonics, the k-th at 1/k^slope of the first, with the even ones
/// turned down by Even (none is hollow, like a square), Stretch pulling
/// them sharp or flat like a stiff string, and Shimmer making each one
/// swell and fade at its own slow rate.
pub(super) struct SpectraVoice {
    pub(super) voices: [SpectraVoiceVoice; MAX_VOICES],
    pub(super) clock: u64,
    pub(super) tap: TrackTap,
}

impl SpectraVoice {
    pub(super) fn new() -> Self {
        Self { voices: [SpectraVoiceVoice::default(); MAX_VOICES], clock: 0, tap: TrackTap::new() }
    }
}

impl Dsp for SpectraVoice {
    voice_controls!(note_off);

    fn track_tap(&mut self) -> Option<&mut TrackTap> {
        Some(&mut self.tap)
    }

    fn note_on(&mut self, key: u32, note: f32, vel: f32) {
        self.clock += 1;
        let i = alloc_voice(&mut self.voices, |v| (&v.slot, v.env.active()));
        let v = &mut self.voices[i];
        *v = SpectraVoiceVoice::default();
        v.slot = VoiceSlot::new(key, note, vel, self.clock);
        v.env.trigger();
    }

    fn process(&mut self, ctx: &Ctx, p: &[f32], _: &[Frame], out: &mut [Frame]) {
        out.fill([0.0; 2]);
        let (vol, harmonics, slope, even, stretch, shimmer) =
            (p[0], (p[1].round() as usize).clamp(1, MAX_PARTIALS), p[2], p[3], p[4], p[5]);
        let (a, d, s, r) = (p[6], p[7], p[8], p[9]);
        let block = out.len() as f32 / ctx.sr;
        for v in self.voices.iter_mut().filter(|v| v.env.active()) {
            let (pl, pr) = pan_gains(p[10] + v.slot.pan);
            let f0 = note_to_freq(v.slot.note);
            // Each partial's level and turn per frame for this block;
            // partials above Nyquist are left out.
            let mut amps = [0.0; MAX_PARTIALS];
            let mut rots = [[1.0, 0.0]; MAX_PARTIALS];
            let mut used = 0;
            let mut power = 0.0;
            for k in 0..harmonics {
                let h = (k + 1) as f32;
                let f = f0 * h * (1.0 + stretch * (h - 1.0));
                if f <= 0.0 || f >= ctx.sr * 0.45 {
                    break;
                }
                let mut amp = h.powf(-slope) * if k % 2 == 1 { even } else { 1.0 };
                power += amp * amp;
                let rate = 0.21 + 0.13 * h;
                amp *= 1.0 - shimmer * 0.5 * (1.0 + sine(v.t * rate + h * 0.37));
                amps[k] = amp;
                let (sn, cs) = (TAU * f / ctx.sr).sin_cos();
                rots[k] = [cs, sn];
                used = k + 1;
            }
            let norm = if power > 0.0 { 0.5 / power.sqrt() } else { 0.0 };
            for (i, o) in out.iter_mut().enumerate() {
                let env = v.env.next(ctx.sr, a, d, s, r);
                let mut x = 0.0;
                for k in 0..used {
                    let [c, sn] = v.z[k];
                    let [rc, rs] = rots[k];
                    v.z[k] = [c * rc - sn * rs, c * rs + sn * rc];
                    x += v.z[k][1] * amps[k];
                }
                let x = x * norm * env * v.slot.vel * vol;
                o[0] += x * pl;
                o[1] += x * pr;
                self.tap.add(v.slot.key, i, [x * pl, x * pr]);
            }
            // Keep the points on the circle as rounding drifts them.
            for z in &mut v.z[..used] {
                let m = (z[0] * z[0] + z[1] * z[1]).sqrt();
                *z = [z[0] / m, z[1] / m];
            }
            v.t += block;
        }
    }
}
