mod automation;
mod backup;
mod block;
mod browser;
mod chain;
mod comments;
mod files;
mod frames;
mod help;
mod icons;
mod instruments;
mod jobs;
mod library;
mod menu;
mod mixer;
mod modulation;
mod modules;
mod pattern;
mod phrase;
mod presets;
mod recorder;
mod routing;
mod sampler;
mod sequencer;
mod settings;
#[cfg(test)]
mod shots;
mod song;
mod soundfonts;
mod spectrum;
mod theme;
mod trackscopes;
mod transport;
mod waveview;
mod widgets;

use crate::audio::{self, Audio};
use crate::engine::{Cmd, Garbage, LIVE_KEY, Shared};
use crate::project::{ModuleKind, OUTPUT_ID, Pattern, Project, SampleSlot};
use crate::project_dir;
use crate::sample::Sample;
use eframe::egui::{self, Color32, Key, Pos2, Rect, RichText, Stroke, Vec2};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{self, Receiver, Sender};

pub use pattern::Cursor;

const UNDO_LIMIT: usize = 200;

/// Makes a demo song.
pub type DemoSong = fn() -> Project;

/// The demo songs built into the program, by name. Concrete Hymn opens
/// when noise starts without a song.
pub const DEMOS: [(&str, DemoSong); 5] = [
    ("Last Light", Project::demo),
    ("Static Heart", Project::static_heart),
    ("Clockwork Rain", Project::clockwork_rain),
    ("Prism Overdrive", Project::prism_overdrive),
    ("Concrete Hymn", Project::concrete_hymn),
];

/// Opening `path`: a project's `.noise` file is imported, anything else
/// opened.
fn open_or_import(path: String) -> Pending {
    if project_dir::is_archive(std::path::Path::new(&path)) { Pending::Import(path) } else { Pending::Open(path) }
}

/// Where a song opened from `path` is saved, and whether it still needs a
/// place: a project folder is saved where it is; a song saved on its own
/// before projects were folders, or a backup, becomes a project folder of
/// its own, named after it, beside it (or for a backup, where songs
/// usually go).
fn song_location(path: &std::path::Path) -> (String, bool) {
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let backup = backup::dir().is_some_and(|d| full.starts_with(d));
    match project_dir::folder_of(&full).filter(|_| !backup) {
        Some(dir) => (dir.to_string_lossy().into_owned(), false),
        None => {
            let name = full.file_name().map_or("song".into(), |n| n.to_string_lossy().into_owned());
            let stem = name.trim_end_matches(".json").trim_end_matches(".noise");
            let dir = if backup {
                crate::paths::default_dir()
            } else {
                full.parent().map(std::path::Path::to_path_buf).unwrap_or_default()
            };
            (dir.join(stem).to_string_lossy().into_owned(), true)
        }
    }
}

/// A sample playing straight to the output.
pub struct Preview {
    pub sample: Arc<Sample>,
    /// The sampler, sample slot and first frame it was cut from, if any.
    pub source: Option<(u8, usize, usize)>,
}

/// What the middle of the window shows.
#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Pattern,
    Mixer,
    Sampler,
}

impl ScopeView {
    const ALL: [(ScopeView, &'static str, &'static str); 4] = [
        (ScopeView::Scope, "Scope", "The output's waveform"),
        (ScopeView::Tracks, "Tracks", "A scope for each track's notes, before effects"),
        (ScopeView::Spectrum, "Spectrum", "The output's frequencies"),
        (ScopeView::Wave, "Wave", "The samples that are playing"),
    ];
}

impl View {
    const ALL: [(View, &'static str, &'static str); 3] =
        [(View::Pattern, "Pattern Editor", "F2"), (View::Mixer, "Mixer", "F3"), (View::Sampler, "Instrument", "F4")];
}

/// What the lower frame shows.
#[derive(Clone, Copy, PartialEq)]
pub enum Lower {
    Modules,
    Automation,
    /// The track effects and the master chain.
    TrackFx,
}

/// Whose effects the Track FX tab shows.
#[derive(Clone, Copy, PartialEq)]
pub enum TrackFx {
    Track(usize),
    Master,
}

/// What the scope box of the upper frame shows.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ScopeView {
    Scope,
    /// A scope per track.
    Tracks,
    Spectrum,
    /// The samples that are playing.
    Wave,
}

