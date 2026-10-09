//! The sample recorder: records the sound device's input,
//! or the song as it plays, into a new sample of a Sampler.

use super::{App, theme};
use crate::audio::{self, Input};
use crate::dsp::Frame;
use crate::engine::{Cmd, Shared, Tape};
use crate::project::{ModuleKind, SampleSlot};
use crate::sample::Sample;
use eframe::egui::{self, RichText};
use std::sync::Arc;

/// Seconds of room the tape has between two frames of the UI.
const TAPE_SECONDS: usize = 4;
/// The longest recording, in seconds.
const MAX_SECONDS: f32 = 600.0;
/// Below this a frame counts as silence, for trimming (-60 dB).
const SILENCE: f32 = 0.001;

#[derive(Clone, Copy, PartialEq, Default)]
pub enum Source {
    /// The sound device's input: a microphone or line in.
    #[default]
    Input,
    /// What the song plays, after the master volume.
    Song,
}

pub struct Recorder {
    pub open: bool,
    source: Source,
    /// Plays the song from the start when recording it starts, and stops
    /// recording when it stops.
    from_start: bool,
    /// Takes away the silence before the sound starts.
    trim: bool,
    take: Option<Take>,
    /// Recordings made so far, to name the next one.
    count: usize,
    error: Option<String>,
}

impl Default for Recorder {
    fn default() -> Self {
        Self { open: false, source: Source::Input, from_start: true, trim: true, take: None, count: 0, error: None }
    }
}

/// A recording in progress.
struct Take {
    frames: Vec<Frame>,
    sample_rate: u32,
    /// The Sampler the recording goes to.
    module: u8,
    source: Source,
    /// The input and the tape it records onto, kept open while recording
    /// it; the song is recorded from the engine's own tape.
    input: Option<(Input, Arc<Tape>)>,
    /// Frames were lost along the way.
    dropped: bool,
    /// Whether the song has been seen playing, so its end ends the take.
    started: bool,
}

impl Take {
    fn tape<'a>(&'a self, shared: &'a Shared) -> &'a Tape {
        self.input.as_ref().map_or(&shared.resample, |(_, tape)| tape)
    }

    fn seconds(&self) -> f32 {
        self.frames.len() as f32 / self.sample_rate as f32
    }

    /// The loudest of the last tenth of a second, for the meter.
    fn level(&self) -> f32 {
        let tail = self.frames.len().saturating_sub(self.sample_rate as usize / 10);
        self.frames[tail..].iter().fold(0f32, |m, f| m.max(f[0].abs()).max(f[1].abs()))
    }
}

/// The frames from the first one louder than silence; all of them if
/// there is none.
pub fn trim_start(frames: &[Frame]) -> &[Frame] {
    let start = frames.iter().position(|f| f[0].abs().max(f[1].abs()) > SILENCE).unwrap_or(0);
    &frames[start..]
}

pub fn window(app: &mut App, ctx: &egui::Context) {
    if app.recorder.take.is_some() {
        collect(app);
        ctx.request_repaint();
    }
    if !app.recorder.open {
        return;
    }
    let mut open = true;
    let window = egui::Window::new("Sample Recorder")
        .open(&mut open)
        .resizable(false)
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center());
    window.show(ctx, |ui| body(app, ui));
    if !open {
        cancel(app);
        app.recorder.open = false;
    }
}

fn body(app: &mut App, ui: &mut egui::Ui) {
    let target = app.instrument().filter(|&id| app.project.module(id).is_some_and(|m| m.kind == ModuleKind::Sampler));
    let recording = app.recorder.take.is_some();
    ui.add_enabled_ui(!recording, |ui| {
        ui.horizontal(|ui| {
            theme::caption(ui, "SOURCE");
            let r = &mut app.recorder;
            ui.radio_value(&mut r.source, Source::Input, "Audio input")
                .on_hover_text("The sound device's input: a microphone or line in");
            ui.radio_value(&mut r.source, Source::Song, "Song output")
                .on_hover_text("What the song plays, to make a sample of it (resampling)");
        });
        let r = &mut app.recorder;
        if r.source == Source::Song {
            ui.checkbox(&mut r.from_start, "Play the song from the start, and stop with it");
        }
        ui.checkbox(&mut r.trim, "Trim the silence before the sound starts");
    });
    ui.add_space(4.0);
    match &app.recorder.take {
        Some(take) => {
            let source = if take.source == Source::Input { "input" } else { "song" };
            let text = format!("Recording the {source}: {:.1} s", take.seconds());
            ui.label(RichText::new(text).color(theme::RECORD));
            let level = take.level();
            ui.add(egui::ProgressBar::new(level.min(1.0)).desired_height(6.0).desired_width(260.0));
        }
        None => {
            let to = match target.and_then(|id| app.project.module(id)) {
                Some(m) => format!("Records into {}, as a new sample", m.name),
                None => "Select a Sampler instrument to record into".into(),
            };
            ui.label(RichText::new(to).small().color(theme::TEXT_WEAK));
        }
    }
    if let Some(e) = &app.recorder.error {
        ui.label(RichText::new(e).small().color(theme::RECORD));
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if recording {
            if ui.button("Stop").on_hover_text("Stop and keep the recording as a sample").clicked() {
                finish(app);
            }
            if ui.button("Cancel").on_hover_text("Stop and throw the recording away").clicked() {
                cancel(app);
            }
        } else if ui.add_enabled(target.is_some(), egui::Button::new("Start")).clicked() {
            start(app, target.unwrap());
        }
    });
}

