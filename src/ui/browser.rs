//! The disk browser: a category to browse (songs,
//! instruments or samples), each remembering its own folder; the folders
//! of the one shown, apart from its files; and the path with buttons to
//! go up, home or to the music folder.

use super::icons::{self, Icon};
use super::theme;
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};

/// What the browser shows, picked with its category buttons.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Filter {
    Songs,
    /// Soundfonts, which load into a Sampler as a whole instrument.
    Instruments,
    Samples,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::Songs, Filter::Instruments, Filter::Samples];

    fn index(self) -> usize {
        Filter::ALL.iter().position(|&f| f == self).unwrap_or(0)
    }

    fn label(self) -> &'static str {
        match self {
            Filter::Songs => "Songs",
            Filter::Instruments => "Instruments",
            Filter::Samples => "Samples",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Filter::Songs => "Songs: double-click to open one",
            Filter::Instruments => "SF2, SF3 and SFZ soundfonts: double-click to load one into the selected Sampler",
            Filter::Samples => "WAV, FLAC and Ogg samples: click to hear one, double-click to load it",
        }
    }

    fn matches(self, path: &Path) -> bool {
        match self {
            Filter::Songs => {
                path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json")) || crate::project_dir::is_archive(path)
            }
            Filter::Instruments => crate::soundfont::is_soundfont(path),
            Filter::Samples => crate::sample::is_audio(path),
        }
    }
}

struct Entry {
    path: PathBuf,
    name: String,
    /// The file's type and size, shown on its right.
    info: String,
}

pub enum Action {
    /// A sample was clicked with Autoplay on, or Play was pressed: play it.
    Preview(PathBuf),
    /// Stop was pressed.
    StopPreview,
    /// The preview's volume or looping changed.
    PreviewSettings,
    /// A sample or soundfont was double-clicked: load it into the selected
    /// instrument.
    LoadSample(PathBuf),
    /// A song was double-clicked.
    OpenSong(PathBuf),
}

pub struct Browser {
    /// The folder of each category, in `Filter::ALL` order.
    pub dirs: [PathBuf; 3],
    pub filter: Filter,
    folders: Vec<Entry>,
    files: Vec<Entry>,
    /// What the lists were read for; reread when this changes.
    listed: Option<(PathBuf, Filter)>,
    selected: Option<PathBuf>,
    /// When the last folder was entered, in the UI's time.
    entered_at: f64,
    /// Play samples as soon as they are clicked, here and in the Sampler's
    /// sample list.
    pub autoplay: bool,
    /// Previews start again from the top until stopped.
    pub looping: bool,
    pub volume: f32,
}

/// What the preview is playing, for the playback strip.
pub struct NowPlaying {
    pub name: String,
    /// Where it is and how long it is, in seconds.
    pub at: f32,
    pub length: f32,
}

/// How long after entering a folder its folders don't take clicks, so the
/// second click of a double-click doesn't go on into another one.
const SETTLE: f64 = 0.5;

/// A time in seconds as m:ss.s.
fn clock(secs: f32) -> String {
    format!("{}:{:04.1}", (secs / 60.0) as u32, secs % 60.0)
}

/// A size in bytes for people.
fn size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

