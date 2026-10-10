//! The kinds of module, their parameters (with ranges, units and choices)
//! and what each kind can do: play notes, take sound, move other modules.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleKind {
    Output,
    Generator,
    Fm,
    Drums,
    Sampler,
    Filter,
    Distortion,
    Delay,
    Reverb,
    Amplifier,
    Lfo,
    Flanger,
    Compressor,
    Eq,
    MultiSynth,
    Modulator,
    Phaser,
    VocalFilter,
    Repeater,
    RingMod,
    Gate,
    Kicker,
    SpectraVoice,
    PitchShifter,
    StereoExpander,
    CombFilter,
    Maximizer,
    Exciter,
    DcBlocker,
    ScreamFilter,
    Multitap,
    Eq10,
    Cabinet,
    Vibrato,
    Fmx,
    Input,
    WaveShaper,
    FilterPro,
    Chorus,
    Echo,
    AnalogFilter,
    PlateReverb,
    Eq5,
    Glide,
    Wavetable,
    Granular,
    Convolver,
    Analog,
    PluckedString,
    Vocoder,
}

/// How a parameter's value is shown.
#[derive(Clone, Copy, PartialEq)]
pub enum Unit {
    Plain,
    /// A linear gain, shown in decibels.
    Gain,
    Hz,
    /// Seconds, shown in milliseconds below one second.
    Seconds,
    /// 0..1 as a percentage.
    Percent,
    Semitones,
    Cents,
    Lines,
    /// -1..1, left to right.
    Pan,
    /// A frequency ratio.
    Ratio,
    /// A MIDI-style note number, shown as a note name.
    Note,
    Octaves,
    /// 0..4 through the vowels A, E, I, O and U.
    Vowel,
}

pub struct ParamSpec {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// If set, the param is a discrete choice and these are its labels.
    pub choices: &'static [&'static str],
    /// The param only takes whole numbers.
    pub integer: bool,
    pub unit: Unit,
}

pub(super) const fn p(name: &'static str, min: f32, max: f32, default: f32) -> ParamSpec {
    ParamSpec { name, min, max, default, choices: &[], integer: false, unit: Unit::Plain }
}

