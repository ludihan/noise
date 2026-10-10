//! What the window and the audio thread pass each other: commands one way,
//! garbage to drop the other, and the meters, scopes and positions the
//! engine publishes.

use super::*;

pub enum Cmd {
    Project(Arc<Project>),
    /// Start playing at `order` / `line`. With `loop_pattern` the current
    /// pattern repeats instead of following the order list.
    Play {
        order: usize,
        line: usize,
        loop_pattern: bool,
    },
    Stop,
    NoteOn {
        module: u8,
        key: u32,
        note: u8,
        vel: f32,
    },
    NoteOff {
        module: u8,
        key: u32,
    },
    Panic,
    /// Play a sample straight to the output, as the disk browser does when
    /// a file is clicked. `None` stops the preview.
    Preview(Option<Arc<Sample>>),
    /// How loud previews play, and whether they loop until stopped.
    PreviewSettings {
        volume: f32,
        looping: bool,
    },
    /// Click on every beat while playing, higher on the first of a bar.
    Metronome(bool),
    /// Loop lines `from..=to` while the song plays position `order`, a
    /// block loop; `None` turns it off.
    BlockLoop(Option<(usize, usize, usize)>),
}

/// Things the audio thread hands back to the UI thread so they are freed
/// there rather than in the audio callback. The contents are never read.
#[allow(dead_code)]
pub enum Garbage {
    Project(Arc<Project>),
    Sample(Arc<Sample>),
}

/// Frames handed from an audio thread to the UI for recording, without the
/// audio thread waiting or allocating: it adds to a buffer made with room
/// for a few seconds, and the UI empties it every frame, keeping the room.
pub struct Tape {
    frames: Mutex<Vec<Frame>>,
    on: AtomicBool,
    /// Frames were lost: the buffer was full or busy.
    dropped: AtomicBool,
    /// The rate of the sound on it, when the one writing says.
    rate: AtomicU32,
}

impl Default for Tape {
    fn default() -> Self {
        Self {
            frames: Mutex::new(Vec::new()),
            on: AtomicBool::new(false),
            dropped: AtomicBool::new(false),
            rate: AtomicU32::new(0),
        }
    }
}

impl Tape {
    /// Starts taking frames, with room for `room` of them between reads.
    pub fn start(&self, room: usize) {
        *self.frames.lock().unwrap() = Vec::with_capacity(room);
        self.dropped.store(false, Ordering::Relaxed);
        self.on.store(true, Ordering::Release);
    }

    pub fn stop(&self) {
        self.on.store(false, Ordering::Release);
    }

    pub fn is_on(&self) -> bool {
        self.on.load(Ordering::Acquire)
    }

    /// Adds `frames` if the tape is on, as many as there is room for.
    pub fn push(&self, frames: &[Frame]) {
        if !self.is_on() {
            return;
        }
        let Ok(mut buf) = self.frames.try_lock() else {
            self.dropped.store(true, Ordering::Relaxed);
            return;
        };
        let room = buf.capacity() - buf.len();
        if frames.len() > room {
            self.dropped.store(true, Ordering::Relaxed);
        }
        buf.extend_from_slice(&frames[..frames.len().min(room)]);
    }

    pub fn set_rate(&self, rate: u32) {
        self.rate.store(rate, Ordering::Relaxed);
    }

    /// The sound's rate, or 0 if not known.
    pub fn rate(&self) -> u32 {
        self.rate.load(Ordering::Relaxed)
    }

    /// For a reader on another audio thread: moves the oldest frames into
    /// `out`, as many as there are, and returns how many. With more than
    /// `most` waiting, the oldest are dropped first, so the reader stays
    /// close behind the writer. Never waits.
    pub fn read(&self, out: &mut [Frame], most: usize) -> usize {
        let Ok(mut buf) = self.frames.try_lock() else { return 0 };
        if buf.len() > most {
            let late = buf.len() - most;
            buf.drain(..late);
        }
        let n = buf.len().min(out.len());
        out[..n].copy_from_slice(&buf[..n]);
        buf.drain(..n);
        n
    }

    /// Moves the frames taken so far onto `into`, and says whether any
    /// were lost since the tape started.
    pub fn take(&self, into: &mut Vec<Frame>) -> bool {
        into.append(&mut self.frames.lock().unwrap());
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Playback state published for the UI.
pub struct Shared {
    pub playing: AtomicBool,
    pub order: AtomicUsize,
    pub line: AtomicUsize,
    /// How far into that line playback is, 0..1, as f32 bits.
    pub line_frac: AtomicU32,
    pub bpm: AtomicU32,
    pub peak: [AtomicU32; 2],
    pub scope: Mutex<Vec<Frame>>,
    /// Where each sounding sampler voice is.
    pub playheads: Mutex<Vec<Playhead>>,
    /// Position of the disk browser preview in frames, or -1 when silent.
    pub preview_pos: AtomicU32,
    /// Seconds since playback started.
    pub time: AtomicU32,
    /// Share of the real time that rendering takes, 0..1.
    pub cpu: AtomicU32,
    /// Peak level of each module's output after its mixer settings, left
    /// and right, indexed by `2 * id + channel`.
    pub levels: Vec<AtomicU32>,
    /// The values envelopes give parameters while the song plays, as
    /// (module, automatable parameter, value).
    pub automated: Mutex<Vec<(u8, usize, f32)>>,
    /// What each track's notes played lately, before effects, mono:
    /// `TRACK_SCOPE_LEN` frames per track, oldest first.
    pub track_scopes: Mutex<Vec<f32>>,
    /// The phrases playing, as (module, phrase, line playing).
    pub phrases: Mutex<Vec<(u8, usize, usize)>>,
    /// The output, while the sample recorder records the song.
    pub resample: Tape,
    /// The sound card's input, while the song has an Input module.
    pub input: Arc<Tape>,
}

impl Shared {
    /// The peak levels of module `id`.
    pub fn level(&self, id: u8) -> [f32; 2] {
        let at = |ch: usize| f32::from_bits(self.levels[2 * id as usize + ch].load(Ordering::Relaxed));
        [at(0), at(1)]
    }
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            playing: AtomicBool::new(false),
            order: AtomicUsize::new(0),
            line: AtomicUsize::new(0),
            line_frac: AtomicU32::new(0),
            bpm: AtomicU32::new(0),
            peak: [AtomicU32::new(0), AtomicU32::new(0)],
            scope: Mutex::new(vec![[0.0; 2]; SCOPE_LEN]),
            playheads: Mutex::new(Vec::with_capacity(4 * MAX_PLAYHEADS)),
            preview_pos: AtomicU32::new((-1f32).to_bits()),
            time: AtomicU32::new(0),
            cpu: AtomicU32::new(0),
            levels: (0..512).map(|_| AtomicU32::new(0)).collect(),
            automated: Mutex::new(Vec::with_capacity(MAX_AUTOMATED)),
            track_scopes: Mutex::new(vec![0.0; MAX_TRACKS * TRACK_SCOPE_LEN]),
            phrases: Mutex::new(Vec::with_capacity(MAX_PHRASES)),
            resample: Tape::default(),
            input: Arc::default(),
        }
    }
}
