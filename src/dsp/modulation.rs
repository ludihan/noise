//! Voice modulation: the pitch and filter envelopes, vibrato and tremolo the synths' and Sampler's voices share.

use super::*;

/// Copies `from` into `to` without giving up `to`'s storage, so edits
/// don't allocate on the audio thread once the points have room.
pub(super) fn copy_envelope(to: &mut VoiceEnvelope, from: &VoiceEnvelope) {
    to.points.clear();
    to.points.extend_from_slice(&from.points);
    (to.on, to.sustain, to.curve, to.amount) = (from.on, from.sustain, from.curve, from.amount);
}

pub(super) fn copy_modulation(to: &mut Modulation, m: &Modulation) {
    copy_envelope(&mut to.pitch, &m.pitch);
    copy_envelope(&mut to.filter_env, &m.filter_env);
    (to.filter, to.filter_mode, to.cutoff, to.resonance) = (m.filter, m.filter_mode, m.cutoff, m.resonance);
    (to.key_track, to.velocity) = (m.key_track, m.velocity);
    to.vibrato = m.vibrato.clone();
    to.tremolo = m.tremolo.clone();
}

/// How far an LFO with a delay has faded in, `age` seconds into a note.
pub(super) fn lfo_fade(lfo: &VoiceLfo, age: f32) -> f32 {
    if lfo.delay <= 0.0 { 1.0 } else { ((age - lfo.delay) / lfo.delay).clamp(0.0, 1.0) }
}

/// Where a voice is in its instrument's `Modulation`: seconds since the
/// note started, where the pitch and filter envelopes are (they stop at
/// their sustain points while it is held), the LFOs' phases and the
/// filter's state.
#[derive(Clone, Copy, Default)]
pub(super) struct VoiceMod {
    pub(super) age: f32,
    pub(super) pitch_t: f32,
    pub(super) filter_t: f32,
    pub(super) vibrato_phase: f32,
    pub(super) tremolo_phase: f32,
    pub(super) svf: [Svf; 2],
}

/// What modulation does to a voice during one block.
pub(super) struct ModBlock {
    /// Semitones to add to the pitch.
    pub(super) bend: f32,
    /// The tremolo gain, moving by `trem_step` each frame.
    pub(super) trem: f32,
    pub(super) trem_step: f32,
    /// The filter's coefficients and mode, when it is on.
    pub(super) filter: Option<(f32, f32, u8)>,
}

impl VoiceMod {
    /// Works out modulation for the next `frames` frames of the voice
    /// playing `slot` and moves on.
    pub(super) fn block(&mut self, m: &Modulation, slot: &VoiceSlot, frames: usize, sr: f32) -> ModBlock {
        let held = !slot.released;
        let block = frames as f32 / sr;
        let mut bend = 0.0;
        if m.pitch.on {
            bend += (m.pitch.value(self.pitch_t) - 0.5) * 2.0 * m.pitch.amount;
            self.pitch_t = m.pitch.advance(self.pitch_t, block, held);
        }
        if m.vibrato.on {
            bend += lfo_shape(m.vibrato.shape as u32, self.vibrato_phase)
                * m.vibrato.depth
                * lfo_fade(&m.vibrato, self.age);
            self.vibrato_phase = fract(self.vibrato_phase + m.vibrato.rate * block);
        }
        let tremolo = |phase: f32, age: f32| {
            let depth = m.tremolo.depth * lfo_fade(&m.tremolo, age);
            1.0 - depth * (0.5 - 0.5 * lfo_shape(m.tremolo.shape as u32, phase))
        };
        let (from, to) = if m.tremolo.on {
            let from = tremolo(self.tremolo_phase, self.age);
            self.tremolo_phase = fract(self.tremolo_phase + m.tremolo.rate * block);
            (from, tremolo(self.tremolo_phase, self.age + block))
        } else {
            (1.0, 1.0)
        };
        let filter = m.filter.then(|| {
            let env = if m.filter_env.on { m.filter_env.value(self.filter_t) * m.filter_env.amount } else { 0.0 };
            self.filter_t = m.filter_env.advance(self.filter_t, block, held);
            // Higher notes open it as far as key tracking says, from C-4, and
            // softer ones close it by up to the velocity amount.
            let octaves = env + m.key_track * (slot.note - 48.0) / 12.0 - m.velocity * (1.0 - slot.vel.clamp(0.0, 1.0));
            let cutoff = (m.cutoff * 2f32.powf(octaves)).clamp(20.0, sr * 0.45);
            ((PI * cutoff / sr).tan(), 2.0 - 2.0 * m.resonance, m.filter_mode)
        });
        self.age += block;
        ModBlock { bend, trem: from, trem_step: (to - from) / frames.max(1) as f32, filter }
    }

    /// Filters a frame of the voice and applies tremolo.
    pub(super) fn apply(&mut self, b: &mut ModBlock, mut x: Frame) -> Frame {
        if let Some((g, k, mode)) = b.filter {
            for (x, svf) in x.iter_mut().zip(&mut self.svf) {
                let (l, h, band) = svf.tick(*x, g, k);
                *x = match mode {
                    0 => l,
                    1 => h,
                    _ => band,
                };
            }
        }
        let g = b.trem;
        b.trem += b.trem_step;
        [x[0] * g, x[1] * g]
    }
}
