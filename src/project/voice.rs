//! How an instrument's voices change while they play: pitch and filter
//! envelopes, the filter, vibrato and tremolo, and the controls the
//! Modulation page edits them with.

use super::*;

/// An envelope a Sampler's voices run through from each note, in seconds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceEnvelope {
    pub on: bool,
    /// Points as (seconds since the note started, value 0..1), sorted.
    pub points: Vec<(f32, f32)>,
    /// The point the envelope holds at while the note is held.
    pub sustain: Option<usize>,
    pub curve: bool,
    /// How far the envelope moves its target: semitones for pitch,
    /// octaves for the filter.
    pub amount: f32,
}

impl Default for VoiceEnvelope {
    fn default() -> Self {
        VoiceEnvelope::flat(0.5, 12.0)
    }
}

impl Default for VoiceLfo {
    fn default() -> Self {
        VoiceLfo::new(0.3)
    }
}

impl VoiceEnvelope {
    fn flat(value: f32, amount: f32) -> Self {
        VoiceEnvelope { on: false, points: vec![(0.0, value)], sustain: None, curve: false, amount }
    }

    /// The value `t` seconds into the note; `advance` keeps `t` at the
    /// sustain point while the note is held.
    pub fn value(&self, t: f32) -> f32 {
        interpolate(&self.points, t, false, self.curve).unwrap_or(0.0)
    }

    /// Where a note `t` seconds in moves to after `dt` more seconds.
    pub fn advance(&self, t: f32, dt: f32, held: bool) -> f32 {
        match self.sustain.and_then(|i| self.points.get(i)) {
            Some(&(hold, _)) if held && t <= hold => (t + dt).min(hold),
            _ => t + dt,
        }
    }

    /// The time of the last point.
    pub fn length(&self) -> f32 {
        self.points.last().map_or(0.0, |p| p.0)
    }
}

/// A low-frequency oscillator on each voice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VoiceLfo {
    pub on: bool,
    /// Index into `LFO_SHAPES`.
    pub shape: u8,
    pub rate: f32,
    /// Semitones for vibrato, 0..1 of the volume for tremolo.
    pub depth: f32,
    /// Seconds before it starts, fading in over as long again.
    pub delay: f32,
}

impl VoiceLfo {
    fn new(depth: f32) -> Self {
        VoiceLfo { on: false, shape: 0, rate: 5.0, depth, delay: 0.0 }
    }
}

/// How a Sampler's voices change while they play: a pitch envelope, a
/// filter with its own envelope, vibrato and tremolo.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Modulation {
    pub pitch: VoiceEnvelope,
    pub filter: bool,
    /// Index into `FILTER_MODES`.
    pub filter_mode: u8,
    pub cutoff: f32,
    pub resonance: f32,
    pub filter_env: VoiceEnvelope,
    /// How far the cutoff follows the note, from none to a whole octave an
    /// octave, around C-4.
    pub key_track: f32,
    /// How many octaves the cutoff closes for the softest notes.
    pub velocity: f32,
    pub vibrato: VoiceLfo,
    pub tremolo: VoiceLfo,
}

impl Default for Modulation {
    fn default() -> Self {
        Modulation {
            pitch: VoiceEnvelope::flat(0.5, 12.0),
            filter: false,
            filter_mode: 0,
            cutoff: 2000.0,
            resonance: 0.3,
            filter_env: VoiceEnvelope { points: vec![(0.0, 1.0), (0.4, 0.3)], ..VoiceEnvelope::flat(0.0, 3.0) },
            key_track: 0.0,
            velocity: 0.0,
            vibrato: VoiceLfo::new(0.3),
            tremolo: VoiceLfo::new(0.5),
        }
    }
}

impl Modulation {
    pub fn is_default(&self) -> bool {
        *self == Modulation::default()
    }

    /// Keeps hand-edited values in range.
    pub(super) fn normalize(&mut self) {
        for env in [&mut self.pitch, &mut self.filter_env] {
            for p in &mut env.points {
                *p = (p.0.clamp(0.0, 60.0), p.1.clamp(0.0, 1.0));
            }
            env.points.sort_by(|a, b| a.0.total_cmp(&b.0));
            if env.points.is_empty() {
                env.points.push((0.0, 0.5));
            }
            env.sustain = env.sustain.filter(|&i| i < env.points.len());
        }
        self.pitch.amount = self.pitch.amount.clamp(0.0, 48.0);
        self.filter_env.amount = self.filter_env.amount.clamp(-8.0, 8.0);
        self.filter_mode = self.filter_mode.min(FILTER_MODES.len() as u8 - 1);
        self.cutoff = self.cutoff.clamp(20.0, 20000.0);
        self.resonance = self.resonance.clamp(0.0, 0.97);
        self.key_track = self.key_track.clamp(0.0, 1.0);
        self.velocity = self.velocity.clamp(0.0, 4.0);
        for lfo in [&mut self.vibrato, &mut self.tremolo] {
            lfo.shape = lfo.shape.min(LFO_SHAPES.len() as u8 - 1);
            lfo.rate = lfo.rate.clamp(0.05, 20.0);
            lfo.delay = lfo.delay.clamp(0.0, 10.0);
        }
        self.vibrato.depth = self.vibrato.depth.clamp(0.0, 12.0);
        self.tremolo.depth = self.tremolo.depth.clamp(0.0, 1.0);
    }
}

/// The controls of the Sampler's modulation, for its editor.
pub static PITCH_AMOUNT: ParamSpec = i("Range", 0.0, 48.0, 12.0).unit(Unit::Semitones);
pub static FILTER_CUTOFF: ParamSpec = p("Cutoff", 20.0, 20000.0, 2000.0).unit(Unit::Hz);
pub static FILTER_RESONANCE: ParamSpec = p("Resonance", 0.0, 0.97, 0.3).unit(Unit::Percent);
pub static FILTER_ENV_AMOUNT: ParamSpec = p("Env amount", -8.0, 8.0, 3.0).unit(Unit::Octaves);
pub static FILTER_KEY_TRACK: ParamSpec = p("Key track", 0.0, 1.0, 0.0).unit(Unit::Percent);
pub static FILTER_VELOCITY: ParamSpec = p("Velocity", 0.0, 4.0, 0.0).unit(Unit::Octaves);
pub static LFO_RATE: ParamSpec = p("Rate", 0.05, 20.0, 5.0).unit(Unit::Hz);
pub static LFO_DELAY: ParamSpec = p("Delay", 0.0, 10.0, 0.0).unit(Unit::Seconds);
pub static VIBRATO_DEPTH: ParamSpec = p("Depth", 0.0, 12.0, 0.3).unit(Unit::Semitones);
pub static TREMOLO_DEPTH: ParamSpec = p("Depth", 0.0, 1.0, 0.5).unit(Unit::Percent);
