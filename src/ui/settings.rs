//! Preferences kept between sessions, in `noise/settings.json` in the
//! user's config folder: the MIDI input, which frames are shown and where
//! the disk browser was.

use super::{App, ScopeView, browser::Filter};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub midi_port: Option<String>,
    pub midi_velocity: bool,
    pub show_upper: bool,
    pub show_matrix: bool,
    pub show_browser: bool,
    pub show_sequencer: bool,
    pub show_instruments: bool,
    pub show_song_settings: bool,
    pub show_entry_settings: bool,
    pub show_cpu: bool,
    /// The lower frame's height.
    pub lower_height: f32,
    pub scope_view: ScopeView,
    /// The disk browser's folder for the category shown, and for each
    /// category in `Filter::ALL` order.
    pub browser_dir: Option<PathBuf>,
    pub browser_dirs: Vec<PathBuf>,
    pub browser_filter: Filter,
    pub follow: bool,
    /// The disk browser's preview: played on a click, looped, its volume.
    pub preview_autoplay: bool,
    pub preview_loop: bool,
    pub preview_volume: f32,
    /// Renders: the sample rate (0 for the sound device's) and format.
    pub render_rate: u32,
    pub render_depth: crate::audio::BitDepth,
    /// Live recording: the quantize step in lines (0 off) and note-offs.
    pub record_quantize: usize,
    pub record_note_offs: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            midi_port: None,
            midi_velocity: true,
            show_upper: true,
            show_matrix: false,
            show_browser: true,
            show_sequencer: true,
            show_instruments: true,
            show_song_settings: true,
            show_entry_settings: true,
            show_cpu: true,
            lower_height: 270.0,
            scope_view: ScopeView::Scope,
            browser_dir: None,
            browser_dirs: Vec::new(),
            browser_filter: Filter::Samples,
            follow: true,
            preview_autoplay: true,
            preview_loop: false,
            preview_volume: 0.8,
            render_rate: 0,
            render_depth: crate::audio::BitDepth::default(),
            record_quantize: 0,
            record_note_offs: true,
        }
    }
}

impl Settings {
    /// Where settings live: `settings.json` in the config folder.
    pub fn path() -> Option<PathBuf> {
        crate::paths::config_dir().map(|d| d.join("settings.json"))
    }

    /// The settings in `path`, or the defaults if there are none or they
    /// can't be read.
    pub fn load_from(path: &Path) -> Settings {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap_or_default())
    }
}

impl App {
    /// The preferences as they stand.
    pub fn settings(&self) -> Settings {
        Settings {
            midi_port: self.midi.port.clone(),
            midi_velocity: self.midi_velocity,
            show_upper: self.show_upper,
            show_matrix: self.show_matrix,
            show_browser: self.show_browser,
            show_sequencer: self.show_sequencer,
            show_instruments: self.show_instruments,
            show_song_settings: self.show_song_settings,
            show_entry_settings: self.show_entry_settings,
            show_cpu: self.show_cpu,
            lower_height: self.lower_height,
            scope_view: self.scope_view,
            browser_dir: Some(self.browser.dir().to_path_buf()),
            browser_dirs: self.browser.dirs.to_vec(),
            browser_filter: self.browser.filter,
            follow: self.follow,
            preview_autoplay: self.browser.autoplay,
            preview_loop: self.browser.looping,
            preview_volume: self.browser.volume,
            render_rate: self.render_rate,
            render_depth: self.render_depth,
            record_quantize: self.record_quantize,
            record_note_offs: self.record_note_offs,
        }
    }

    /// Takes on saved preferences. The disk browser's folder is used only
    /// when no song was opened, which brings its own.
    pub fn apply_settings(&mut self, s: &Settings, song_given: bool) {
        self.midi_velocity = s.midi_velocity;
        self.show_upper = s.show_upper;
        // The lower frame starts closed, to its tabs; a tab opens it.
        self.show_matrix = s.show_matrix;
        self.show_browser = s.show_browser;
        self.show_sequencer = s.show_sequencer;
        self.show_instruments = s.show_instruments;
        self.show_song_settings = s.show_song_settings;
        self.show_entry_settings = s.show_entry_settings;
        self.show_cpu = s.show_cpu;
        self.lower_height = s.lower_height.clamp(120.0, 2000.0);
        self.scope_view = s.scope_view;
        self.browser.filter = s.browser_filter;
        self.follow = s.follow;
        self.browser.autoplay = s.preview_autoplay;
        self.browser.looping = s.preview_loop;
        self.browser.volume = s.preview_volume.clamp(0.0, 1.0);
        self.render_rate = s.render_rate.min(192_000);
        self.render_depth = s.render_depth;
        self.record_quantize = s.record_quantize.min(64);
        self.record_note_offs = s.record_note_offs;
        self.send_preview_settings();
        if !song_given {
            // Each category's folder, or for older settings the one folder
            // for all of them.
            // A folder that is just where noise was started from was the old
            // default, not a choice; the new default is better.
            let started_in = std::env::current_dir().ok();
            for (k, dir) in self.browser.dirs.iter_mut().enumerate() {
                let saved = s.browser_dirs.get(k).or(s.browser_dir.as_ref());
                if let Some(d) = saved.filter(|d| d.is_dir() && Some(*d) != started_in.as_ref()) {
                    *dir = d.clone();
                }
            }
        }
        if let Some(port) = &s.midi_port
            && let Err(e) = self.midi.connect(port)
        {
            self.set_status(format!("MIDI input {port} isn't there: {e}"));
        }
    }

    /// Writes the preferences when they changed.
    pub fn save_settings(&mut self) {
        let now = self.settings();
        if now == self.saved_settings {
            return;
        }
        if let Some(path) = Settings::path()
            && let Err(e) = now.save_to(&path)
        {
            self.set_status(format!("Could not save preferences to {}: {e}", path.display()));
        }
        self.saved_settings = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_default_when_missing() {
        let dir = std::env::temp_dir().join(format!("noise-settings-{}", std::process::id()));
        let path = dir.join("noise").join("settings.json");
        assert_eq!(Settings::load_from(&path), Settings::default(), "nothing saved yet");
        let s = Settings {
            midi_port: Some("Keys".into()),
            show_browser: false,
            scope_view: ScopeView::Tracks,
            ..Settings::default()
        };
        s.save_to(&path).unwrap();
        assert_eq!(Settings::load_from(&path), s);
        // Settings from an older version miss fields, which get defaults.
        // and fields they had that are gone now are passed over.
        std::fs::write(&path, r#"{"show_upper": false, "show_lower": true}"#).unwrap();
        let old = Settings::load_from(&path);
        assert!(!old.show_upper && old.show_sequencer && old.midi_velocity);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
