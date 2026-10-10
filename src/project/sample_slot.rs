//! A Sampler's sample slots: a sample with its tuning, loop, keyzone and
//! slices.

use super::*;

/// How one slice of a sliced sample plays, on top of the sample's own
/// settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SliceSettings {
    /// Linear gain.
    pub volume: f32,
    pub panning: f32,
    pub transpose: i32,
    pub finetune: i32,
    /// Index into `LOOP_MODES`; the loop is the whole slice.
    pub loop_mode: u8,
    pub oneshot: bool,
}

impl Default for SliceSettings {
    fn default() -> Self {
        SliceSettings { volume: 1.0, panning: 0.0, transpose: 0, finetune: 0, loop_mode: 0, oneshot: false }
    }
}

/// One sample of a Sampler, with its own playback settings and keyzone.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SampleSlot {
    pub name: String,
    /// Where the audio is stored. Relative paths are resolved against the
    /// song file's folder.
    pub path: Option<String>,
    /// Linear gain, 0..4.
    pub volume: f32,
    pub panning: f32,
    /// Semitones.
    pub transpose: i32,
    /// Cents.
    pub finetune: i32,
    /// The note that plays the sample at its original pitch.
    pub base_note: u8,
    /// Index into `LOOP_MODES`.
    pub loop_mode: u8,
    /// Loop range in frames, end exclusive.
    pub loop_start: usize,
    pub loop_end: usize,
    /// The notes (inclusive) that play this sample.
    pub keys: [u8; 2],
    /// The velocities (inclusive, 0..=127) that play this sample.
    pub velocities: [u8; 2],
    /// Play the whole sample in this many lines at the song's tempo, with
    /// notes still moving its pitch; 0 is off. Beat sync.
    pub beat_sync: u16,
    /// Note-offs don't stop the sample.
    pub oneshot: bool,
    /// Starting a sample stops the others in the same group (1..=15) of
    /// this Sampler, as a closed hi-hat cuts an open one; 0 is none.
    pub mute_group: u8,
    /// Starting the song partway through plays the sample from where it
    /// would be by then, rather than not at all, for long samples such as
    /// loops and vocals.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub autoseek: bool,
    /// Slice markers in frames: the sample then plays
    /// whole on its base note only, and the slices from each marker to the
    /// next on the notes after it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<usize>,
    /// The settings of each slice, in order; missing ones are defaults.
    #[serde(skip_serializing_if = "all_default")]
    pub slice_settings: Vec<SliceSettings>,
    /// The audio. Shared, so cloning a project stays cheap.
    #[serde(skip)]
    pub data: Option<Arc<Sample>>,
    /// The audio was edited and has to be written out on save.
    #[serde(skip)]
    pub unsaved: bool,
}

impl Default for SampleSlot {
    fn default() -> Self {
        Self {
            name: "Sample".into(),
            path: None,
            volume: 1.0,
            panning: 0.0,
            transpose: 0,
            finetune: 0,
            base_note: 48,
            loop_mode: 0,
            loop_start: 0,
            loop_end: 0,
            keys: [0, 119],
            velocities: [0, 127],
            beat_sync: 0,
            oneshot: false,
            mute_group: 0,
            autoseek: false,
            slices: Vec::new(),
            slice_settings: Vec::new(),
            data: None,
            unsaved: false,
        }
    }
}

impl SampleSlot {
    pub fn new(sample: Sample, path: Option<String>) -> Self {
        Self {
            name: sample.name.clone(),
            path,
            loop_end: sample.len(),
            data: Some(Arc::new(sample)),
            ..Default::default()
        }
    }

    pub fn len(&self) -> usize {
        self.data.as_ref().map_or(0, |d| d.len())
    }

    /// The frames each slice plays, `start..end`: from the start of the
    /// sample, or a marker, to the next marker or the end. Empty without
    /// markers.
    pub fn slice_ranges(&self) -> Vec<(usize, usize)> {
        if self.slices.is_empty() {
            return Vec::new();
        }
        let len = self.len();
        let mut bounds: Vec<usize> = std::iter::once(0).chain(self.slices.iter().copied()).collect();
        bounds.push(len);
        bounds.windows(2).map(|w| (w[0], w[1])).filter(|r| r.0 < r.1).collect()
    }

    /// How slice `i` plays.
    pub fn slice(&self, i: usize) -> SliceSettings {
        self.slice_settings.get(i).cloned().unwrap_or_default()
    }

    /// Changes how slice `i` plays.
    pub fn slice_mut(&mut self, i: usize) -> &mut SliceSettings {
        if self.slice_settings.len() <= i {
            self.slice_settings.resize(i + 1, SliceSettings::default());
        }
        &mut self.slice_settings[i]
    }

    /// Adds a marker at `frame`, the slice it splits handing its settings
    /// to both halves, so the slices after it keep theirs.
    pub fn add_slice(&mut self, frame: usize) {
        if frame == 0 || frame >= self.len() || self.slices.contains(&frame) {
            return;
        }
        let k = self.slices.iter().filter(|&&m| m < frame).count();
        if k < self.slice_settings.len() {
            let split = self.slice_settings[k].clone();
            self.slice_settings.insert(k + 1, split);
        }
        self.slices.push(frame);
        self.clamp_slices();
    }

    /// Removes marker `n`, joining the slices on either side; the joined
    /// slice keeps the first one's settings.
    pub fn remove_slice(&mut self, n: usize) {
        if n < self.slices.len() {
            self.slices.remove(n);
            if n + 1 < self.slice_settings.len() {
                self.slice_settings.remove(n + 1);
            }
        }
    }

    /// The note slice `i` plays on.
    pub fn slice_note(&self, i: usize) -> u8 {
        (self.base_note as usize + 1 + i).min(119) as u8
    }

    /// Keeps the slice markers sorted, apart and inside the sample.
    pub fn clamp_slices(&mut self) {
        let len = self.len();
        self.slices.retain(|&s| s > 0 && (len == 0 || s < len));
        self.slices.sort_unstable();
        self.slices.dedup();
        // The notes after the base note run out at B-9.
        self.slices.truncate(119usize.saturating_sub(self.base_note as usize + 1));
        let n = if self.slices.is_empty() { 0 } else { self.slices.len() + 1 };
        self.slice_settings.truncate(n);
    }

    /// Keeps the loop inside the sample and at least one frame long.
    pub fn clamp_loop(&mut self) {
        let len = self.len();
        if len == 0 {
            return;
        }
        if self.loop_end == 0 || self.loop_end > len {
            self.loop_end = len;
        }
        self.loop_start = self.loop_start.min(self.loop_end - 1);
    }
}