impl Browser {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dirs: [dir.clone(), dir.clone(), dir],
            filter: Filter::Samples,
            folders: Vec::new(),
            files: Vec::new(),
            listed: None,
            selected: None,
            entered_at: f64::NEG_INFINITY,
            autoplay: true,
            looping: false,
            volume: 0.8,
        }
    }

    /// The folder of the category shown.
    pub fn dir(&self) -> &Path {
        &self.dirs[self.filter.index()]
    }

    fn set_dir(&mut self, dir: PathBuf) {
        self.dirs[self.filter.index()] = dir;
    }

    fn refresh(&mut self) {
        self.folders.clear();
        self.files.clear();
        let dir = self.dir().to_path_buf();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let path = e.path();
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                if path.is_dir() && self.filter == Filter::Songs && crate::project_dir::is_project(&path) {
                    // A project folder is a song.
                    self.files.push(Entry { path, name, info: "PROJECT".into() });
                } else if path.is_dir() {
                    self.folders.push(Entry { path, name, info: String::new() });
                } else if self.filter.matches(&path) {
                    let ext = path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
                    let bytes = e.metadata().map_or(0, |m| m.len());
                    self.files.push(Entry { path, name, info: format!("{ext}  {}", size(bytes)) });
                }
            }
        }
        for list in [&mut self.folders, &mut self.files] {
            list.sort_by_key(|e| e.name.to_lowercase());
        }
        self.listed = Some((dir, self.filter));
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, now: Option<NowPlaying>) -> Option<Action> {
        if self.listed.as_ref().is_none_or(|(d, f)| d != self.dir() || *f != self.filter) {
            self.refresh();
        }
        let mut action = None;
        ui.horizontal(|ui| {
            theme::caption(ui, "DISK BROWSER");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(ui, Icon::Refresh).on_hover_text("Read the folder again").clicked() {
                    self.listed = None;
                }
            });
        });
        // The categories, each with its own folder.
        ui.columns(Filter::ALL.len(), |cols| {
            for (ui, f) in cols.iter_mut().zip(Filter::ALL) {
                let label = RichText::new(f.label()).small();
                let button =
                    egui::Button::selectable(self.filter == f, label).min_size(egui::vec2(ui.available_width(), 0.0));
                if ui.add(button).on_hover_text(f.tip()).clicked() {
                    self.filter = f;
                }
            }
        });
        // Where it is, and the way up, home and to the music folder.
        let mut go = None;
        ui.horizontal(|ui| {
            if icons::button(ui, Icon::Up).on_hover_text("The folder above").clicked() {
                go = self.dir().parent().map(Path::to_path_buf);
            }
            if let Some(home) = crate::paths::home_dir()
                && icons::button(ui, Icon::Home).on_hover_text("Home").clicked()
            {
                go = Some(home);
            }
            if let Some(music) = crate::paths::music_dir()
                && icons::button(ui, Icon::Music).on_hover_text("Music").clicked()
            {
                go = Some(music);
            }
            let shown = self.dir().to_string_lossy().into_owned();
            ui.add(egui::Label::new(RichText::new(&shown).color(theme::TEXT_WEAK).small()).truncate())
                .on_hover_text(shown);
        });

        // The folders, apart from the files: a click goes into one.
        let folders_h = (ui.available_height() * 0.35).clamp(48.0, 220.0);
        egui::Frame::new().fill(theme::INSET).show(ui, |ui| {
            ui.set_height(folders_h);
            egui::ScrollArea::vertical().id_salt("browser_folders").auto_shrink(false).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                    for e in &self.folders {
                        // A folder icon before the name, the whole row a button.
                        let (row, resp) =
                            ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0), egui::Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(row, 2.0, theme::BUTTON_HOVER);
                        }
                        let icon = egui::Rect::from_min_size(row.min + egui::vec2(4.0, 3.0), egui::Vec2::splat(12.0));
                        icons::paint(ui.painter(), icon, Icon::Folder, theme::SELECTED);
                        let text_at = egui::pos2(icon.right() + 6.0, row.center().y);
                        ui.painter().with_clip_rect(row).text(
                            text_at,
                            egui::Align2::LEFT_CENTER,
                            &e.name,
                            egui::FontId::proportional(14.0),
                            theme::SELECTED,
                        );
                        if resp.clicked() && ui.input(|i| i.time) - self.entered_at > SETTLE {
                            go = Some(e.path.clone());
                        }
                    }
                    if self.folders.is_empty() {
                        ui.label(RichText::new("No folders here").color(theme::TEXT_WEAK).small());
                    }
                });
            });
        });
        ui.add_space(3.0);
        let strip = self.filter == Filter::Samples;
        let files_h = ui.available_height() - if strip { 52.0 } else { 0.0 };
        egui::Frame::new().fill(theme::INSET).show(ui, |ui| {
            ui.set_height(files_h.max(40.0));
            let list = egui::ScrollArea::vertical().id_salt("browser_files").max_height(files_h.max(40.0));
            list.auto_shrink(false).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                    for e in &self.files {
                        let selected = self.selected.as_ref() == Some(&e.path);
                        let info = RichText::new(&e.info).small().color(theme::TEXT_WEAK);
                        let resp = ui.add(egui::Button::selectable(selected, e.name.as_str()).right_text(info));
                        if resp.clicked() {
                            self.selected = Some(e.path.clone());
                            if self.filter == Filter::Samples && self.autoplay {
                                action = Some(Action::Preview(e.path.clone()));
                            }
                        }
                        if resp.double_clicked() {
                            action = Some(match self.filter {
                                Filter::Songs => Action::OpenSong(e.path.clone()),
                                Filter::Instruments | Filter::Samples => Action::LoadSample(e.path.clone()),
                            });
                        }
                    }
                    if self.files.is_empty() {
                        let what = self.filter.label().to_lowercase();
                        ui.label(RichText::new(format!("No {what} in this folder")).color(theme::TEXT_WEAK).small());
                    }
                });
            });
        });
        if strip && let Some(a) = self.playback(ui, now) {
            action = Some(a);
        }
        if let Some(d) = go {
            self.set_dir(d);
            self.entered_at = ui.input(|i| i.time);
        }
        action
    }

    /// The preview's playback strip, under the disk browser: what
    /// plays and where it is, play and stop, Autoplay, Loop and the volume.
    fn playback(&mut self, ui: &mut egui::Ui, now: Option<NowPlaying>) -> Option<Action> {
        let mut action = None;
        ui.add_space(3.0);
        let (bar, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 14.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(bar, 2.0, theme::INSET);
        let text = match &now {
            Some(n) => {
                let done = if n.length > 0.0 { (n.at / n.length).clamp(0.0, 1.0) } else { 0.0 };
                let filled = egui::Rect::from_min_size(bar.min, egui::vec2(bar.width() * done, bar.height()));
                painter.rect_filled(filled, 2.0, theme::SELECTED.gamma_multiply(0.45));
                format!("{}  {} / {}", n.name, clock(n.at), clock(n.length))
            }
            None => "Stopped".into(),
        };
        let font = egui::FontId::proportional(11.0);
        let at = egui::pos2(bar.left() + 4.0, bar.center().y);
        painter.with_clip_rect(bar).text(at, egui::Align2::LEFT_CENTER, text, font, theme::TEXT);
        ui.horizontal(|ui| {
            let selected = self.selected.clone().filter(|p| self.filter.matches(p));
            let play = ui.add_enabled_ui(selected.is_some(), |ui| icons::button(ui, Icon::Play)).inner;
            if play.on_hover_text("Play the selected sample").clicked()
                && let Some(p) = selected
            {
                action = Some(Action::Preview(p));
            }
            let stop = ui.add_enabled_ui(now.is_some(), |ui| icons::button(ui, Icon::Stop)).inner;
            if stop.on_hover_text("Stop the preview").clicked() {
                action = Some(Action::StopPreview);
            }
            let tip = "Play samples as soon as they are clicked, here and in the Sampler's sample list";
            if theme::toggle(ui, self.autoplay, "Auto").on_hover_text(tip).clicked() {
                self.autoplay = !self.autoplay;
            }
            let size = egui::Vec2::splat(ui.spacing().interact_size.y);
            let tip = "Play previews over and over until stopped";
            if icons::sized_button(ui, Icon::Loop, size, self.looping).on_hover_text(tip).clicked() {
                self.looping = !self.looping;
                action = Some(Action::PreviewSettings);
            }
            let slider = egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false);
            if ui.add(slider).on_hover_text("Preview volume").changed() {
                action = Some(Action::PreviewSettings);
            }
        });
        action
    }
}
