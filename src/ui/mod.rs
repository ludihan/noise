mod automation;
mod backup;
mod block;
mod browser;
mod chain;
mod comments;
mod files;
mod help;
mod icons;
mod instruments;
mod library;
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
mod soundfonts;
mod spectrum;
mod theme;
mod trackscopes;
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

/// The demo songs built into the program, by name: the first opens when
/// noise starts without a song.
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
    browser: browser::Browser,
    /// Mouse wheel movement not yet turned into whole pattern lines.
    pub wheel: f32,
    /// Where the pattern editor last scrolled to show the cursor: its
    /// track, column and field.
    pub shown_cursor: Option<(usize, usize, usize)>,
    /// Until when (in the UI's time) the pattern editor is scrolling to
    /// the cursor, so the cursor doesn't follow that scroll back.
    pub scrolling_to_cursor: f64,
    show_upper: bool,
    /// The disk browser below the instrument list, on the right.
    show_browser: bool,
    /// The other parts that can be hidden: the pattern
    /// sequencer on the left, the instrument list on the right, groups of
    /// the transport, and the lower frame folded to its tabs.
    show_sequencer: bool,
    show_instruments: bool,
    show_song_settings: bool,
    show_entry_settings: bool,
    show_cpu: bool,
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
    /// The pattern matrix beside the sequencer.
    show_matrix: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<String>) -> Self {
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
                Project::demo()
            }
            None => Project::demo(),
        };
        let (tx, rx) = mpsc::channel();
        let (gtx, garbage) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let audio = match audio::start(Arc::new(project.clone()), rx, gtx, shared.clone()) {
            Ok(a) => Some(a),
            Err(e) => {
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
            browser: browser::Browser::new(browse_dir),
            wheel: 0.0,
            shown_cursor: None,
            scrolling_to_cursor: 0.0,
            show_upper: true,
            show_browser: true,
            show_sequencer: true,
            show_instruments: true,
            show_song_settings: true,
            show_entry_settings: true,
            show_cpu: true,
            transport_width: 0.0,
            settings_width: 0.0,
            lower_height: 270.0,
            lower_reopen: true,
            show_lower: false,
            show_help: false,
            help: Default::default(),
            show_comments: false,
            show_matrix: false,
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

    /// Opens demo song `i` of `DEMOS` as a new, unsaved song.
    fn open_demo(&mut self, i: usize) {
        let Some(&(name, make)) = DEMOS.get(i) else { return };
        self.send(Cmd::Stop);
        self.project = make();
        self.selected_module = self.project.modules.iter().find(|m| m.kind.is_instrument()).map(|m| m.id);
        self.slot = 0;
        self.cursor = Cursor::default();
        self.song_replaced();
        let dir = self.song_dir().unwrap_or_default();
        self.path = dir.join(name).to_string_lossy().into_owned();
        self.untitled = true;
        self.status = format!("Opened the demo song {name}; press Space to play it");
    }

    fn new_project(&mut self) {
        self.send(Cmd::Stop);
        self.project = Project::empty();
        let id = self.project.add_module(ModuleKind::Generator, [100.0, 100.0]).unwrap();
        self.project.connect(id, crate::project::OUTPUT_ID);
        self.selected_module = Some(id);
        self.slot = 0;
        self.cursor = Cursor::default();
        self.song_replaced();
        // Keep the folder, so Save As starts there, but never the old song.
        let dir = self.song_dir().unwrap_or_default();
        self.path = dir.join("song").to_string_lossy().into_owned();
        self.untitled = true;
        self.status = "New song".into();
    }

    /// Sends a song that replaced the old one to the engine and starts the
    /// undo history over.
    fn song_replaced(&mut self) {
        self.block_loop = None;
        self.send(Cmd::BlockLoop(None));
        self.audio_dirty = true;
        self.modified = false;
        self.undo.clear();
        self.redo.clear();
        self.undo_base = None;
        self.gesture_open = false;
        self.changed = false;
    }

    /// Saves the song as its project folder, `path`.
    fn save(&mut self) {
        self.status = match project_dir::save(&mut self.project, std::path::Path::new(&self.path), &mut self.hashes) {
            Ok(saved) => {
                self.untitled = false;
                self.modified = false;
                let mut done = Vec::new();
                if saved.written > 0 {
                    done.push(format!("{} samples written", saved.written));
                }
                if saved.removed > 0 {
                    done.push(format!("{} no longer played deleted", saved.removed));
                }
                let done = if done.is_empty() { String::new() } else { format!(" ({})", done.join(", ")) };
                format!("Saved {}{done}", self.path)
            }
            Err(e) => format!("Save failed: {e}"),
        };
    }

    /// Opens the song at `path`: a project folder, or a song file.
    fn open(&mut self, path: String) {
        match project_dir::open(std::path::Path::new(&path)) {
            Ok((p, warnings)) => {
                self.send(Cmd::Stop);
                self.project = p;
                self.slot = 0;
                self.cursor = Cursor::default();
                self.selected_module = self.project.modules.iter().find(|m| m.kind.is_instrument()).map(|m| m.id);
                self.song_replaced();
                (self.path, self.untitled) = song_location(std::path::Path::new(&path));
                self.status = if warnings.is_empty() {
                    format!("Opened {path}")
                } else {
                    format!("Opened {path} with problems: {}", warnings.join("; "))
                };
                if self.untitled {
                    self.status.push_str("; Save As makes it a project folder");
                }
            }
            Err(e) => self.status = format!("Open failed: {e}"),
        }
    }

    /// Unpacks the project in the `.noise` file at `path` into a folder
    /// beside it, and opens it.
    fn import(&mut self, path: &str) {
        match project_dir::import_beside(std::path::Path::new(path)) {
            Ok(dir) => self.open(dir.to_string_lossy().into_owned()),
            Err(e) => self.status = format!("Import failed: {e}"),
        }
    }

    /// Writes the song, with every sample it plays, to the `.noise` file at
    /// `path`, as a project folder named after the song.
    fn export_project(&mut self, path: &str) {
        let name = self.song_name();
        self.status = match project_dir::export(&self.project, &name, std::path::Path::new(path), &mut self.hashes) {
            Ok(()) => format!("Exported the project to {path}"),
            Err(e) => format!("Export failed: {e}"),
        };
    }

    /// Does `p`, first asking about unsaved changes if there are any.
    fn request(&mut self, p: Pending) {
        if self.modified {
            self.confirm = Some(p);
        } else {
            self.perform(p);
        }
    }

    fn perform(&mut self, p: Pending) {
        match p {
            Pending::New => self.new_project(),
            Pending::Open(path) => self.open(path),
            Pending::Import(path) => self.import(&path),
            Pending::Demo(i) => self.open_demo(i),
            Pending::Quit => {
                self.quitting = true;
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// The "save changes?" dialog shown before `confirm` is done.
    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.confirm else { return };
        let doing = match pending {
            Pending::New => "starting a new song",
            Pending::Open(_) | Pending::Import(_) | Pending::Demo(_) => "opening another song",
            Pending::Quit => "quitting",
        };
        let (mut save, mut discard, mut cancel) = (false, false, false);
        let modal = egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Unsaved changes").heading().color(theme::SELECTED));
            ui.add_space(4.0);
            ui.label(format!("The song has changes that aren't saved. Save them before {doing}?"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                save = ui.button(if self.untitled { "Save As…" } else { "Save" }).clicked();
                discard = ui.button("Don't Save").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        cancel |= modal.should_close();
        if save {
            let pending = self.confirm.take().unwrap();
            if self.untitled {
                self.after_save = Some(pending);
                self.pick_file(files::Purpose::SaveSong);
            } else {
                self.save();
                if !self.modified {
                    self.perform(pending);
                }
            }
        } else if discard {
            let pending = self.confirm.take().unwrap();
            self.perform(pending);
        } else if cancel {
            self.confirm = None;
        }
    }

    /// Adds an audio file to sampler `id` as a new sample slot, or loads a
    /// soundfont into it in place of its samples.
    pub fn load_sample(&mut self, id: u8, path: &std::path::Path) -> bool {
        if crate::soundfont::is_soundfont(path) {
            return soundfonts::open(self, id, path);
        }
        match Sample::load(path) {
            Ok(sample) => {
                let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
                let secs = sample.seconds();
                let name = sample.name.clone();
                if let Some(m) = self.project.module_mut(id) {
                    if m.kind == ModuleKind::Convolver {
                        // Its one impulse, which it then plays.
                        m.samples.clear();
                        m.params[0] = (crate::project::IMPULSES.len() - 1) as f32;
                    } else if m.name == m.kind.name() && m.samples.is_empty() {
                        m.name = name.clone();
                    }
                    m.samples.push(SampleSlot::new(sample, Some(full.to_string_lossy().into_owned())));
                    self.sampler.sample = m.samples.len() - 1;
                    self.sampler.slice = None;
                }
                self.status = format!("Loaded {name} ({secs:.2}s)");
                self.mark();
                true
            }
            Err(e) => {
                self.status = format!("Could not load {}: {e}", path.display());
                false
            }
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let files: Vec<_> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        if !files.is_empty() {
            self.load_samples(files);
        }
    }

    /// Adds audio files to the selected Sampler, or to a new one wired to
    /// the output. Several files at once are laid out as a drum kit. A
    /// soundfont is loaded in place of the Sampler's samples.
    pub fn load_samples(&mut self, paths: Vec<std::path::PathBuf>) {
        let (fonts, paths): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| crate::soundfont::is_soundfont(p));
        let (wavs, other): (Vec<_>, Vec<_>) = paths.into_iter().partition(|p| crate::sample::is_audio(p));
        if let Some(p) = other.first() {
            self.status = format!("Only WAV, FLAC, Ogg and soundfont files can be loaded: {}", p.display());
        }
        if wavs.is_empty() && fonts.is_empty() {
            return;
        }
        let is_sampler = |id: &u8| self.project.module(*id).is_some_and(|m| m.kind.holds_samples());
        let selected = self.instrument().filter(is_sampler);
        let id = match selected {
            Some(id) => id,
            None => {
                instruments::add(self, ModuleKind::Sampler);
                match self.instrument() {
                    Some(id) => id,
                    None => return,
                }
            }
        };
        if let Some(font) = fonts.first() {
            soundfonts::open(self, id, font);
            return;
        }
        let mut loaded = 0;
        for path in &wavs {
            loaded += self.load_sample(id, path) as usize;
        }
        if loaded > 1 {
            sampler::drum_kit(&mut self.project.module_mut(id).unwrap().samples);
            self.status = format!("Loaded {loaded} samples as a drum kit from C-4");
        }
    }

    /// The format renders are written in: the one picked, at the sound
    /// device's rate unless a rate was picked.
    fn render_format(&self) -> audio::RenderFormat {
        let device = self.audio.as_ref().map_or(44100, |a| a.sample_rate);
        let rate = if self.render_rate == 0 { device } else { self.render_rate };
        audio::RenderFormat { sample_rate: rate, depth: self.render_depth }
    }

    fn export(&mut self, path: &str) {
        self.status = match audio::export_wav(Arc::new(self.project.clone()), path, self.render_format()) {
            Ok(()) => format!("Rendered {path}"),
            Err(e) => format!("Render failed: {e}"),
        };
    }

    fn export_stems(&mut self, path: &str) {
        self.status = match audio::export_stems(&self.project, path, self.render_format()) {
            Ok(files) => format!("Rendered {} stems next to {path}", files.len()),
            Err(e) => format!("Render failed: {e}"),
        };
    }

    /// The render dialog: the sample rate and format, before the file
    /// to render the song or its stems to.
    fn render_dialog(&mut self, ctx: &egui::Context) {
        let Some(purpose) = self.render_dialog else { return };
        let (mut go, mut cancel) = (false, false);
        let modal = egui::Modal::new(egui::Id::new("render")).show(ctx, |ui| {
            ui.set_width(320.0);
            let title = if purpose == files::Purpose::RenderStems { "Render Stems" } else { "Render Song" };
            ui.label(RichText::new(title).heading().color(theme::SELECTED));
            ui.add_space(4.0);
            egui::Grid::new("render_options").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Sample rate");
                let device = self.audio.as_ref().map_or(44100, |a| a.sample_rate);
                let name = |r: u32| if r == 0 { format!("As the device ({device} Hz)") } else { format!("{r} Hz") };
                egui::ComboBox::from_id_salt("render_rate").selected_text(name(self.render_rate)).width(180.0).show_ui(
                    ui,
                    |ui| {
                        for r in [0, 22050, 44100, 48000, 88200, 96000] {
                            ui.selectable_value(&mut self.render_rate, r, name(r));
                        }
                    },
                );
                ui.end_row();
                ui.label("Format");
                ui.horizontal(|ui| {
                    for d in audio::BitDepth::ALL {
                        if theme::toggle(ui, self.render_depth == d, d.name()).clicked() {
                            self.render_depth = d;
                        }
                    }
                });
                ui.end_row();
            });
            let note = if self.render_depth == audio::BitDepth::Float32 {
                "Float keeps peaks over 0 dB; integer formats are dithered."
            } else {
                "Dithered; peaks over 0 dB are clipped."
            };
            ui.label(RichText::new(note).small().color(theme::TEXT_WEAK));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                go = ui.button("Render…").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if go {
            self.render_dialog = None;
            self.pick_file(purpose);
        } else if cancel || modal.should_close() {
            self.render_dialog = None;
        }
    }

    /// The folder the song's project folder is in.
    fn song_dir(&self) -> Option<std::path::PathBuf> {
        let full = std::fs::canonicalize(&self.path).ok().or_else(|| std::path::absolute(&self.path).ok())?;
        full.parent().map(|d| d.to_path_buf())
    }

    /// Opens the file explorer to pick a file for `purpose`.
    pub fn pick_file(&mut self, purpose: files::Purpose) {
        let song_file = std::path::Path::new(&self.path).file_name().map(|n| n.to_string_lossy().into_owned());
        let name = match purpose {
            files::Purpose::SaveSong => song_file.unwrap_or_default(),
            files::Purpose::Render | files::Purpose::RenderStems => {
                format!("{}.wav", song_file.unwrap_or_else(|| "song".into()))
            }
            files::Purpose::ExportProject => {
                format!("{}.{}", song_file.unwrap_or_else(|| "song".into()), project_dir::ARCHIVE)
            }
            files::Purpose::OpenSong | files::Purpose::ImportProject | files::Purpose::LoadSample(_) => String::new(),
        };
        let dir = match purpose {
            files::Purpose::LoadSample(_) => Some(self.browser.dir().to_path_buf()),
            _ => self.song_dir(),
        };
        let dir = dir.unwrap_or_else(crate::paths::default_dir);
        self.file_dialog = Some(files::FileDialog::new(purpose, dir, name, self.song_dir()));
    }

    /// Opens the file explorer on the backups folder, to open one.
    fn pick_backup(&mut self) {
        let Some(dir) = backup::dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        self.file_dialog = Some(files::FileDialog::new(files::Purpose::OpenSong, dir, String::new(), self.song_dir()));
    }

    fn file_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &mut self.file_dialog else { return };
        let Some(event) = dialog.show(ctx) else { return };
        match event {
            files::Event::Preview(path) if self.browser.autoplay => self.preview_file(&path),
            files::Event::Preview(_) => {}
            files::Event::Cancelled => {
                self.stop_preview();
                self.file_dialog = None;
                self.after_save = None;
            }
            files::Event::Picked(purpose, path) => {
                self.stop_preview();
                self.file_dialog = None;
                let path_str = path.to_string_lossy().into_owned();
                match purpose {
                    files::Purpose::OpenSong => self.request(open_or_import(path_str)),
                    files::Purpose::SaveSong => {
                        self.path = path_str;
                        self.save();
                        if let Some(p) = self.after_save.take()
                            && !self.modified
                        {
                            self.perform(p);
                        }
                    }
                    files::Purpose::ExportProject => self.export_project(&path_str),
                    files::Purpose::ImportProject => self.request(Pending::Import(path_str)),
                    files::Purpose::Render => self.export(&path_str),
                    files::Purpose::RenderStems => self.export_stems(&path_str),
                    files::Purpose::LoadSample(id) => {
                        self.load_sample(id, &path);
                    }
                }
            }
        }
    }

    fn preview_file(&mut self, path: &std::path::Path) {
        if !crate::sample::is_audio(path) {
            return;
        }
        match Sample::load(path) {
            Ok(s) => self.preview(Arc::new(s), None),
            Err(e) => self.status = format!("Could not load {}: {e}", path.display()),
        }
    }

    /// Saves to the song's file, or asks for one if it hasn't been saved yet.
    fn save_or_ask(&mut self) {
        if self.untitled {
            self.pick_file(files::Purpose::SaveSong);
        } else {
            self.save();
        }
    }

    // ------------------------------------------------------------ panels

    /// The menu bar along the top of the window.
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let config = egui::containers::menu::MenuConfig::new().style(theme::menu_style);
        egui::MenuBar::new().config(config).ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if menu_item(ui, "New", "Ctrl+N") {
                    self.request(Pending::New);
                }
                if menu_item(ui, "Open…", "Ctrl+O") {
                    self.pick_file(files::Purpose::OpenSong);
                }
                ui.menu_button("Demo Songs", |ui| {
                    for (i, (name, _)) in DEMOS.iter().enumerate() {
                        if ui.button(*name).clicked() {
                            self.request(Pending::Demo(i));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                if menu_item(ui, "Save", "Ctrl+S") {
                    self.save_or_ask();
                }
                if menu_item(ui, "Save As…", "Ctrl+Shift+S") {
                    self.pick_file(files::Purpose::SaveSong);
                }
                if ui
                    .button("Open Backup…")
                    .on_hover_text(
                        "Songs with unsaved changes are backed up every few minutes; the newest ten of each are kept",
                    )
                    .clicked()
                {
                    self.pick_backup();
                }
                ui.separator();
                if ui
                    .button("Import Project…")
                    .on_hover_text("Unpacks a project's .noise file into a folder beside it, and opens it")
                    .clicked()
                {
                    self.pick_file(files::Purpose::ImportProject);
                }
                if ui
                    .button("Export Project…")
                    .on_hover_text("Writes the song and every sample it plays to a .noise file (a zip archive), to share or move")
                    .clicked()
                {
                    self.pick_file(files::Purpose::ExportProject);
                }
                ui.separator();
                if menu_item(ui, "Render to WAV…", "") {
                    self.render_dialog = Some(files::Purpose::Render);
                }
                if ui
                    .button("Render Stems…")
                    .on_hover_text("A WAV file for each instrument, with its effects, all as long as the song")
                    .clicked()
                {
                    self.render_dialog = Some(files::Purpose::RenderStems);
                }
                ui.separator();
                if menu_item(ui, "Quit", "Ctrl+Q") {
                    self.request(Pending::Quit);
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = egui::Button::new("Undo").shortcut_text("Ctrl+Z");
                if ui.add_enabled(!self.undo.is_empty(), undo).clicked() {
                    self.undo();
                }
                let redo = egui::Button::new("Redo").shortcut_text("Ctrl+Y");
                if ui.add_enabled(!self.redo.is_empty(), redo).clicked() {
                    self.redo();
                }
                if self.view == View::Pattern {
                    ui.separator();
                    pattern::edit_menu(self, ui);
                }
                ui.separator();
                let names = |q: usize| if q == 0 { "Off".to_string() } else { format!("{q} line{}", if q > 1 { "s" } else { "" }) };
                ui.menu_button(format!("Record Quantize: {}", names(self.record_quantize)), |ui| {
                    for q in [0, 1, 2, 3, 4, 6, 8, 12, 16] {
                        if ui.selectable_label(self.record_quantize == q, names(q)).clicked() {
                            self.record_quantize = q;
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Notes recorded while the song plays go to the nearest multiple of this many lines. Off, they go to the line playing, with how late in it in the delay column when it is shown");
                ui.checkbox(&mut self.record_note_offs, "Record Note-Offs")
                    .on_hover_text("Releasing a key while recording with the song playing writes a note-off");
                ui.separator();
                if ui.button("MIDI Input…").clicked() {
                    self.show_midi = true;
                    self.midi_ports = crate::midi::Midi::ports();
                }
            });
            ui.menu_button("View", |ui| {
                for (view, label, key) in View::ALL {
                    if ui.add(egui::Button::selectable(self.view == view, label).shortcut_text(key)).clicked() {
                        self.view = view;
                    }
                }
                ui.separator();
                for (view, label, _) in ScopeView::ALL {
                    if ui.add(egui::Button::selectable(self.scope_view == view, label)).clicked() {
                        self.scope_view = view;
                    }
                }
                ui.separator();
                // The parts of the window that can be hidden, with their keys.
                for (on, label, key) in [
                    (&mut self.show_upper, "Upper Frame", "Ctrl+1"),
                    (&mut self.show_sequencer, "Pattern Sequencer", "Ctrl+2"),
                    (&mut self.show_lower, "Lower Frame", "Ctrl+3"),
                    (&mut self.show_instruments, "Instrument List", "Ctrl+4"),
                    (&mut self.show_browser, "Disk Browser", "Ctrl+5"),
                ] {
                    if ui.add(egui::Button::selectable(*on, label).shortcut_text(key)).clicked() {
                        *on = !*on;
                    }
                }
                ui.checkbox(&mut self.show_matrix, "Extended Sequencer (names and matrix)");
                ui.menu_button("Transport", |ui| {
                    ui.checkbox(&mut self.show_song_settings, "Song Settings (BPM, LPB, TPL, Swing)");
                    ui.checkbox(&mut self.show_entry_settings, "Note Entry (Octave, Step, Volume)");
                    ui.checkbox(&mut self.show_cpu, "CPU Meter");
                });
                ui.separator();
                ui.checkbox(&mut self.show_comments, "Song Comments");
            });
            ui.menu_button("Help", |ui| {
                if menu_item(ui, "Manual", "F1") {
                    self.show_help = true;
                }
                for topic in ["Effect commands", "Keys", "Getting started"] {
                    if ui.button(topic).clicked() {
                        self.help.open(topic);
                        self.show_help = true;
                    }
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // The song's own title and artist go first, then its file, or
                // that it has none yet.
                let credit = match (self.project.title.trim(), self.project.artist.trim()) {
                    ("", "") => String::new(),
                    (title, "") => title.to_string(),
                    ("", artist) => artist.to_string(),
                    (title, artist) => format!("{title} by {artist}"),
                };
                let name = match (self.untitled, credit.is_empty()) {
                    (true, true) => "Untitled".into(),
                    (true, false) => "not saved".into(),
                    (false, _) => self.song_name(),
                };
                let mut text = if self.modified { format!("{name} *") } else { name };
                if !credit.is_empty() {
                    text = format!("{credit}  ·  {text}");
                }
                let label = ui.label(RichText::new(text).color(theme::TEXT_WEAK));
                label.on_hover_text(if self.untitled { "Not saved yet" } else { &self.path });
            });
        });
    }

    fn song_name(&self) -> String {
        std::path::Path::new(&self.path).file_name().unwrap_or_default().to_string_lossy().into_owned()
    }

    /// The right column: the instrument list, and the disk
    /// browser below it.
    fn right_column(&mut self, ui: &mut egui::Ui) {
        let h = ui.available_height();
        let w = ui.available_width();
        if self.show_instruments {
            let list_h = if self.show_browser { (h * 0.38).clamp(132.0, 360.0) } else { h };
            boxed(ui, "instruments", Vec2::new(w, list_h), |ui| instruments::panel(self, ui));
        }
        if self.show_browser {
            if self.show_instruments {
                ui.add_space(5.0);
            }
            boxed(ui, "browser", Vec2::new(w, ui.available_height()), |ui| {
                let now = self.now_playing();
                if now.is_some() {
                    ui.ctx().request_repaint();
                }
                if let Some(action) = self.browser.ui(ui, now) {
                    self.browser_action(action);
                }
            });
        }
    }

    /// The upper frame: the scopes.
    fn upper_frame(&mut self, ui: &mut egui::Ui) {
        let h = 132.0;
        ui.horizontal(|ui| {
            boxed(ui, "scopes", Vec2::new(ui.available_width(), h), |ui| {
                ui.horizontal(|ui| {
                    let caption = match self.scope_view {
                        ScopeView::Scope => "MASTER SCOPE",
                        ScopeView::Tracks => "TRACK SCOPES",
                        ScopeView::Spectrum => "MASTER SPECTRUM",
                        ScopeView::Wave => "PLAYING SAMPLES",
                    };
                    theme::caption(ui, caption);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        for (view, label, tip) in ScopeView::ALL.into_iter().rev() {
                            if theme::toggle(ui, self.scope_view == view, label).on_hover_text(tip).clicked() {
                                self.scope_view = view;
                            }
                        }
                    });
                });
                match self.scope_view {
                    ScopeView::Scope => self.scopes(ui),
                    ScopeView::Tracks => trackscopes::panel(self, ui),
                    ScopeView::Spectrum => {
                        let frames = self.shared.scope.lock().map(|s| s.clone()).unwrap_or_default();
                        let sr = self.audio.as_ref().map_or(44100, |a| a.sample_rate) as f32;
                        spectrum::panel(&mut self.spectrum, ui, &frames, sr);
                    }
                    ScopeView::Wave => waveview::panel(self, ui),
                }
            });
        });
    }

    /// The transport bar: playback, song and edit settings, the position and
    /// the CPU load, in framed groups.
    fn transport(&mut self, ui: &mut egui::Ui) {
        // One row when it fits, as in a wide window; else the song and
        // entry settings go below.
        let two_rows = self.transport_width > ui.available_width();
        let mut width = 0.0;
        ui.horizontal(|ui| {
            let x0 = ui.cursor().min.x;
            let playing = self.is_playing();
            let big = Vec2::new(30.0, 22.0);
            group(ui, |ui| {
                let play = icons::sized_button(ui, icons::Icon::Play, big, playing);
                if play.on_hover_text("Play (Space)").clicked() {
                    self.toggle_play();
                }
                if icons::sized_button(ui, icons::Icon::PlayFrom, big, false)
                    .on_hover_text("Play from the cursor's line (Shift+Space); Play starts the pattern from its top")
                    .clicked()
                {
                    self.play_from_cursor();
                }
                if icons::sized_button(ui, icons::Icon::Loop, big, self.loop_pattern)
                    .on_hover_text("Loop the current pattern")
                    .clicked()
                {
                    self.loop_pattern = !self.loop_pattern;
                }
                if icons::sized_button(ui, icons::Icon::Stop, big, false).on_hover_text("Stop").clicked() {
                    self.send(Cmd::Stop);
                }
                let rec = egui::Button::new("").min_size(big);
                let rec = if self.edit_mode { rec.fill(theme::RECORD) } else { rec };
                let resp = ui.add(rec).on_hover_text("Edit mode (Esc)");
                let dot = if self.edit_mode { Color32::WHITE } else { theme::RECORD };
                ui.painter().circle_filled(resp.rect.center(), 5.0, dot);
                if resp.clicked() {
                    self.edit_mode = !self.edit_mode;
                }
            });
            group(ui, |ui| {
                let h = Vec2::new(0.0, 22.0);
                if ui
                    .add(egui::Button::selectable(self.follow, "Follow").min_size(h))
                    .on_hover_text("Follow the play position")
                    .clicked()
                {
                    self.follow = !self.follow;
                }
                if ui
                    .add(egui::Button::selectable(self.metronome, "Metronome").min_size(h))
                    .on_hover_text("Click on every beat while playing")
                    .clicked()
                {
                    self.metronome = !self.metronome;
                    self.send(Cmd::Metronome(self.metronome));
                }
                let panic = ui.add(egui::Button::new("Panic").min_size(h));
                if panic.on_hover_text("Silence everything").clicked() {
                    self.send(Cmd::Panic);
                }
            });
            let settings_x = ui.cursor().min.x;
            if !two_rows {
                self.settings_groups(ui);
            }
            let settings_width = ui.cursor().min.x - settings_x;
            if settings_width > 0.0 {
                self.settings_width = settings_width;
            }
            group(ui, |ui| self.position(ui));
            let left = ui.cursor().min.x - x0;

            // On the right: the CPU meter and, before it, the master volume.
            let right = ui
                .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let x1 = ui.cursor().max.x;
                    if self.show_cpu {
                        group(ui, |ui| self.cpu_meter(ui));
                    }
                    group(ui, |ui| self.master_volume(ui));
                    x1 - ui.cursor().max.x
                })
                .inner;
            width = left + right + if two_rows { self.settings_width } else { 0.0 };
        });
        if two_rows && (self.show_song_settings || self.show_entry_settings) {
            ui.horizontal(|ui| {
                let x0 = ui.cursor().min.x;
                self.settings_groups(ui);
                self.settings_width = ui.cursor().min.x - x0;
            });
        }
        if !self.show_song_settings && !self.show_entry_settings {
            self.settings_width = 0.0;
        }
        self.transport_width = width;
    }

    /// The song settings (tempo, lines per beat, ticks, swing, song loop)
    /// and the entry settings (octave, step, volume) of the transport.
    fn settings_groups(&mut self, ui: &mut egui::Ui) {
        let playing = self.is_playing();
        if self.show_song_settings {
            group(ui, |ui| {
                transport_label(ui, "BPM");
                // While an Fxx has changed the tempo, the field shows the
                // tempo playing, in the play colour; editing it sets the
                // song's.
                let live = f32::from_bits(self.shared.bpm.load(Ordering::Relaxed));
                let changed = playing && (live - self.project.bpm).abs() > 0.01;
                let mut bpm = if changed { live } else { self.project.bpm };
                let drag = egui::DragValue::new(&mut bpm).range(20.0..=999.0).speed(0.5).max_decimals(1);
                let tip = if changed {
                    format!("Beats per minute: {live:.0} set by an Fxx command, {:.1} for the song", self.project.bpm)
                } else {
                    "Beats per minute".into()
                };
                let resp = ui
                    .scope(|ui| {
                        if changed {
                            ui.visuals_mut().override_text_color = Some(theme::SCOPE);
                        }
                        field(ui, 46.0, drag)
                    })
                    .inner;
                if resp.on_hover_text(tip).changed() {
                    self.project.bpm = bpm;
                    self.mark();
                }
                transport_label(ui, "LPB");
                let mut lpb = self.project.lpb;
                if field(ui, 30.0, egui::DragValue::new(&mut lpb).range(1..=32))
                    .on_hover_text("Lines per beat")
                    .changed()
                {
                    self.project.lpb = lpb;
                    self.mark();
                }
                transport_label(ui, "TPL");
                let mut tpl = self.project.tpl;
                let tip = "Ticks per line: the steps effects take within a line";
                if field(ui, 30.0, egui::DragValue::new(&mut tpl).range(1..=16)).on_hover_text(tip).changed() {
                    self.project.tpl = tpl;
                    self.mark();
                }
                transport_label(ui, "SWING");
                let mut swing = self.project.groove * 100.0;
                let tip = "Groove: every odd line starts up to half a line late; double-click for none";
                let swing_field =
                    egui::DragValue::new(&mut swing).range(0.0..=100.0).speed(0.5).suffix("%").max_decimals(0);
                let resp = field(ui, 40.0, swing_field).on_hover_text(tip);
                if resp.double_clicked() {
                    swing = 0.0;
                }
                if swing / 100.0 != self.project.groove {
                    self.project.groove = swing / 100.0;
                    self.mark();
                }
            });
        }
        if self.show_entry_settings {
            group(ui, |ui| {
                transport_label(ui, "OCT");
                field(ui, 30.0, egui::DragValue::new(&mut self.octave).range(0..=9)).on_hover_text("Octave (- / =)");
                transport_label(ui, "STEP");
                field(ui, 30.0, egui::DragValue::new(&mut self.step).range(0..=16))
                    .on_hover_text("Lines the cursor moves after entering a note");
                transport_label(ui, "VOL");
                let vol = egui::DragValue::new(&mut self.entry_vol)
                    .range(0..=0x80)
                    .speed(1.0)
                    .custom_formatter(|v, _| if v >= 128.0 { "--".into() } else { format!("{:02X}", v as u8) })
                    .custom_parser(|t| {
                        if t.trim() == "--" {
                            Some(128.0)
                        } else {
                            u8::from_str_radix(t.trim(), 16).ok().map(f64::from)
                        }
                    });
                field(ui, 30.0, vol)
                    .on_hover_text("Volume written with the notes you enter, in hex; -- writes none (full volume)");
            });
        }
    }

    /// The play position, or the cursor while stopped, and the time played.
    fn position(&self, ui: &mut egui::Ui) {
        let playing = self.is_playing();
        let (slot, line) = if playing { self.play_position() } else { (self.slot, self.cursor.line) };
        let pattern = self.project.order.get(slot).map_or(0, |s| s.pattern);
        let secs = f32::from_bits(self.shared.time.load(Ordering::Relaxed));
        let text =
            format!("SEQ {slot:02} PAT {pattern:02} LINE {line:03} {}:{:04.1}", (secs / 60.0) as u32, secs % 60.0);
        egui::Frame::new().fill(theme::INSET).corner_radius(2).inner_margin(egui::Margin::symmetric(6, 2)).show(
            ui,
            |ui| {
                let color = if playing { theme::SCOPE } else { theme::TEXT_WEAK };
                ui.label(RichText::new(text).monospace().color(color))
                    .on_hover_text("Song position, pattern, line and time played");
            },
        );
    }

    /// The master volume: the Output module's, as a bar to drag (Shift
    /// for fine steps), double-click for its default.
    fn master_volume(&mut self, ui: &mut egui::Ui) {
        let Some(out) = self.project.module(OUTPUT_ID) else { return };
        let spec = crate::project::ParamSpec { name: "Master", ..out.kind.params()[0] };
        // While an envelope moves it, the bar follows, as parameter bars do.
        let shown = self.automated(OUTPUT_ID, 0).unwrap_or(out.params[0]);
        let mut volume = shown;
        let resp = ui
            .allocate_ui(Vec2::new(104.0, ui.spacing().interact_size.y), |ui| {
                widgets::param_bar(ui, &spec, &mut volume, |_| {})
            })
            .inner;
        if resp.on_hover_text("The volume of everything, after the mix").changed() && volume != shown {
            self.project.module_mut(OUTPUT_ID).unwrap().params[0] = volume;
            self.mark();
        }
    }

    /// How much of the real time rendering audio takes.
    fn cpu_meter(&self, ui: &mut egui::Ui) {
        let cpu = f32::from_bits(self.shared.cpu.load(Ordering::Relaxed));
        // The load as a bar with its figure over it.
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(62.0, 16.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 2.0, theme::INSET);
        let color = match cpu {
            c if c > 0.8 => theme::RECORD,
            c if c > 0.5 => theme::SELECTED,
            _ => theme::SCOPE,
        };
        let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width() * cpu.clamp(0.0, 1.0), rect.height()));
        painter.rect_filled(bar, 2.0, color);
        let rate = self.audio.as_ref().map_or("no audio device".into(), |a| format!("{} Hz", a.sample_rate));
        let text = format!("CPU {:.0}%", cpu * 100.0);
        let font = egui::FontId::monospace(10.0);
        painter.text(
            rect.center() + Vec2::new(1.0, 1.0),
            egui::Align2::CENTER_CENTER,
            &text,
            font.clone(),
            Color32::BLACK,
        );
        painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, font, theme::TEXT);
        resp.on_hover_text(format!("Time spent rendering audio ({rate})"));
    }

    /// Master scope (left and right channel) and peak meters.
    fn scopes(&self, ui: &mut egui::Ui) {
        let size = ui.available_size();
        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2.0, theme::INSET);
        let meters_w = 34.0;
        let scope = Rect::from_min_max(rect.min, Pos2::new(rect.right() - meters_w - 4.0, rect.bottom()));
        let lane_h = scope.height() / 2.0;
        if let Ok(data) = self.shared.scope.lock() {
            let data = &data[data.len().saturating_sub(1024)..];
            for ch in [0, 1] {
                let mid = scope.top() + lane_h * (ch as f32 + 0.5);
                painter.line_segment(
                    [Pos2::new(scope.left(), mid), Pos2::new(scope.right(), mid)],
                    Stroke::new(1.0, Color32::from_gray(36)),
                );
                let w = scope.width().max(1.0) as usize;
                let pts: Vec<Pos2> = (0..w)
                    .map(|x| {
                        let v = data[x * data.len() / w][ch];
                        Pos2::new(scope.left() + x as f32, mid - v.clamp(-1.0, 1.0) * lane_h * 0.45)
                    })
                    .collect();
                painter.line(pts, Stroke::new(1.0, theme::SCOPE));
            }
        }
        for ch in 0..2 {
            let peak = f32::from_bits(self.shared.peak[ch].load(Ordering::Relaxed));
            let db = 20.0 * peak.max(1e-6).log10();
            // Show -48 dB .. +6 dB.
            let frac = ((db + 48.0) / 54.0).clamp(0.0, 1.0);
            let x = rect.right() - meters_w + ch as f32 * 17.0;
            let full = Rect::from_min_max(Pos2::new(x, rect.top() + 2.0), Pos2::new(x + 14.0, rect.bottom() - 2.0));
            painter.rect_filled(full, 1.0, Color32::from_gray(30));
            let top = full.bottom() - full.height() * frac;
            let bar = Rect::from_min_max(Pos2::new(full.left(), top), full.max);
            let color = if peak >= 1.0 {
                theme::RECORD
            } else if db > -6.0 {
                theme::SELECTED
            } else {
                theme::SCOPE
            };
            painter.rect_filled(bar, 1.0, color);
            let zero = full.bottom() - full.height() * (48.0 / 54.0);
            painter.line_segment(
                [Pos2::new(full.left(), zero), Pos2::new(full.right(), zero)],
                Stroke::new(1.0, Color32::from_gray(120)),
            );
        }
    }

    /// The lower frame's tabs, along the bottom of the window: a tab opens
    /// the frame on its page, the open one closes it.
    fn lower_tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, label, tip) in [
                (Lower::Modules, "Modules", "Every module in a list, with the selected one's parameters and wiring"),
                (Lower::Automation, "Automation", "Envelopes that move parameters over the pattern"),
                (Lower::TrackFx, "Track FX", "Each track's effects, and the master chain the whole mix goes through"),
            ] {
                let open = self.show_lower && self.lower == tab;
                let button = egui::Button::selectable(open, label).min_size(Vec2::new(90.0, 20.0));
                let tip = format!("{tip} (Ctrl+3 shows or hides the lower frame)");
                if ui.add(button).on_hover_text(tip).clicked() {
                    self.show_lower = !open;
                    self.lower = tab;
                }
            }
        });
    }

    /// Opens the lower frame on track `t`'s effects.
    pub fn show_track_fx(&mut self, t: usize) {
        self.show_lower = true;
        self.lower = Lower::TrackFx;
        self.track_fx = TrackFx::Track(t);
        self.cursor.track = t;
        self.fx_cursor = t;
        self.clamp_cursor();
    }

    /// The Track FX tab: a chip for each track in its color, and the
    /// master, over the chosen one's effects. It follows the cursor's track.
    fn track_fx_page(&mut self, ui: &mut egui::Ui) {
        if self.cursor.track != self.fx_cursor {
            self.fx_cursor = self.cursor.track;
            self.track_fx = TrackFx::Track(self.cursor.track);
        }
        ui.horizontal_wrapped(|ui| {
            for t in 0..self.pattern().num_tracks() {
                let n = self.project.tracks.get(t).map_or(0, |t| t.effects.len());
                let name = self.project.track_name(t);
                let text = if n > 0 { format!("{name}  {n}") } else { name };
                let on = self.track_fx == TrackFx::Track(t);
                let tip = format!("{} effect{} on this track; click to edit them", n, if n == 1 { "" } else { "s" });
                if chip(ui, pattern::track_color(&self.project, t), &text, on).on_hover_text(tip).clicked() {
                    self.track_fx = TrackFx::Track(t);
                    self.cursor.track = t;
                    self.clamp_cursor();
                    self.fx_cursor = t;
                }
            }
            ui.separator();
            let n = self.project.master.len();
            let text = if n > 0 { format!("Master  {n}") } else { "Master".into() };
            let tip = "The master chain: effects the whole mix goes through before the output";
            if chip(ui, theme::SELECTED, &text, self.track_fx == TrackFx::Master).on_hover_text(tip).clicked() {
                self.track_fx = TrackFx::Master;
            }
        });
        ui.add_space(3.0);
        let owner = match self.track_fx {
            TrackFx::Track(t) => crate::project::Owner::Track(t),
            TrackFx::Master => crate::project::Owner::Module(OUTPUT_ID),
        };
        chain::page(self, ui, owner);
    }

    /// The lower frame: the module graph, with the selected module's
    /// parameters beside it, or the automation editor.
    fn lower_frame(&mut self, ui: &mut egui::Ui) {
        match self.lower {
            Lower::Automation => return automation::panel(self, ui),
            Lower::TrackFx => return self.track_fx_page(ui),
            Lower::Modules => {}
        }
        let h = ui.available_height();
        ui.horizontal(|ui| {
            let params_w = 300.0;
            let graph_w = ui.available_width() - params_w - 6.0;
            boxed(ui, "modules", Vec2::new(graph_w, h), |ui| modules::list(self, ui));
            boxed(ui, "params", Vec2::new(ui.available_width(), h), |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| modules::params_panel(self, ui));
            });
        });
    }

    /// Tabs that switch the middle of the window.
    /// With arrows at its ends that show and hide the frames around the
    /// middle: the sequencer on the left, the
    /// scopes above and the instrument list and disk browser on the right.
    fn view_tabs(&mut self, ui: &mut egui::Ui) {
        use icons::Icon;
        ui.horizontal(|ui| {
            let (icon, tip) = if self.show_sequencer { (Icon::Left, "Hide") } else { (Icon::Right, "Show") };
            if icons::button(ui, icon).on_hover_text(format!("{tip} the pattern sequencer (Ctrl+2)")).clicked() {
                self.show_sequencer = !self.show_sequencer;
            }
            for (view, label, key) in View::ALL {
                let tab = egui::Button::selectable(self.view == view, label).min_size(Vec2::new(110.0, 22.0));
                if ui.add(tab).on_hover_text(key).clicked() {
                    self.view = view;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let right = self.show_instruments || self.show_browser;
                let (icon, tip) = if right { (Icon::Right, "Hide") } else { (Icon::Left, "Show") };
                let tip = format!("{tip} the instrument list and disk browser (Ctrl+4, Ctrl+5)");
                if icons::button(ui, icon).on_hover_text(tip).clicked() {
                    (self.show_instruments, self.show_browser) = (!right, !right);
                }
                let (icon, tip) = if self.show_upper { (Icon::Up, "Hide") } else { (Icon::Down, "Show") };
                let tip = format!("{tip} the scopes: track scopes, spectrum and wave view (Ctrl+1)");
                if icons::button(ui, icon).on_hover_text(tip).clicked() {
                    self.show_upper = !self.show_upper;
                }
            });
        });
        ui.add_space(3.0);
    }

    fn browser_action(&mut self, action: browser::Action) {
        match action {
            browser::Action::Preview(path) => self.preview_file(&path),
            browser::Action::StopPreview => self.stop_preview(),
            browser::Action::PreviewSettings => self.send_preview_settings(),
            browser::Action::LoadSample(path) => {
                self.stop_preview();
                self.load_samples(vec![path]);
            }
            browser::Action::OpenSong(path) => self.request(open_or_import(path.to_string_lossy().into_owned())),
        }
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
            (Key::Num1, &mut self.show_upper),
            (Key::Num2, &mut self.show_sequencer),
            (Key::Num3, &mut self.show_lower),
            (Key::Num4, &mut self.show_instruments),
            (Key::Num5, &mut self.show_browser),
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
                    ui.label(&self.status);
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
        if self.show_instruments || self.show_browser {
            egui::Panel::right("right")
                .frame(frame)
                .resizable(true)
                .default_size(270.0)
                .min_size(200.0)
                .show(ui, |ui| self.right_column(ui));
        }
        if self.show_upper {
            egui::Panel::top("upper").frame(frame).show(ui, |ui| self.upper_frame(ui));
        }
        let matrix_w = sequencer::matrix_width(self);
        if self.show_sequencer {
            // Narrow with just the pattern numbers; the extended view has
            // its own width, for the names and the matrix.
            let panel = if self.show_matrix {
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

/// A chip in `color` with `text`: lit when `on`, dim otherwise.
fn chip(ui: &mut egui::Ui, color: Color32, text: &str, on: bool) -> egui::Response {
    let font = egui::FontId::proportional(12.0);
    let text_color = if on { theme::SELECTED_TEXT } else { theme::TEXT };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, text_color);
    let size = galley.size() + Vec2::new(16.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let fill = if on { color } else { color.gamma_multiply(if resp.hovered() { 0.5 } else { 0.3 }) };
    ui.painter().rect_filled(rect, 9.0, fill);
    if on {
        ui.painter().rect_stroke(rect, 9.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Inside);
    }
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, text_color);
    resp
}

fn setup_style(ctx: &egui::Context) {
    theme::setup(ctx);
}

fn transport_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).small().color(theme::TEXT_WEAK));
}

/// Adds a transport field at a fixed width, so a value growing a digit
/// doesn't push what comes after it.
fn field(ui: &mut egui::Ui, width: f32, widget: impl egui::Widget) -> egui::Response {
    ui.add_sized(Vec2::new(width, ui.spacing().interact_size.y), widget)
}

/// A framed group of controls in the transport bar.
fn group<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::FRAME_BG)
        .stroke(Stroke::new(1.0, theme::FRAME_LINE))
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(4, 3))
        .show(ui, |ui| ui.horizontal(add).inner)
        .inner
}

/// A menu entry with its shortcut on the right; true when clicked.
fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> bool {
    ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked()
}

/// Allocates `size` and draws a framed box around `add`.
/// `id` keeps the widgets of different boxes apart.
fn boxed<R>(ui: &mut egui::Ui, id: &str, size: Vec2, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect(rect, 3.0, theme::FRAME_BG, Stroke::new(1.0, theme::FRAME_LINE), egui::StrokeKind::Inside);
    let inner = rect.shrink(5.0);
    let layout = egui::Layout::top_down(egui::Align::Min);
    let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(inner).layout(layout));
    child.shrink_clip_rect(inner);
    add(&mut child)
}