/// What holds a note being previewed: a key of the computer keyboard or a
/// note of a MIDI keyboard.
#[derive(Clone, Copy, PartialEq)]
pub enum Held {
    Key(Key),
    Midi(u8),
}

/// Something that replaces the song, waiting until unsaved changes are
/// saved or dropped.
enum Pending {
    New,
    Open(String),
    /// A project's `.noise` file, to unpack beside it and open.
    Import(String),
    /// One of `DEMOS`.
    Demo(usize),
    Quit,
}

pub struct App {
    pub project: Project,
    ctx: egui::Context,
    tx: Sender<Cmd>,
    garbage: Receiver<Garbage>,
    shared: Arc<Shared>,
    audio: Option<Audio>,
    /// The sound card's input, open while the song has an Input module.
    live_input: Option<audio::Input>,
    /// Opening it failed; not tried again until no Input module is left.
    live_input_failed: bool,
    status: String,
    /// The song's project folder: where it was opened from, or while it is
    /// untitled, where Save As suggests.
    path: String,
    /// The song hasn't been saved to `path` or opened from it.
    untitled: bool,
    backups: backup::Backups,
    /// The hashes of the samples' audio, which name their files.
    hashes: project_dir::Hashes,
    /// The render dialog, open for a song or its stems.
    render_dialog: Option<files::Purpose>,
    /// The sample rate renders are made at, or 0 for the device's.
    render_rate: u32,
    render_depth: audio::BitDepth,
    /// The song has changes that aren't saved.
    modified: bool,
    /// Asking whether to save before doing this.
    confirm: Option<Pending>,
    /// Done once the song has been saved under a name picked for it.
    after_save: Option<Pending>,
    quitting: bool,
    /// The window title last set.
    title: String,

    /// Index into the order list of the pattern being edited.
    pub slot: usize,
    pub cursor: Cursor,
    /// The block selected in the pattern editor, with the index of its
    /// pattern, and the corner the selection started from.
    selection: Option<(usize, block::Block)>,
    anchor: Option<(usize, usize)>,
    /// Pattern data cut or copied.
    clip: Option<block::Clip>,
    /// The track being renamed and the name typed so far.
    track_rename: Option<(usize, String)>,
    pub octave: u8,
    pub step: usize,
    /// The volume written with entered notes; 80 (full) writes none, as
    /// when the keyboard velocity is off.
    pub entry_vol: u8,
    pub edit_mode: bool,
    pub follow: bool,
    pub loop_pattern: bool,
    /// The block loop: the order position and lines it loops, and
    /// its size as a fraction of the pattern (0 for the selection).
    pub block_loop: Option<(usize, usize, usize)>,
    pub block_size: usize,
    metronome: bool,
    pub selected_module: Option<u8>,

    /// Keys currently held for live preview: (key, module, note).
    held: Vec<(Held, u8, u8)>,
    /// Notes written while recording with the song playing, by the key
    /// that plays them, with where they went: pattern, track and column.
    recorded: Vec<(Held, usize, usize, usize)>,
    /// Recording while playing puts notes on the nearest multiple of this
    /// many lines; 0 is off, and keeps their timing in the delay column.
    record_quantize: usize,
    /// Releasing a key while recording writes a note-off.
    record_note_offs: bool,
    pub midi: crate::midi::Midi,
    show_midi: bool,
    /// The MIDI inputs found when the MIDI window was opened.
    midi_ports: Vec<String>,
    /// Write the velocity of MIDI notes into the volume column.
    pub midi_velocity: bool,

    // Undo is grouped per user gesture: `undo_base` holds the project as it
    // was before the gesture, and is pushed once the mouse is released.
    undo: Vec<Project>,
    redo: Vec<Project>,
    undo_base: Option<Project>,
    gesture_open: bool,
    changed: bool,
    audio_dirty: bool,