fn start(app: &mut App, module: u8) {
    app.recorder.error = None;
    let source = app.recorder.source;
    let (input, sample_rate) = match source {
        Source::Input => {
            let tape = Arc::new(Tape::default());
            match audio::open_input(tape.clone()) {
                Ok(input) => {
                    app.set_status(format!("Recording from {}", input.name));
                    let sr = input.sample_rate;
                    (Some((input, tape)), sr)
                }
                Err(e) => {
                    app.recorder.error = Some(format!("Can't open the input: {e}"));
                    return;
                }
            }
        }
        Source::Song => match app.audio.as_ref() {
            Some(a) => (None, a.sample_rate),
            None => {
                app.recorder.error = Some("There is no sound device playing the song".into());
                return;
            }
        },
    };
    let take = Take { frames: Vec::new(), sample_rate, module, source, input, dropped: false, started: false };
    take.tape(&app.shared).start(TAPE_SECONDS * sample_rate as usize);
    if source == Source::Song && app.recorder.from_start {
        app.send(Cmd::Play { order: 0, line: 0, loop_pattern: false });
    }
    app.recorder.take = Some(take);
}

/// Moves what the tape took onto the take, and ends it when it is long
/// enough or the song it records has stopped.
fn collect(app: &mut App) {
    let playing = app.shared.playing.load(std::sync::atomic::Ordering::Relaxed);
    let Some(take) = app.recorder.take.as_mut() else { return };
    let mut frames = std::mem::take(&mut take.frames);
    take.dropped |= take.tape(&app.shared).take(&mut frames);
    take.frames = frames;
    let song_ended = take.source == Source::Song && app.recorder.from_start && take.started && !playing;
    take.started |= playing;
    if song_ended || take.seconds() >= MAX_SECONDS {
        finish(app);
    }
}

fn stop_tape(app: &mut App) -> Option<Take> {
    let mut take = app.recorder.take.take()?;
    let mut frames = std::mem::take(&mut take.frames);
    let tape = take.tape(&app.shared);
    tape.stop();
    take.dropped |= tape.take(&mut frames);
    take.frames = frames;
    if take.source == Source::Song && app.recorder.from_start {
        app.send(Cmd::Stop);
    }
    Some(take)
}

fn cancel(app: &mut App) {
    if stop_tape(app).is_some() {
        app.set_status("Recording thrown away");
    }
}

/// Stops recording and adds what was recorded as a new sample.
fn finish(app: &mut App) {
    let Some(take) = stop_tape(app) else { return };
    let frames = if app.recorder.trim { trim_start(&take.frames) } else { &take.frames[..] };
    if frames.is_empty() {
        app.set_status("Nothing was recorded");
        return;
    }
    let Some(m) = app.project.module_mut(take.module) else {
        app.set_status("The Sampler being recorded into is gone, so the recording was thrown away");
        return;
    };
    app.recorder.count += 1;
    let name = format!("Recording {}", app.recorder.count);
    let sample = Sample { name, sample_rate: take.sample_rate as f32, channels: 2, frames: frames.to_vec() };
    let secs = sample.frames.len() as f32 / sample.sample_rate;
    let mut slot = SampleSlot::new(sample, None);
    slot.unsaved = true;
    m.samples.push(slot);
    app.sampler.sample = m.samples.len() - 1;
    app.sampler.slice = None;
    app.mark();
    let lost = if take.dropped { " (some of it was lost: the computer was too busy)" } else { "" };
    app.set_status(format!("Recorded {secs:.1} s into a new sample{lost}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimming_starts_at_the_first_sound() {
        let frames = [[0.0; 2], [0.0005, 0.0], [0.0, -0.2], [0.0; 2]];
        assert_eq!(trim_start(&frames), &frames[2..]);
        let silent = [[0.0; 2]; 3];
        assert_eq!(trim_start(&silent).len(), 3, "silence is kept rather than lost");
    }

    #[test]
    fn the_tape_keeps_what_fits_and_says_when_it_lost_some() {
        let tape = Tape::default();
        tape.push(&[[1.0; 2]; 4]);
        let mut got = Vec::new();
        assert!(!tape.take(&mut got) && got.is_empty(), "off, it takes nothing");
        tape.start(3);
        tape.push(&[[0.5; 2]; 2]);
        assert!(!tape.take(&mut got));
        assert_eq!(got, [[0.5; 2]; 2]);
        tape.push(&[[0.25; 2]; 4]);
        assert!(tape.take(&mut got), "a push past the room is reported");
        assert_eq!(got.len(), 5, "three of them fit");
    }
}