pub(super) const fn c(name: &'static str, default: f32, choices: &'static [&'static str]) -> ParamSpec {
    ParamSpec { name, min: 0.0, max: (choices.len() - 1) as f32, default, choices, integer: false, unit: Unit::Plain }
}

pub(super) const fn i(name: &'static str, min: f32, max: f32, default: f32) -> ParamSpec {
    ParamSpec { name, min, max, default, choices: &[], integer: true, unit: Unit::Plain }
}

impl ParamSpec {
    pub(super) const fn unit(self, unit: Unit) -> Self {
        ParamSpec { unit, ..self }
    }

    /// Wide ranges of positive values, like frequencies, move in octaves.
    fn logarithmic(&self) -> bool {
        self.min > 0.0 && self.max / self.min >= 100.0
    }

    /// Envelope times get more room for short values.
    fn squared(&self) -> bool {
        self.unit == Unit::Seconds && !self.logarithmic()
    }

    /// Where `v` sits along the parameter's slider, 0..1.
    pub fn position(&self, v: f32) -> f32 {
        let v = v.clamp(self.min, self.max);
        let t = if self.logarithmic() {
            (v / self.min).ln() / (self.max / self.min).ln()
        } else {
            (v - self.min) / (self.max - self.min)
        };
        if self.squared() { t.sqrt() } else { t }
    }

    /// The value at `t` (0..1) along the parameter's slider.
    pub fn value_at(&self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let t = if self.squared() { t * t } else { t };
        let v = if self.logarithmic() {
            self.min * (self.max / self.min).powf(t)
        } else {
            self.min + t * (self.max - self.min)
        };
        if self.integer || !self.choices.is_empty() { v.round() } else { v }
    }

    /// The value as shown to the user, with its unit.
    pub fn format(&self, v: f32) -> String {
        if !self.choices.is_empty() {
            return self.choices[(v.round().max(0.0) as usize).min(self.choices.len() - 1)].to_string();
        }
        match self.unit {
            Unit::Plain if self.integer => format!("{}", v.round() as i32),
            Unit::Plain => format!("{v:.2}"),
            Unit::Gain if v <= 0.0 => "-inf dB".into(),
            Unit::Gain => format!("{:+.1} dB", 20.0 * v.log10()),
            Unit::Hz if v >= 1000.0 => format!("{:.2} kHz", v / 1000.0),
            Unit::Hz if v < 10.0 => format!("{v:.2} Hz"),
            Unit::Hz => format!("{v:.0} Hz"),
            Unit::Seconds if v < 1.0 => format!("{:.0} ms", v * 1000.0),
            Unit::Seconds => format!("{v:.2} s"),
            Unit::Percent => format!("{:.0}%", v * 100.0),
            Unit::Semitones if self.integer => format!("{:+} st", v.round() as i32),
            Unit::Semitones => format!("{v:.2} st"),
            Unit::Cents => format!("{v:+.0} ct"),
            Unit::Lines => format!("{v:.2} lines"),
            Unit::Pan if v.abs() < 0.005 => "Center".into(),
            Unit::Pan => format!("{:.0}{}", v.abs() * 100.0, if v < 0.0 { "L" } else { "R" }),
            Unit::Note => Note::On(v.round().clamp(0.0, 119.0) as u8).label(),
            Unit::Octaves => format!("{v:+.2} oct"),
            Unit::Vowel => {
                // The vowel, or the two it is between and how far along.
                let names = ['A', 'E', 'I', 'O', 'U'];
                let (k, t) = (v.clamp(0.0, 4.0).floor() as usize, v.clamp(0.0, 4.0).fract());
                match t {
                    t if t < 0.05 => names[k].to_string(),
                    t if t > 0.95 => names[k + 1].to_string(),
                    t => format!("{}–{} {:.0}%", names[k], names[k + 1], t * 100.0),
                }
            }
            Unit::Ratio if self.integer => format!("×{}", v.round() as i32),
            Unit::Ratio => format!("×{v:.2}"),
        }
    }
}

pub const WAVES: &[&str] = &["Saw", "Square", "Triangle", "Sine", "Noise"];
/// The Wavetable synth's tables: sine to triangle to saw to square, a
/// narrowing pulse, a hard-synced saw, a folded sine, and vowels.
pub const WAVETABLES: &[&str] = &["Basic", "Pulse", "Sync", "Fold", "Vocal"];
/// The Convolver's impulses: made-up spaces and a cabinet, or the sample
/// loaded into it.
pub const IMPULSES: &[&str] = &["Room", "Hall", "Plate", "Spring", "Cabinet", "Sample"];
pub const LOOP_MODES: &[&str] = &["Off", "Forward", "Backward", "Ping-pong"];
pub const FILTER_MODES: &[&str] = &["Lowpass", "Highpass", "Bandpass"];
pub const DISTORTION_TYPES: &[&str] = &["Soft clip", "Hard clip", "Wave fold"];
pub const LFO_MODES: &[&str] = &["Tremolo", "Auto-pan"];
pub const LFO_SHAPES: &[&str] = &["Sine", "Triangle", "Square", "Saw down", "Saw up"];
/// A Modulator's LFO shapes: the LFO's, and one drawn by hand.
pub const MODULATOR_SHAPES: &[&str] = &["Sine", "Triangle", "Square", "Saw down", "Saw up", "Drawn"];
/// The index of the drawn shape in `MODULATOR_SHAPES`.
pub const DRAWN_SHAPE: u32 = 5;
/// What a Modulator's cycle can be timed by: Hz, a period in lines, or a
/// length in beats.
pub const MODULATOR_SYNCS: &[&str] = &["Off (Hz)", "Lines", "Beats"];
/// The lengths in beats a synced Modulator's cycle takes, and their labels.
pub const MODULATOR_BEATS: &[&str] = &["1/16", "1/8", "1/4", "1/2", "1", "2", "4", "8", "16", "32"];
pub const MODULATOR_BEAT_LENGTHS: [f32; 10] = [0.0625, 0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];
/// The shape a drawn LFO starts as: up and down once, as (where in the
/// cycle, value 0..1).
pub const DEFAULT_SHAPE: [(f32, f32); 3] = [(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)];
pub const FLANGER_MODES: &[&str] = &["Flanger", "Chorus"];
pub const MULTISYNTH_MODES: &[&str] = &["All", "Round robin", "Random"];
/// When a Glide slides: into every note, or only one played on top of the
/// last, without a gap, which then doesn't start again.
pub const GLIDE_MODES: &[&str] = &["Always", "Legato"];
/// What a Modulator moves its parameters by: an LFO, or what it follows:
/// the level of its input, the last note played on it, or how hard; an
/// envelope each note starts; or just its Amount, as a knob that turns
/// them all.
pub const MODULATOR_MODES: &[&str] = &["LFO", "Follow input", "Key", "Velocity", "Envelope", "Manual"];
pub const REPEATER_HOLD: &[&str] = &["Off", "Hold"];
/// The Repeater's lengths, and how many lines each is.
pub const REPEATER_LENGTHS: &[&str] = &[
    "8 lines",
    "4 lines",
    "2 lines",
    "1 line",
    "1/2 line",
    "1/3 line",
    "1/4 line",
    "1/6 line",
    "1/8 line",
    "1/16 line",
];
pub const REPEATER_LINES: [f32; 10] = [8.0, 4.0, 2.0, 1.0, 0.5, 1.0 / 3.0, 0.25, 1.0 / 6.0, 0.125, 0.0625];

use Unit::{Cents, Gain, Hz, Lines, Pan, Percent, Ratio, Seconds, Semitones};

static OUTPUT_PARAMS: [ParamSpec; 1] = [p("Volume", 0.0, 1.5, 0.8).unit(Gain)];
static GENERATOR_PARAMS: [ParamSpec; 10] = [
    p("Volume", 0.0, 1.0, 0.5).unit(Gain),
    c("Wave", 0.0, WAVES),
    p("Attack", 0.0, 2.0, 0.005).unit(Seconds),
    p("Decay", 0.0, 2.0, 0.2).unit(Seconds),
    p("Sustain", 0.0, 1.0, 0.6).unit(Percent),
    p("Release", 0.0, 4.0, 0.2).unit(Seconds),
    p("Detune", -100.0, 100.0, 0.0).unit(Cents),
    i("Unison", 1.0, 4.0, 1.0),
    p("Pulse width", 0.05, 0.95, 0.5).unit(Percent),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
];
static WAVETABLE_PARAMS: [ParamSpec; 12] = [
    p("Volume", 0.0, 1.0, 0.5).unit(Gain),
    c("Table", 0.0, WAVETABLES),
    p("Position", 0.0, 1.0, 0.0).unit(Percent),
    p("Sweep", -1.0, 1.0, 0.0),
    i("Unison", 1.0, 7.0, 1.0),
    p("Detune", 0.0, 100.0, 15.0).unit(Cents),
    p("Stereo", 0.0, 1.0, 0.5).unit(Percent),
    p("Attack", 0.0, 2.0, 0.005).unit(Seconds),
    p("Decay", 0.0, 2.0, 0.3).unit(Seconds),
    p("Sustain", 0.0, 1.0, 0.7).unit(Percent),
    p("Release", 0.0, 4.0, 0.3).unit(Seconds),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
];
static GRANULAR_PARAMS: [ParamSpec; 11] = [
    p("Volume", 0.0, 1.0, 0.6).unit(Gain),
    p("Position", 0.0, 1.0, 0.0).unit(Percent),
    p("Scan", -2.0, 2.0, 0.25),
    p("Size", 0.01, 0.5, 0.08).unit(Seconds),
    p("Density", 1.0, 200.0, 30.0).unit(Hz),
    p("Spray", 0.0, 0.5, 0.02).unit(Seconds),
    p("Random pitch", 0.0, 100.0, 0.0).unit(Cents),
    p("Stereo", 0.0, 1.0, 0.5).unit(Percent),
    p("Reverse", 0.0, 1.0, 0.0).unit(Percent),
    p("Attack", 0.0, 4.0, 0.05).unit(Seconds),
    p("Release", 0.0, 8.0, 0.5).unit(Seconds),
];
static CONVOLVER_PARAMS: [ParamSpec; 3] =
    [c("Impulse", 0.0, IMPULSES), p("Mix", 0.0, 1.0, 0.3).unit(Percent), p("Gain", 0.0, 2.0, 1.0).unit(Gain)];
static FM_PARAMS: [ParamSpec; 9] = [
    p("Volume", 0.0, 1.0, 0.5).unit(Gain),
    p("Ratio", 0.25, 8.0, 2.0).unit(Ratio),
    p("Mod index", 0.0, 10.0, 2.0),
    p("Mod decay", 0.0, 4.0, 0.4).unit(Seconds),
    p("Feedback", 0.0, 1.0, 0.0).unit(Percent),
    p("Attack", 0.0, 2.0, 0.002).unit(Seconds),
    p("Decay", 0.0, 4.0, 0.6).unit(Seconds),
    p("Sustain", 0.0, 1.0, 0.0).unit(Percent),
    p("Release", 0.0, 4.0, 0.3).unit(Seconds),
];
static DRUM_PARAMS: [ParamSpec; 5] = [
    p("Volume", 0.0, 1.0, 0.7).unit(Gain),
    p("Kick tone", 30.0, 120.0, 50.0).unit(Hz),
    p("Kick decay", 0.05, 1.5, 0.45).unit(Seconds),
    p("Snare tone", 0.0, 1.0, 0.5).unit(Percent),
    p("Hat decay", 0.01, 0.5, 0.06).unit(Seconds),
];
pub const KICKER_WAVES: &[&str] = &["Sine", "Triangle", "Square"];
static KICKER_PARAMS: [ParamSpec; 8] = [
    p("Volume", 0.0, 1.0, 0.7).unit(Gain),
    c("Wave", 0.0, KICKER_WAVES),
    p("Pitch drop", 0.0, 6.0, 3.0).unit(Unit::Octaves),
    p("Sweep", 0.005, 0.5, 0.04).unit(Seconds),
    p("Attack", 0.0, 0.05, 0.0).unit(Seconds),
    p("Decay", 0.05, 3.0, 0.5).unit(Seconds),
    p("Boost", 0.0, 1.0, 0.2).unit(Percent),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
];
static SPECTRAVOICE_PARAMS: [ParamSpec; 11] = [
    p("Volume", 0.0, 1.0, 0.4).unit(Gain),
    i("Harmonics", 1.0, 32.0, 12.0),
    p("Slope", 0.0, 3.0, 1.0),
    p("Even", 0.0, 1.0, 1.0).unit(Percent),
    p("Stretch", -0.02, 0.02, 0.0).unit(Percent),
    p("Shimmer", 0.0, 1.0, 0.0).unit(Percent),
    p("Attack", 0.0, 4.0, 0.01).unit(Seconds),
    p("Decay", 0.0, 4.0, 0.3).unit(Seconds),
    p("Sustain", 0.0, 1.0, 0.7).unit(Percent),
    p("Release", 0.0, 8.0, 0.4).unit(Seconds),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
];
/// The FMX's algorithms: how its four operators feed each other, as the
/// classic four-operator FM synths have them. `>` is "modulates"; an
/// operator that modulates none is heard.
pub const FMX_ALGORITHMS: &[&str] = &[
    "4 > 3 > 2 > 1",
    "3 + 4 > 2 > 1",
    "4 > 1, 3 > 2 > 1",
    "4 > 3 > 1, 2 > 1",
    "4 > 3, 2 > 1: two stacks",
    "4 > 1, 2, 3",
    "4 > 3, 1, 2",
    "1, 2, 3, 4 side by side",
];
static FMX_PARAMS: [ParamSpec; 27] = [
    p("Volume", 0.0, 1.0, 0.5).unit(Gain),
    c("Algorithm", 0.0, FMX_ALGORITHMS),
    p("Feedback", 0.0, 1.0, 0.0).unit(Percent),
    p("Op1 level", 0.0, 1.0, 1.0).unit(Percent),
    p("Op1 ratio", 0.25, 16.0, 1.0).unit(Ratio),
    p("Op1 attack", 0.0, 4.0, 0.005).unit(Seconds),
    p("Op1 decay", 0.0, 8.0, 1.2).unit(Seconds),
    p("Op1 sustain", 0.0, 1.0, 0.5).unit(Percent),
    p("Op1 release", 0.0, 8.0, 0.5).unit(Seconds),
    p("Op2 level", 0.0, 1.0, 0.45).unit(Percent),
    p("Op2 ratio", 0.25, 16.0, 1.0).unit(Ratio),
    p("Op2 attack", 0.0, 4.0, 0.001).unit(Seconds),
    p("Op2 decay", 0.0, 8.0, 0.8).unit(Seconds),
    p("Op2 sustain", 0.0, 1.0, 0.2).unit(Percent),
    p("Op2 release", 0.0, 8.0, 0.4).unit(Seconds),
    p("Op3 level", 0.0, 1.0, 0.3).unit(Percent),
    p("Op3 ratio", 0.25, 16.0, 2.0).unit(Ratio),
    p("Op3 attack", 0.0, 4.0, 0.001).unit(Seconds),
    p("Op3 decay", 0.0, 8.0, 0.5).unit(Seconds),
    p("Op3 sustain", 0.0, 1.0, 0.1).unit(Percent),
    p("Op3 release", 0.0, 8.0, 0.3).unit(Seconds),
    p("Op4 level", 0.0, 1.0, 0.2).unit(Percent),
    p("Op4 ratio", 0.25, 16.0, 3.0).unit(Ratio),
    p("Op4 attack", 0.0, 4.0, 0.001).unit(Seconds),
    p("Op4 decay", 0.0, 8.0, 0.3).unit(Seconds),
    p("Op4 sustain", 0.0, 1.0, 0.0).unit(Percent),
    p("Op4 release", 0.0, 8.0, 0.2).unit(Seconds),
];
/// Instrument-wide settings; each sample has its own in `SampleSlot`.
static SAMPLER_PARAMS: [ParamSpec; 7] = [
    p("Volume", 0.0, 1.5, 0.8).unit(Gain),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
    i("Transpose", -48.0, 48.0, 0.0).unit(Semitones),
    p("Attack", 0.0, 2.0, 0.0).unit(Seconds),
    p("Decay", 0.0, 4.0, 0.5).unit(Seconds),
    p("Sustain", 0.0, 1.0, 1.0).unit(Percent),
    p("Release", 0.0, 4.0, 0.1).unit(Seconds),
];
static FILTER_PARAMS: [ParamSpec; 5] = [
    c("Mode", 0.0, FILTER_MODES),
    p("Cutoff", 20.0, 20000.0, 2000.0).unit(Hz),
    p("Resonance", 0.0, 0.97, 0.3).unit(Percent),
    p("LFO rate", 0.0, 20.0, 0.0).unit(Hz),
    p("LFO depth", 0.0, 1.0, 0.0).unit(Percent),
];
static DISTORTION_PARAMS: [ParamSpec; 6] = [
    p("Drive", 1.0, 50.0, 4.0).unit(Gain),
    p("Tone", 0.0, 1.0, 0.7).unit(Percent),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
    c("Type", 0.0, DISTORTION_TYPES),
    i("Bits", 1.0, 16.0, 16.0),
    i("Downsample", 1.0, 32.0, 1.0).unit(Ratio),
];
static DELAY_PARAMS: [ParamSpec; 4] = [
    p("Time", 0.25, 16.0, 3.0).unit(Lines),
    p("Feedback", 0.0, 0.95, 0.4).unit(Percent),
    p("Mix", 0.0, 1.0, 0.3).unit(Percent),
    p("Stereo", 0.0, 1.0, 0.5).unit(Percent),
];
static REVERB_PARAMS: [ParamSpec; 3] = [
    p("Room size", 0.0, 1.0, 0.7).unit(Percent),
    p("Damping", 0.0, 1.0, 0.4).unit(Percent),
    p("Mix", 0.0, 1.0, 0.25).unit(Percent),
];
static AMP_PARAMS: [ParamSpec; 3] =
    [p("Volume", 0.0, 4.0, 1.0).unit(Gain), p("Pan", -1.0, 1.0, 0.0).unit(Pan), c("Invert", 0.0, &["Off", "On"])];
static LFO_PARAMS: [ParamSpec; 6] = [
    c("Mode", 0.0, LFO_MODES),
    c("Shape", 0.0, LFO_SHAPES),
    p("Depth", 0.0, 1.0, 0.5).unit(Percent),
    p("Rate", 0.05, 20.0, 2.0).unit(Hz),
    c("Sync", 0.0, &["Off", "Lines"]),
    p("Period", 0.25, 64.0, 4.0).unit(Lines),
];
static FLANGER_PARAMS: [ParamSpec; 6] = [
    c("Mode", 0.0, FLANGER_MODES),
    p("Delay", 0.0005, 0.03, 0.003).unit(Seconds),
    p("Depth", 0.0, 1.0, 0.5).unit(Percent),
    p("Rate", 0.02, 10.0, 0.3).unit(Hz),
    p("Feedback", -0.95, 0.95, 0.5).unit(Percent),
    p("Mix", 0.0, 1.0, 0.5).unit(Percent),
];
static PHASER_PARAMS: [ParamSpec; 7] = [
    p("Rate", 0.02, 10.0, 0.4).unit(Hz),
    p("Depth", 0.0, 1.0, 0.8).unit(Percent),
    p("Floor", 20.0, 5000.0, 250.0).unit(Hz),
    p("Ceiling", 200.0, 18000.0, 4000.0).unit(Hz),
    i("Stages", 1.0, 12.0, 4.0),
    p("Feedback", -0.95, 0.95, 0.4).unit(Percent),
    p("Mix", 0.0, 1.0, 0.5).unit(Percent),
];
static VOCAL_FILTER_PARAMS: [ParamSpec; 6] = [
    p("Vowel", 0.0, 4.0, 0.0).unit(Unit::Vowel),
    p("Shift", -12.0, 12.0, 0.0).unit(Semitones),
    p("Width", 0.25, 4.0, 1.0).unit(Ratio),
    i("Formants", 1.0, 5.0, 5.0),
    p("Gain", 0.0, 4.0, 2.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static REPEATER_PARAMS: [ParamSpec; 3] =
    [c("Hold", 0.0, REPEATER_HOLD), c("Length", 3.0, REPEATER_LENGTHS), p("Mix", 0.0, 1.0, 1.0).unit(Percent)];
static RING_MOD_PARAMS: [ParamSpec; 4] = [
    p("Frequency", 1.0, 5000.0, 440.0).unit(Hz),
    c("Shape", 0.0, LFO_SHAPES),
    p("Stereo", 0.0, 0.5, 0.0).unit(Percent),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static GATE_PARAMS: [ParamSpec; 5] = [
    p("Threshold", 0.001, 1.0, 0.05).unit(Gain),
    p("Attack", 0.0001, 0.1, 0.001).unit(Seconds),
    p("Hold", 0.0, 1.0, 0.05).unit(Seconds),
    p("Release", 0.005, 2.0, 0.1).unit(Seconds),
    p("Floor", 0.0, 1.0, 0.0).unit(Gain),
];
static PITCH_SHIFTER_PARAMS: [ParamSpec; 5] = [
    i("Pitch", -24.0, 24.0, 12.0).unit(Semitones),
    p("Finetune", -100.0, 100.0, 0.0).unit(Cents),
    p("Grain", 0.01, 0.2, 0.06).unit(Seconds),
    p("Feedback", 0.0, 0.9, 0.0).unit(Percent),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static STEREO_EXPANDER_PARAMS: [ParamSpec; 2] =
    [p("Width", 0.0, 2.0, 1.5).unit(Percent), p("Mono bass", 0.0, 500.0, 0.0).unit(Hz)];
static COMB_FILTER_PARAMS: [ParamSpec; 5] = [
    i("Note", 24.0, 108.0, 57.0).unit(Unit::Note),
    p("Finetune", -100.0, 100.0, 0.0).unit(Cents),
    p("Feedback", -0.98, 0.98, 0.8).unit(Percent),
    p("Damping", 0.0, 1.0, 0.2).unit(Percent),
    p("Mix", 0.0, 1.0, 0.5).unit(Percent),
];
static MAXIMIZER_PARAMS: [ParamSpec; 3] = [
    p("Boost", 1.0, 8.0, 2.0).unit(Gain),
    p("Ceiling", 0.1, 1.0, 0.95).unit(Gain),
    p("Release", 0.005, 1.0, 0.1).unit(Seconds),
];
static EXCITER_PARAMS: [ParamSpec; 3] = [
    p("Frequency", 1000.0, 16000.0, 3000.0).unit(Hz),
    p("Drive", 1.0, 10.0, 3.0).unit(Gain),
    p("Amount", 0.0, 1.0, 0.3).unit(Percent),
];
static DC_BLOCKER_PARAMS: [ParamSpec; 1] = [p("Cutoff", 1.0, 100.0, 10.0).unit(Hz)];
static SCREAM_FILTER_PARAMS: [ParamSpec; 5] = [
    c("Mode", 0.0, FILTER_MODES),
    p("Cutoff", 20.0, 20000.0, 1500.0).unit(Hz),
    p("Resonance", 0.0, 1.0, 0.7).unit(Percent),
    p("Scream", 1.0, 20.0, 4.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static MULTITAP_PARAMS: [ParamSpec; 14] = [
    p("Tap 1 time", 0.25, 16.0, 1.0).unit(Lines),
    p("Tap 1 level", 0.0, 1.0, 0.6).unit(Gain),
    p("Tap 1 pan", -1.0, 1.0, -0.6).unit(Pan),
    p("Tap 2 time", 0.25, 16.0, 2.0).unit(Lines),
    p("Tap 2 level", 0.0, 1.0, 0.45).unit(Gain),
    p("Tap 2 pan", -1.0, 1.0, 0.6).unit(Pan),
    p("Tap 3 time", 0.25, 16.0, 3.0).unit(Lines),
    p("Tap 3 level", 0.0, 1.0, 0.3).unit(Gain),
    p("Tap 3 pan", -1.0, 1.0, -0.3).unit(Pan),
    p("Tap 4 time", 0.25, 16.0, 4.0).unit(Lines),
    p("Tap 4 level", 0.0, 1.0, 0.2).unit(Gain),
    p("Tap 4 pan", -1.0, 1.0, 0.3).unit(Pan),
    p("Feedback", 0.0, 0.95, 0.3).unit(Percent),
    p("Mix", 0.0, 1.0, 0.4).unit(Percent),
];
/// The EQ 10's band centers, an octave apart.
pub const EQ10_FREQS: [f32; 10] = [31.25, 62.5, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
const fn band(name: &'static str) -> ParamSpec {
    p(name, 0.25, 4.0, 1.0).unit(Gain)
}
static EQ10_PARAMS: [ParamSpec; 10] = [
    band("31 Hz"),
    band("63 Hz"),
    band("125 Hz"),
    band("250 Hz"),
    band("500 Hz"),
    band("1 kHz"),
    band("2 kHz"),
    band("4 kHz"),
    band("8 kHz"),
    band("16 kHz"),
];
pub const CABINETS: &[&str] = &["Small combo", "British 4x12", "Bass 1x15", "Radio"];
static CABINET_PARAMS: [ParamSpec; 3] =
    [c("Cabinet", 0.0, CABINETS), p("Drive", 1.0, 10.0, 1.5).unit(Gain), p("Mix", 0.0, 1.0, 1.0).unit(Percent)];
static VIBRATO_PARAMS: [ParamSpec; 4] = [
    p("Rate", 0.1, 20.0, 5.0).unit(Hz),
    p("Depth", 0.0, 1.0, 0.3).unit(Percent),
    p("Stereo", 0.0, 0.5, 0.0).unit(Percent),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
pub const INPUT_CHANNELS: &[&str] = &["Stereo", "Left", "Right", "Mono"];
static INPUT_PARAMS: [ParamSpec; 2] = [p("Volume", 0.0, 4.0, 1.0).unit(Gain), c("Channels", 0.0, INPUT_CHANNELS)];
/// The WaveShaper's curve points: what each of these input levels comes out as.
static WAVESHAPER_PARAMS: [ParamSpec; 13] = [
    p("Input", 0.0, 4.0, 1.0).unit(Gain),
    c("Symmetric", 1.0, &["Off", "On"]),
    p("Output", 0.0, 2.0, 1.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
    p("At -100%", -1.0, 1.0, -1.0),
    p("At -75%", -1.0, 1.0, -0.75),
    p("At -50%", -1.0, 1.0, -0.5),
    p("At -25%", -1.0, 1.0, -0.25),
    p("At 0%", -1.0, 1.0, 0.0),
    p("At +25%", -1.0, 1.0, 0.25),
    p("At +50%", -1.0, 1.0, 0.5),
    p("At +75%", -1.0, 1.0, 0.75),
    p("At +100%", -1.0, 1.0, 1.0),
];
/// The Filter Pro's types, in the order of `Shape::ALL`.
pub const FILTER_PRO_TYPES: &[&str] =
    &["Lowpass", "Highpass", "Bandpass", "Notch", "Allpass", "Peak", "Low shelf", "High shelf"];
pub const FILTER_SLOPES: &[&str] = &["12 dB", "24 dB", "36 dB", "48 dB"];
static FILTER_PRO_PARAMS: [ParamSpec; 6] = [
    c("Type", 0.0, FILTER_PRO_TYPES),
    p("Frequency", 20.0, 20000.0, 1000.0).unit(Hz),
    p("Resonance", 0.3, 20.0, 0.707),
    p("Gain", 0.25, 4.0, 1.0).unit(Gain),
    c("Slope", 0.0, FILTER_SLOPES),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static CHORUS_PARAMS: [ParamSpec; 7] = [
    i("Voices", 1.0, 4.0, 3.0),
    p("Rate", 0.05, 5.0, 0.8).unit(Hz),
    p("Depth", 0.0, 1.0, 0.5).unit(Percent),
    p("Delay", 0.005, 0.04, 0.015).unit(Seconds),
    p("Stereo", 0.0, 0.5, 0.25).unit(Percent),
    p("Feedback", 0.0, 0.8, 0.0).unit(Percent),
    p("Mix", 0.0, 1.0, 0.5).unit(Percent),
];
static ECHO_PARAMS: [ParamSpec; 5] = [
    p("Time", 0.01, 2.0, 0.3).unit(Seconds),
    p("Feedback", 0.0, 0.95, 0.5).unit(Percent),
    p("Damping", 0.0, 1.0, 0.3).unit(Percent),
    p("Stereo", 0.0, 1.0, 0.3).unit(Percent),
    p("Mix", 0.0, 1.0, 0.35).unit(Percent),
];
pub const LADDER_TYPES: &[&str] = &["Lowpass 24 dB", "Lowpass 12 dB", "Bandpass", "Highpass 24 dB"];
static ANALOG_FILTER_PARAMS: [ParamSpec; 5] = [
    c("Type", 0.0, LADDER_TYPES),
    p("Cutoff", 20.0, 18000.0, 1200.0).unit(Hz),
    p("Resonance", 0.0, 1.0, 0.3).unit(Percent),
    p("Drive", 1.0, 10.0, 1.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static PLATE_PARAMS: [ParamSpec; 5] = [
    p("Decay", 0.0, 0.98, 0.6).unit(Percent),
    p("Predelay", 0.0, 0.2, 0.02).unit(Seconds),
    p("Damping", 0.0, 1.0, 0.3).unit(Percent),
    p("Width", 0.0, 1.0, 1.0).unit(Percent),
    p("Mix", 0.0, 1.0, 0.3).unit(Percent),
];
static EQ5_PARAMS: [ParamSpec; 15] = [
    p("Low freq", 20.0, 20000.0, 100.0).unit(Hz),
    p("Low gain", 0.25, 4.0, 1.0).unit(Gain),
    p("Low Q", 0.3, 10.0, 0.9),
    p("Low mid freq", 20.0, 20000.0, 400.0).unit(Hz),
    p("Low mid gain", 0.25, 4.0, 1.0).unit(Gain),
    p("Low mid Q", 0.3, 10.0, 0.9),
    p("Mid freq", 20.0, 20000.0, 1200.0).unit(Hz),
    p("Mid gain", 0.25, 4.0, 1.0).unit(Gain),
    p("Mid Q", 0.3, 10.0, 0.9),
    p("High mid freq", 20.0, 20000.0, 3500.0).unit(Hz),
    p("High mid gain", 0.25, 4.0, 1.0).unit(Gain),
    p("High mid Q", 0.3, 10.0, 0.9),
    p("High freq", 20.0, 20000.0, 9000.0).unit(Hz),
    p("High gain", 0.25, 4.0, 1.0).unit(Gain),
    p("High Q", 0.3, 10.0, 0.9),
];
static COMPRESSOR_PARAMS: [ParamSpec; 6] = [
    p("Threshold", 0.01, 1.0, 0.25).unit(Gain),
    p("Ratio", 1.0, 20.0, 4.0).unit(Ratio),
    p("Attack", 0.0001, 0.2, 0.005).unit(Seconds),
    p("Release", 0.01, 2.0, 0.15).unit(Seconds),
    p("Makeup", 1.0, 8.0, 1.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static EQ_PARAMS: [ParamSpec; 6] = [
    p("Low", 0.0, 4.0, 1.0).unit(Gain),
    p("Mid", 0.0, 4.0, 1.0).unit(Gain),
    p("High", 0.0, 4.0, 1.0).unit(Gain),
    p("Low freq", 20.0, 2000.0, 200.0).unit(Hz),
    p("Mid freq", 100.0, 10000.0, 1000.0).unit(Hz),
    p("High freq", 500.0, 20000.0, 5000.0).unit(Hz),
];
static MULTISYNTH_PARAMS: [ParamSpec; 8] = [
    c("Mode", 0.0, MULTISYNTH_MODES),
    i("Transpose", -48.0, 48.0, 0.0).unit(Semitones),
    p("Finetune", -100.0, 100.0, 0.0).unit(Cents),
    p("Random pitch", 0.0, 100.0, 0.0).unit(Cents),
    p("Velocity", 0.0, 2.0, 1.0).unit(Percent),
    p("Random velocity", 0.0, 1.0, 0.0).unit(Percent),
    i("Low note", 0.0, 119.0, 0.0).unit(Unit::Note),
    i("High note", 0.0, 119.0, 119.0).unit(Unit::Note),
];
pub const ANALOG_WAVES: &[&str] = &["Saw", "Square", "Triangle", "Sine"];
pub const VOICE_MODES: &[&str] = &["Poly", "Mono", "Legato"];
static ANALOG_PARAMS: [ParamSpec; 34] = [
    p("Volume", 0.0, 1.0, 0.5).unit(Gain),
    c("Osc 1", 0.0, ANALOG_WAVES),
    c("Osc 2", 0.0, ANALOG_WAVES),
    i("Osc 2 pitch", -24.0, 24.0, 0.0).unit(Semitones),
    p("Osc 2 detune", -50.0, 50.0, 7.0).unit(Cents),
    p("Osc mix", 0.0, 1.0, 0.5).unit(Percent),
    p("Sub", 0.0, 1.0, 0.0).unit(Percent),
    p("Noise", 0.0, 1.0, 0.0).unit(Percent),
    p("Pulse width", 0.05, 0.95, 0.5).unit(Percent),
    c("Filter", 0.0, LADDER_TYPES),
    p("Cutoff", 20.0, 20000.0, 900.0).unit(Hz),
    p("Resonance", 0.0, 1.0, 0.3).unit(Percent),
    p("Env amount", -8.0, 8.0, 3.0).unit(Unit::Octaves),
    p("Key track", 0.0, 1.0, 0.5).unit(Percent),
    p("Velocity", 0.0, 4.0, 1.0).unit(Unit::Octaves),
    p("Filter attack", 0.0, 4.0, 0.002).unit(Seconds),
    p("Filter decay", 0.005, 4.0, 0.35).unit(Seconds),
    p("Filter sustain", 0.0, 1.0, 0.2).unit(Percent),
    p("Filter release", 0.005, 4.0, 0.3).unit(Seconds),
    p("Attack", 0.0, 4.0, 0.003).unit(Seconds),
    p("Decay", 0.005, 4.0, 0.4).unit(Seconds),
    p("Sustain", 0.0, 1.0, 0.7).unit(Percent),
    p("Release", 0.005, 4.0, 0.25).unit(Seconds),
    c("LFO shape", 0.0, LFO_SHAPES),
    p("LFO rate", 0.05, 20.0, 5.0).unit(Hz),
    p("LFO to pitch", 0.0, 12.0, 0.0).unit(Semitones),
    p("LFO to cutoff", 0.0, 4.0, 0.0).unit(Unit::Octaves),
    p("LFO to width", 0.0, 0.45, 0.0).unit(Percent),
    p("Env to pitch", -24.0, 24.0, 0.0).unit(Semitones),
    c("Voices", 0.0, VOICE_MODES),
    p("Glide", 0.0, 2.0, 0.0).unit(Seconds),
    i("Unison", 1.0, 4.0, 1.0),
    p("Spread", 0.0, 50.0, 12.0).unit(Cents),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
];
static PLUCKED_STRING_PARAMS: [ParamSpec; 9] = [
    p("Volume", 0.0, 1.0, 0.5).unit(Gain),
    p("Position", 0.02, 0.5, 0.15).unit(Percent),
    p("Brightness", 0.0, 1.0, 0.7).unit(Percent),
    p("Decay", 0.05, 30.0, 4.0).unit(Seconds),
    p("Damping", 0.0, 0.95, 0.3).unit(Percent),
    p("Release", 0.01, 4.0, 0.3).unit(Seconds),
    i("Strings", 1.0, 3.0, 1.0),
    p("Detune", 0.0, 30.0, 6.0).unit(Cents),
    p("Pan", -1.0, 1.0, 0.0).unit(Pan),
];
static VOCODER_PARAMS: [ParamSpec; 9] = [
    i("Bands", 4.0, 32.0, 16.0),
    p("Low", 50.0, 1000.0, 120.0).unit(Hz),
    p("High", 1000.0, 12000.0, 7000.0).unit(Hz),
    p("Sharpness", 1.0, 20.0, 6.0),
    p("Attack", 0.0005, 0.1, 0.004).unit(Seconds),
    p("Release", 0.005, 0.5, 0.05).unit(Seconds),
    p("Noise", 0.0, 1.0, 0.0).unit(Percent),
    p("Gain", 0.0, 8.0, 2.0).unit(Gain),
    p("Mix", 0.0, 1.0, 1.0).unit(Percent),
];
static GLIDE_PARAMS: [ParamSpec; 2] = [c("Mode", 0.0, GLIDE_MODES), p("Time", 0.005, 2.0, 0.12).unit(Seconds)];
static MODULATOR_PARAMS: [ParamSpec; 9] = [
    c("Mode", 0.0, MODULATOR_MODES),
    c("Shape", 0.0, MODULATOR_SHAPES),
    p("Rate", 0.05, 20.0, 1.0).unit(Hz),
    c("Sync", 0.0, MODULATOR_SYNCS),
    p("Period", 0.25, 64.0, 16.0).unit(Lines),
    p("Amount", -1.0, 1.0, 0.5).unit(Percent),
    p("Attack", 0.001, 1.0, 0.01).unit(Seconds),
    p("Release", 0.01, 2.0, 0.2).unit(Seconds),
    c("Beats", 4.0, MODULATOR_BEATS),
];
/// How many macros an instrument has: knobs that each move several of its
/// and its effects' parameters at once.
pub const MACROS: usize = 8;
const fn macro_knob(name: &'static str) -> ParamSpec {
    p(name, 0.0, 1.0, 0.0).unit(Percent)
}
static MACRO_PARAMS: [ParamSpec; MACROS] = [
    macro_knob("Macro 1"),
    macro_knob("Macro 2"),
    macro_knob("Macro 3"),
    macro_knob("Macro 4"),
    macro_knob("Macro 5"),
    macro_knob("Macro 6"),
    macro_knob("Macro 7"),
    macro_knob("Macro 8"),
];
/// The pan control of a mixer strip.
pub static MIXER_PAN: ParamSpec = p("Pan", -1.0, 1.0, 0.0).unit(Pan);
/// The fader of a mixer strip, as automation sees it.
pub static MIXER_GAIN: ParamSpec = p("Fader", 0.0, 2.0, 1.0).unit(Gain);

impl ModuleKind {
    pub const ADDABLE: [ModuleKind; 49] = [
        ModuleKind::Generator,
        ModuleKind::Analog,
        ModuleKind::PluckedString,
        ModuleKind::Wavetable,
        ModuleKind::Fm,
        ModuleKind::Drums,
        ModuleKind::Kicker,
        ModuleKind::SpectraVoice,
        ModuleKind::Fmx,
        ModuleKind::Sampler,
        ModuleKind::Granular,
        ModuleKind::Input,
        ModuleKind::MultiSynth,
        ModuleKind::Glide,
        ModuleKind::Modulator,
        ModuleKind::Filter,
        ModuleKind::Distortion,
        ModuleKind::Delay,
        ModuleKind::Reverb,
        ModuleKind::Amplifier,
        ModuleKind::Lfo,
        ModuleKind::Flanger,
        ModuleKind::Phaser,
        ModuleKind::VocalFilter,
        ModuleKind::Vocoder,
        ModuleKind::Repeater,
        ModuleKind::RingMod,
        ModuleKind::Gate,
        ModuleKind::PitchShifter,
        ModuleKind::StereoExpander,
        ModuleKind::CombFilter,
        ModuleKind::Maximizer,
        ModuleKind::Exciter,
        ModuleKind::DcBlocker,
        ModuleKind::ScreamFilter,
        ModuleKind::Multitap,
        ModuleKind::Eq10,
        ModuleKind::Cabinet,
        ModuleKind::Vibrato,
        ModuleKind::WaveShaper,
        ModuleKind::FilterPro,
        ModuleKind::Chorus,
        ModuleKind::Echo,
        ModuleKind::AnalogFilter,
        ModuleKind::PlateReverb,
        ModuleKind::Convolver,
        ModuleKind::Eq5,
        ModuleKind::Compressor,
        ModuleKind::Eq,
    ];

    /// The kind's name and its parameters.
    fn info(self) -> (&'static str, &'static [ParamSpec]) {
        match self {
            ModuleKind::Output => ("Output", &OUTPUT_PARAMS),
            ModuleKind::Generator => ("Generator", &GENERATOR_PARAMS),
            ModuleKind::Wavetable => ("Wavetable", &WAVETABLE_PARAMS),
            ModuleKind::Analog => ("Analog Synth", &ANALOG_PARAMS),
            ModuleKind::PluckedString => ("Plucked String", &PLUCKED_STRING_PARAMS),
            ModuleKind::Fm => ("FM", &FM_PARAMS),
            ModuleKind::Drums => ("Drums", &DRUM_PARAMS),
            ModuleKind::Sampler => ("Sampler", &SAMPLER_PARAMS),
            ModuleKind::Granular => ("Granular", &GRANULAR_PARAMS),
            ModuleKind::Filter => ("Filter", &FILTER_PARAMS),
            ModuleKind::Distortion => ("Distortion", &DISTORTION_PARAMS),
            ModuleKind::Delay => ("Delay", &DELAY_PARAMS),
            ModuleKind::Reverb => ("Reverb", &REVERB_PARAMS),
            ModuleKind::Amplifier => ("Amplifier", &AMP_PARAMS),
            ModuleKind::Lfo => ("LFO", &LFO_PARAMS),
            ModuleKind::Flanger => ("Flanger", &FLANGER_PARAMS),
            ModuleKind::Phaser => ("Phaser", &PHASER_PARAMS),
            ModuleKind::VocalFilter => ("Vocal Filter", &VOCAL_FILTER_PARAMS),
            ModuleKind::Vocoder => ("Vocoder", &VOCODER_PARAMS),
            ModuleKind::Repeater => ("Repeater", &REPEATER_PARAMS),
            ModuleKind::RingMod => ("Ring Mod", &RING_MOD_PARAMS),
            ModuleKind::Gate => ("Gate", &GATE_PARAMS),
            ModuleKind::Kicker => ("Kicker", &KICKER_PARAMS),
            ModuleKind::SpectraVoice => ("SpectraVoice", &SPECTRAVOICE_PARAMS),
            ModuleKind::Fmx => ("FMX", &FMX_PARAMS),
            ModuleKind::PitchShifter => ("Pitch Shifter", &PITCH_SHIFTER_PARAMS),
            ModuleKind::StereoExpander => ("Stereo Expander", &STEREO_EXPANDER_PARAMS),
            ModuleKind::CombFilter => ("Comb Filter", &COMB_FILTER_PARAMS),
            ModuleKind::Maximizer => ("Maximizer", &MAXIMIZER_PARAMS),
            ModuleKind::Exciter => ("Exciter", &EXCITER_PARAMS),
            ModuleKind::DcBlocker => ("DC Blocker", &DC_BLOCKER_PARAMS),
            ModuleKind::ScreamFilter => ("Scream Filter", &SCREAM_FILTER_PARAMS),
            ModuleKind::Multitap => ("Multitap Delay", &MULTITAP_PARAMS),
            ModuleKind::Eq10 => ("EQ 10", &EQ10_PARAMS),
            ModuleKind::Cabinet => ("Cabinet Simulator", &CABINET_PARAMS),
            ModuleKind::Vibrato => ("Vibrato", &VIBRATO_PARAMS),
            ModuleKind::Input => ("Input", &INPUT_PARAMS),
            ModuleKind::WaveShaper => ("WaveShaper", &WAVESHAPER_PARAMS),
            ModuleKind::FilterPro => ("Filter Pro", &FILTER_PRO_PARAMS),
            ModuleKind::Chorus => ("Chorus", &CHORUS_PARAMS),
            ModuleKind::Echo => ("Echo", &ECHO_PARAMS),
            ModuleKind::AnalogFilter => ("Analog Filter", &ANALOG_FILTER_PARAMS),
            ModuleKind::PlateReverb => ("Plate Reverb", &PLATE_PARAMS),
            ModuleKind::Convolver => ("Convolver", &CONVOLVER_PARAMS),
            ModuleKind::Eq5 => ("EQ 5", &EQ5_PARAMS),
            ModuleKind::Compressor => ("Compressor", &COMPRESSOR_PARAMS),
            ModuleKind::Eq => ("EQ", &EQ_PARAMS),
            ModuleKind::MultiSynth => ("MultiSynth", &MULTISYNTH_PARAMS),
            ModuleKind::Glide => ("Glide", &GLIDE_PARAMS),
            ModuleKind::Modulator => ("Modulator", &MODULATOR_PARAMS),
        }
    }

    pub fn name(self) -> &'static str {
        self.info().0
    }

    pub fn params(self) -> &'static [ParamSpec] {
        self.info().1
    }

    /// What envelopes can move: the module's parameters, then its mixer
    /// strip's fader and pan, then an instrument's macros.
    pub fn automatable(self, i: usize) -> Option<&'static ParamSpec> {
        let n = self.params().len();
        match i {
            i if i < n => Some(&self.params()[i]),
            i if i == n => Some(&MIXER_GAIN),
            i if i == n + 1 => Some(&MIXER_PAN),
            i if self.plays_sound() => MACRO_PARAMS.get(i - n - 2),
            _ => None,
        }
    }

    /// How many parameters `automatable` knows.
    pub fn num_automatable(self) -> usize {
        self.params().len() + 2 + if self.plays_sound() { MACROS } else { 0 }
    }

    /// Whether the module plays notes (as opposed to processing audio).
    pub fn is_instrument(self) -> bool {
        matches!(
            self,
            ModuleKind::Generator
                | ModuleKind::Wavetable
                | ModuleKind::Analog
                | ModuleKind::PluckedString
                | ModuleKind::Fm
                | ModuleKind::Drums
                | ModuleKind::Kicker
                | ModuleKind::SpectraVoice
                | ModuleKind::Fmx
                | ModuleKind::Sampler
                | ModuleKind::Granular
                | ModuleKind::MultiSynth
                | ModuleKind::Glide
        )
    }

    /// Whether the module plays samples the instrument editor loads and
    /// edits.
    pub fn holds_samples(self) -> bool {
        matches!(self, ModuleKind::Sampler | ModuleKind::Granular)
    }

    /// Whether the module is an instrument that makes sound of its own, not
    /// one that only passes notes on: it has macros, presets and stems.
    pub fn plays_sound(self) -> bool {
        self.is_instrument() && !self.notes_only()
    }

    /// Whether the module listens to another module's sound, its key
    /// input, to decide what to do with its own.
    pub fn takes_key(self) -> bool {
        matches!(self, ModuleKind::Compressor | ModuleKind::Gate | ModuleKind::Vocoder)
    }

    /// Whether the module's voices take a `Modulation`.
    pub fn has_modulation(self) -> bool {
        matches!(self, ModuleKind::Sampler | ModuleKind::Generator | ModuleKind::Fm | ModuleKind::Wavetable)
    }

    /// Whether the module only passes notes on and makes no sound, as the
    /// MultiSynth does.
    pub fn notes_only(self) -> bool {
        matches!(self, ModuleKind::MultiSynth | ModuleKind::Glide)
    }

    /// Whether the module's links move other modules' parameters rather
    /// than carry sound.
    pub fn controls(self) -> bool {
        self == ModuleKind::Modulator
    }

    /// Whether the module outputs sound, and so has a mixer strip.
    pub fn makes_sound(self) -> bool {
        !self.notes_only() && !self.controls()
    }

    pub fn has_input(self) -> bool {
        !self.is_instrument() && self != ModuleKind::Input
    }

    pub fn has_output(self) -> bool {
        self != ModuleKind::Output
    }
}