    pub lower: Lower,
    pub track_fx: TrackFx,
    /// The cursor's track when the Track FX tab last looked, so it follows
    /// the cursor to another track.
    fx_cursor: usize,
    automation: automation::AutomationView,
    pub instrument_list: instruments::InstrumentList,
    pub sequencer: sequencer::State,
    pub modulation: modulation::ModulationView,
    pub phrase: phrase::PhraseView,
    pub view: View,
    pub sampler: sampler::SamplerView,
    pub recorder: recorder::Recorder,
    pub scope_view: ScopeView,
    waves: waveview::WaveView,
    spectrum: spectrum::SpectrumView,
    pub previewing: Option<Preview>,
    /// The file explorer, while it is open.
    file_dialog: Option<files::FileDialog>,
    preset_picker: Option<soundfonts::PresetPicker>,
    soundfont_loads: Vec<soundfonts::Loading>,
    /// Work going on on a thread of its own: a render, export or import.
    job: Option<jobs::Job>,
    browser: browser::Browser,
    /// Mouse wheel movement not yet turned into whole pattern lines.
    pub wheel: f32,
    /// Where the pattern editor last scrolled to show the cursor: its
    /// track, column and field.
    pub shown_cursor: Option<(usize, usize, usize)>,
    /// Until when (in the UI's time) the pattern editor is scrolling to
    /// the cursor, so the cursor doesn't follow that scroll back.
    pub scrolling_to_cursor: f64,
    /// Which frames and parts of the window are shown: the upper frame,
    /// the pattern sequencer and matrix, the instrument list and disk
    /// browser on the right, and groups of the transport.
    panels: settings::Panels,
    /// The transport's widths last frame: everything on one row, and the
    /// song and entry settings alone, which go to a second row when the
    /// window is too narrow for one.
    transport_width: f32,
    settings_width: f32,
    /// The lower frame's height, kept while it is closed.
    lower_height: f32,
    /// The lower frame was closed last frame, so it opens at `lower_height`.
    lower_reopen: bool,
    show_lower: bool,
    show_help: bool,
    help: help::HelpView,
    show_comments: bool,
    /// The preferences as last written, to write them again when they change.
    saved_settings: settings::Settings,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<String>) -> Self {
        Self::start(cc, path, true)
    }

    /// The app on `path` (or the demo), with the sound card when `sound`;
    /// without, as the screenshot test draws it.
    pub fn start(cc: &eframe::CreationContext<'_>, path: Option<String>, sound: bool) -> Self {
        setup_style(&cc.egui_ctx);
        let mut status = String::from("Ready");
        let (mut song_path, mut untitled) = ("song".to_string(), true);
        // A project's `.noise` file is unpacked beside it, and that opened.
        let opened = path.as_deref().map(|p| {
            let p = std::path::Path::new(p);
            let song = if project_dir::is_archive(p) { project_dir::import_beside(p)? } else { p.to_path_buf() };
            project_dir::open(&song).map(|(project, warnings)| (song, project, warnings))
        });
        let project = match opened {
            Some(Ok((song, project, warnings))) => {
                if !warnings.is_empty() {
                    status = warnings.join("; ");
                }
                (song_path, untitled) = song_location(&song);
                project
            }
            Some(Err(e)) => {
                status = format!("Could not open {}: {e}", path.as_deref().unwrap_or_default());
                Project::concrete_hymn()
            }
            None => Project::concrete_hymn(),
        };
        let (tx, rx) = mpsc::channel();
        let (gtx, garbage) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let audio = match sound.then(|| audio::start(Arc::new(project.clone()), rx, gtx, shared.clone())) {
            None => None,
            Some(Ok(a)) => Some(a),
            Some(Err(e)) => {
                status = format!("Audio unavailable: {e}");
                None
            }
        };
        let selected_module = project.modules.iter().find(|m| m.kind.is_instrument()).map(|m| m.id);
        let browse_dir = path
            .as_deref()
            .and_then(|p| std::fs::canonicalize(p).ok())
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(crate::paths::default_dir);
        let song_given = path.is_some();
        let mut app = Self {
            project,
            ctx: cc.egui_ctx.clone(),
            tx,
            garbage,
            shared,
            audio,
            live_input: None,
            live_input_failed: false,
            status,
            path: song_path,
            backups: backup::Backups::default(),
            hashes: project_dir::Hashes::default(),
            render_dialog: None,
            render_rate: 0,
            render_depth: audio::BitDepth::default(),
            untitled,
            modified: false,
            confirm: None,
            after_save: None,
            quitting: false,
            title: String::new(),
            slot: 0,
            cursor: Cursor::default(),
            selection: None,
            anchor: None,
            clip: None,
            track_rename: None,
            octave: 4,
            step: 1,
            entry_vol: 0x80,
            edit_mode: false,
            follow: true,
            loop_pattern: false,
            block_loop: None,
            block_size: 4,
            metronome: false,
            selected_module,
            held: Vec::new(),
            recorded: Vec::new(),
            record_quantize: 0,
            record_note_offs: true,
            midi: Default::default(),
            show_midi: false,
            midi_ports: Vec::new(),
            midi_velocity: true,
            undo: Vec::new(),
            redo: Vec::new(),
            undo_base: None,
            gesture_open: false,
            changed: false,
            audio_dirty: false,
            lower: Lower::Modules,
            track_fx: TrackFx::Track(0),
            fx_cursor: 0,
            automation: Default::default(),
            instrument_list: Default::default(),
            sequencer: Default::default(),
            modulation: Default::default(),
            phrase: Default::default(),
            view: View::Pattern,
            sampler: Default::default(),
            recorder: Default::default(),
            scope_view: ScopeView::Scope,
            waves: Default::default(),
            spectrum: Default::default(),
            previewing: None,
            file_dialog: None,
            preset_picker: None,
            soundfont_loads: Vec::new(),
            job: None,
            browser: browser::Browser::new(browse_dir),
            wheel: 0.0,
            shown_cursor: None,
            scrolling_to_cursor: 0.0,
            panels: Default::default(),
            transport_width: 0.0,
            settings_width: 0.0,
            lower_height: 270.0,
            lower_reopen: true,
            show_lower: false,
            show_help: false,
            help: Default::default(),
            show_comments: false,
            saved_settings: settings::Settings::default(),
        };
        // Preferences from the last session.
        if let Some(file) = settings::Settings::path() {
            let saved = settings::Settings::load_from(&file);
            app.apply_settings(&saved, song_given);
            app.saved_settings = app.settings();
        }
        app
    }

    /// Plays `sample` straight to the output.
    pub fn preview(&mut self, sample: Arc<Sample>, source: Option<(u8, usize, usize)>) {
        self.send(Cmd::Preview(Some(sample.clone())));
        self.previewing = Some(Preview { sample, source });
    }

    pub fn stop_preview(&mut self) {
        self.send(Cmd::Preview(None));
        self.previewing = None;
    }

    /// Tells the engine the browser's preview volume and looping.
    pub fn send_preview_settings(&self) {
        self.send(Cmd::PreviewSettings { volume: self.browser.volume, looping: self.browser.looping });
    }

    /// What the preview plays, for the browser's playback strip.
    fn now_playing(&self) -> Option<browser::NowPlaying> {
        let pos = self.preview_pos()?;
        let s = &self.previewing.as_ref()?.sample;
        let sr = s.sample_rate.max(1.0);
        Some(browser::NowPlaying { name: s.name.clone(), at: pos as f32 / sr, length: s.len() as f32 / sr })
    }

    /// Where the preview is in its sample, while it plays.
    pub fn preview_pos(&self) -> Option<f64> {
        let pos = f32::from_bits(self.shared.preview_pos.load(Ordering::Relaxed));
        (pos >= 0.0 && self.previewing.is_some()).then_some(pos as f64)
    }

    /// Peak levels of module `id` for the mixer meters.
    pub fn level(&self, id: u8) -> [f32; 2] {
        self.shared.level(id)
    }

    /// The value an envelope gives automatable parameter `param` of module
    /// `id` while the song plays, if one does.
    pub fn automated(&self, id: u8, param: usize) -> Option<f32> {
        let live = self.shared.automated.lock().ok()?;
        // A Modulator's value comes after the envelope it moves on from.
        live.iter().rev().find(|a| a.0 == id && a.1 == param).map(|a| a.2)
    }

    /// The track scopes, `TRACK_SCOPE_LEN` frames a track.
    pub fn track_scopes(&self) -> Vec<f32> {
        self.shared.track_scopes.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn playheads(&self) -> Vec<crate::dsp::Playhead> {
        self.shared.playheads.lock().map(|p| p.clone()).unwrap_or_default()
    }

    /// The phrases playing, as (module, phrase, line playing).
    pub fn playing_phrases(&self) -> Vec<(u8, usize, usize)> {
        self.shared.phrases.lock().map(|p| p.clone()).unwrap_or_default()
    }

    pub fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }

    /// Marks the project as edited: recorded for undo and sent to the engine.
    pub fn mark(&mut self) {
        self.changed = true;
        self.audio_dirty = true;
        self.modified = true;
    }

    /// Marks an edit that the audio engine doesn't care about (e.g. layout).
    pub fn mark_layout(&mut self) {
        self.changed = true;
        self.modified = true;
    }

    pub fn current_pattern_index(&self) -> usize {
        self.project.order.get(self.slot).map_or(0, |s| s.pattern)
    }

    pub fn pattern(&self) -> &Pattern {
        &self.project.patterns[self.current_pattern_index()]
    }

    pub fn pattern_mut(&mut self) -> &mut Pattern {
        let i = self.current_pattern_index();
        &mut self.project.patterns[i]
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn play_position(&self) -> (usize, usize) {
        (self.shared.order.load(Ordering::Relaxed), self.shared.line.load(Ordering::Relaxed))
    }

    /// How far into its line playback is, 0..1.
    pub fn play_line_frac(&self) -> f32 {
        f32::from_bits(self.shared.line_frac.load(Ordering::Relaxed))
    }

    /// Plays the pattern shown from the cursor's line; while playing, jumps
    /// there.
    pub fn play_from_cursor(&mut self) {
        self.send(Cmd::Play { order: self.slot, line: self.cursor.line, loop_pattern: self.loop_pattern });
    }

    pub fn toggle_play(&mut self) {
        if self.is_playing() {
            self.send(Cmd::Stop);
        } else {
            let line = if self.loop_pattern || !self.follow { self.cursor.line } else { 0 };
            self.send(Cmd::Play { order: self.slot, line, loop_pattern: self.loop_pattern });
        }
    }

    /// Plays `note` on `module` while `key` is held, at `vel` (0..1).
    pub fn preview_on(&mut self, key: Held, module: u8, note: u8, vel: f32) {
        if self.held.iter().any(|h| h.0 == key) {
            return;
        }
        self.held.push((key, module, note));
        self.send(Cmd::NoteOn { module, key: LIVE_KEY + note as u32, note, vel });
    }

    pub fn preview_off(&mut self, key: Held) {
        pattern::record_release(self, key);
        if let Some(i) = self.held.iter().position(|h| h.0 == key) {
            let (_, module, note) = self.held.remove(i);
            self.send(Cmd::NoteOff { module, key: LIVE_KEY + note as u32 });
        }
    }

    /// The module new notes are written to.
    pub fn instrument(&self) -> Option<u8> {
        self.selected_module.filter(|id| self.project.module(*id).is_some_and(|m| m.kind.is_instrument()))
    }

    fn undo(&mut self) {
        if let Some(p) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.project, p));
            self.after_history();
        }
    }

    fn redo(&mut self) {
        if let Some(p) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.project, p));
            self.after_history();
        }
    }

    fn after_history(&mut self) {
        self.audio_dirty = true;
        self.modified = true;
        self.undo_base = None;
        self.gesture_open = false;
        self.clamp_cursor();
    }

    pub fn clamp_cursor(&mut self) {
        self.slot = self.slot.min(self.project.order.len().saturating_sub(1));
        let (lines, tracks) = (self.pattern().lines, self.pattern().num_tracks());
        self.cursor.line = self.cursor.line.min(lines - 1);
        self.cursor.track = self.cursor.track.min(tracks - 1);
        self.cursor.column = self.cursor.column.min(self.pattern().width(self.cursor.track) - 1);
    }

    fn handle_global_keys(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused()
            || self.file_dialog.is_some()
            || self.confirm.is_some()
            || self.preset_picker.is_some()
        {
            return;
        }
        // Keep egui from routing Space/Enter to whatever button was last clicked.
        ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
        let ctrl = |k| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, k));
        let ctrl_shift = |k| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, k));
        if ctrl_shift(Key::S) {
            self.pick_file(files::Purpose::SaveSong);
        }
        for (key, on) in [
            (Key::Num1, &mut self.panels.upper),
            (Key::Num2, &mut self.panels.sequencer),
            (Key::Num3, &mut self.show_lower),
            (Key::Num4, &mut self.panels.instruments),
            (Key::Num5, &mut self.panels.browser),
        ] {
            if ctrl(key) {
                *on = !*on;
            }
        }
        if ctrl(Key::S) {
            self.save_or_ask();
        }
        if ctrl(Key::O) {
            self.pick_file(files::Purpose::OpenSong);
        }
        if ctrl(Key::N) {
            self.request(Pending::New);
        }
        if ctrl(Key::Q) {
            self.request(Pending::Quit);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::F1)) {
            self.show_help = !self.show_help;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::F2)) {
            self.view = View::Pattern;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::F3)) {
            self.view = View::Mixer;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::F4)) {
            self.view = View::Sampler;
        }
        if ctrl(Key::ArrowUp) {
            instruments::step(self, -1);
        }
        if ctrl(Key::ArrowDown) {
            instruments::step(self, 1);
        }
        if ctrl_shift(Key::Z) || ctrl(Key::Y) {
            self.redo();
        }
        if ctrl(Key::Z) {
            self.undo();
        }
        pattern::handle_keys(self, ctx);
    }

    /// Plays and records the notes of the MIDI keyboard.
    fn handle_midi(&mut self) {
        for ev in self.midi.poll() {
            match ev {
                crate::midi::MidiEvent::NoteOn(note, vel) => pattern::midi_note(self, note, vel),
                crate::midi::MidiEvent::NoteOff(note) => self.preview_off(Held::Midi(note)),
            }
        }
    }

    /// The MIDI Input window: which keyboard to listen to.
    fn midi_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_midi;
        egui::Window::new("MIDI Input").open(&mut open).resizable(false).show(ctx, |ui| {
            let ports = self.midi_ports.clone();
            let current = self.midi.port.clone();
            ui.horizontal(|ui| {
                ui.label("Device");
                let mut pick = current.clone();
                egui::ComboBox::from_id_salt("midi_port")
                    .selected_text(current.as_deref().unwrap_or("None"))
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut pick, None, "None");
                        for p in &ports {
                            ui.selectable_value(&mut pick, Some(p.clone()), p);
                        }
                    });
                if icons::button(ui, icons::Icon::Refresh).on_hover_text("Look for MIDI inputs again").clicked() {
                    self.midi_ports = crate::midi::Midi::ports();
                }
                if pick != current {
                    match pick {
                        Some(name) => match self.midi.connect(&name) {
                            Ok(()) => self.set_status(format!("Listening to {name}")),
                            Err(e) => self.set_status(format!("MIDI: {e}")),
                        },
                        None => self.midi.disconnect(),
                    }
                }
            });
            if ports.is_empty() {
                ui.label(
                    RichText::new("No MIDI inputs found. Plug a keyboard in and press the refresh button.")
                        .color(theme::TEXT_WEAK),
                );
            }
            ui.checkbox(&mut self.midi_velocity, "Record velocity in the volume column");
            let hint = "Notes play the selected instrument, and in edit mode they are written to the pattern.";
            ui.label(RichText::new(hint).small().color(theme::TEXT_WEAK));
        });
        self.show_midi = open;
    }

    /// Names the song in the window title, with a star if it has unsaved changes.
    fn update_title(&mut self, ctx: &egui::Context) {
        // An unsaved song goes by its own title, if it has one.
        let name = match self.project.title.trim() {
            _ if !self.untitled => self.song_name(),
            "" => "Untitled".into(),
            title => title.to_string(),
        };
        let title = format!("{}{name} - noise", if self.modified { "*" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }

    /// Opens the sound card's input while the song has an Input module to
    /// play it, and closes it when none is left.
    fn update_live_input(&mut self) {
        let wanted = self.project.modules.iter().any(|m| m.kind == ModuleKind::Input);
        let tape = &self.shared.input;
        if !wanted {
            if self.live_input.take().is_some() {
                tape.stop();
            }
            self.live_input_failed = false;
            return;
        }
        if self.live_input.is_some() || self.live_input_failed {
            return;
        }
        let rate = self.audio.as_ref().map_or(48000, |a| a.sample_rate);
        tape.start(rate as usize / 4);
        match audio::open_input(tape.clone()) {
            Ok(input) => {
                tape.set_rate(input.sample_rate);
                self.set_status(format!("Listening to {}", input.name));
                self.live_input = Some(input);
            }
            Err(e) => {
                tape.stop();
                self.live_input_failed = true;
                self.set_status(format!("Can't open the sound card's input for the Input module: {e}"));
            }
        }
    }

    fn finish_frame(&mut self, ctx: &egui::Context) {
        self.update_live_input();
        self.save_settings();
        self.backup_tick();
        while self.garbage.try_recv().is_ok() {}

        if self.changed {
            self.gesture_open = true;
            self.redo.clear();
        }
        self.changed = false;
        let pointer_down = ctx.input(|i| i.pointer.any_down());
        if self.gesture_open && !pointer_down {
            if let Some(base) = self.undo_base.take() {
                self.undo.push(base);
                if self.undo.len() > UNDO_LIMIT {
                    self.undo.remove(0);
                }
            }
            self.gesture_open = false;
        } else if !self.gesture_open {
            self.undo_base = None;
        }

        if self.audio_dirty {
            self.audio_dirty = false;
            self.send(Cmd::Project(Arc::new(self.project.clone())));
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.undo_base.is_none() {
            self.undo_base = Some(self.project.clone());
        }
        jobs::poll(self);
        self.handle_global_keys(&ctx);
        self.handle_midi();
        self.handle_dropped_files(&ctx);

        if ctx.input(|i| i.viewport().close_requested()) && self.modified && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Pending::Quit);
        }

        let bar = egui::Frame::new().fill(theme::FRAME_BG).inner_margin(egui::Margin::symmetric(6, 1));
        egui::Panel::top("menu").frame(bar).show(ui, |ui| self.menu_bar(ui));
        let frame = egui::Frame::new().fill(theme::BODY).inner_margin(5);
        egui::Panel::top("transport").frame(frame).show(ui, |ui| self.transport(ui));
        egui::Panel::bottom("status")
            .frame(egui::Frame::new().fill(theme::FRAME_BG).inner_margin(egui::Margin::symmetric(6, 2)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if !jobs::status(self, ui) {
                        ui.label(&self.status);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(
                                "Notes: Z-M / Q-P · A = note off · Esc edit · Space play · -/= octave · F1 manual",
                            )
                            .color(theme::TEXT_WEAK),
                        );
                    });
                });
            });
        // The right column runs from the transport to the bottom, beside
        // the upper and lower frames.
        if self.panels.instruments || self.panels.browser {
            egui::Panel::right("right")
                .frame(frame)
                .resizable(true)
                .default_size(270.0)
                .min_size(200.0)
                .show(ui, |ui| self.right_column(ui));
        }
        if self.panels.upper {
            egui::Panel::top("upper").frame(frame).show(ui, |ui| self.upper_frame(ui));
        }
        let matrix_w = sequencer::matrix_width(self);
        if self.panels.sequencer {
            // Narrow with just the pattern numbers; the extended view has
            // its own width, for the names and the matrix.
            let panel = if self.panels.matrix {
                egui::Panel::left("sequencer_extended").default_size(210.0 + matrix_w).min_size(150.0 + matrix_w)
            } else {
                egui::Panel::left("sequencer").exact_size(sequencer::TOOLBAR_W + 92.0)
            };
            panel.frame(frame).show(ui, |ui| sequencer::panel(self, ui));
        }
        // The tabs stay at the bottom; the frame opens above them at the
        // height it had, however it was closed.
        egui::Panel::bottom("lower_tabs").frame(frame).resizable(false).show(ui, |ui| self.lower_tabs(ui));
        if self.show_lower {
            let panel = egui::Panel::bottom("lower").frame(frame).min_size(120.0);
            let panel = if self.lower_reopen {
                panel.exact_size(self.lower_height)
            } else {
                panel.resizable(true).default_size(self.lower_height)
            };
            let shown = panel.show(ui, |ui| self.lower_frame(ui));
            self.lower_height = shown.response.rect.height();
        }
        self.lower_reopen = !self.show_lower;
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            self.view_tabs(ui);
            match self.view {
                View::Pattern => pattern::editor(self, ui),
                View::Mixer => mixer::view(self, ui),
                View::Sampler => sampler::editor(self, ui),
            }
        });
        self.file_dialog(&ctx);
        self.render_dialog(&ctx);
        self.confirm_dialog(&ctx);
        help::window(&ctx, &mut self.show_help, &mut self.help);
        self.midi_window(&ctx);
        comments::window(self, &ctx);
        soundfonts::window(self, &ctx);
        recorder::window(self, &ctx);
        self.update_title(&ctx);

        self.finish_frame(&ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
}

fn setup_style(ctx: &egui::Context) {
    theme::setup(ctx);
}

/// The last part of `path`, for messages.
fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}
