//! The built-in file explorer, used wherever a file has to be picked.

use super::theme;
use eframe::egui::{self, Key, RichText};
use std::path::{Path, PathBuf};

/// What the picked file is for.
#[derive(Clone, Copy, PartialEq)]
pub enum Purpose {
    /// A project folder, or a song saved on its own before projects were
    /// folders.
    OpenSong,
    /// A project folder to save into: a new one, or a project's.
    SaveSong,
    /// A `.noise` file, a zip archive, to export the project to.
    ExportProject,
    /// A project's `.noise` file to import.
    ImportProject,
    Render,
    /// Render each instrument to a file named after the one picked.
    RenderStems,
    /// Load a sample into this module.
    LoadSample(u8),
}

impl Purpose {
    fn title(self) -> &'static str {
        match self {
            Purpose::OpenSong => "Open Song",
            Purpose::SaveSong => "Save Song As",
            Purpose::ExportProject => "Export Project",
            Purpose::ImportProject => "Import Project",
            Purpose::Render => "Render Song to WAV",
            Purpose::RenderStems => "Render Stems to WAV",
            Purpose::LoadSample(_) => "Load Sample",
        }
    }

    fn confirm(self) -> &'static str {
        match self {
            Purpose::OpenSong | Purpose::LoadSample(_) => "Open",
            Purpose::SaveSong => "Save",
            Purpose::ExportProject => "Export",
            Purpose::ImportProject => "Import",
            Purpose::Render | Purpose::RenderStems => "Render",
        }
    }

    /// Whether the file may not exist yet, so a name can be typed.
    fn saving(self) -> bool {
        matches!(self, Purpose::SaveSong | Purpose::ExportProject | Purpose::Render | Purpose::RenderStems)
    }

    /// Whether a project folder is picked, rather than gone into.
    fn picks_projects(self) -> bool {
        matches!(self, Purpose::OpenSong | Purpose::SaveSong)
    }

    /// The extension a name typed when saving gets: none for a project
    /// folder.
    fn extension(self) -> &'static str {
        match self {
            Purpose::OpenSong => ".json",
            Purpose::SaveSong => "",
            Purpose::ExportProject | Purpose::ImportProject => ".noise",
            Purpose::Render | Purpose::RenderStems | Purpose::LoadSample(_) => ".wav",
        }
    }

    /// What the explorer lists, unless all files are asked for.
    fn kinds(self) -> &'static str {
        match self {
            Purpose::OpenSong | Purpose::SaveSong => "songs",
            Purpose::ExportProject | Purpose::ImportProject => ".noise projects",
            Purpose::Render | Purpose::RenderStems => "WAV files",
            Purpose::LoadSample(_) => "samples and soundfonts",
        }
    }

    fn offers(self, name: &str) -> bool {
        match self {
            Purpose::LoadSample(_) => {
                let path = Path::new(name);
                crate::sample::is_audio(path) || crate::soundfont::is_soundfont(path)
            }
            // Songs are saved as folders, which are listed anyway.
            Purpose::SaveSong => false,
            // Songs on their own from earlier versions, and exported projects.
            Purpose::OpenSong => {
                let name = name.to_lowercase();
                name.ends_with(".json") || name.ends_with(".noise")
            }
            _ => name.to_lowercase().ends_with(self.extension()),
        }
    }
}

pub enum Event {
    /// A sample was clicked while loading samples: play it.
    Preview(PathBuf),
    Picked(Purpose, PathBuf),
    Cancelled,
}

struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    /// A project folder, picked like a file when opening or saving songs.
    project: bool,
    size: u64,
}

pub struct FileDialog {
    purpose: Purpose,
    dir: PathBuf,
    /// Folder the path bar is being edited to.
    dir_input: String,
    name: String,
    show_all: bool,
    show_hidden: bool,
    entries: Vec<Entry>,
    error: Option<String>,
    places: Vec<(String, PathBuf)>,
}

impl FileDialog {
    /// Opens in `dir`, with `name` filled in as the file name.
    pub fn new(purpose: Purpose, dir: PathBuf, name: String, song_dir: Option<PathBuf>) -> Self {
        let mut places = Vec::new();
        if let Some(home) = crate::paths::home_dir() {
            places.push(("Home".to_string(), home));
        }
        if let Some(music) = crate::paths::music_dir() {
            places.push(("Music".to_string(), music));
        }
        if let Some(d) = song_dir {
            places.push(("Song folder".to_string(), d));
        }
        if let Ok(d) = std::env::current_dir() {
            places.push(("Working folder".to_string(), d));
        }
        places.push(("/".to_string(), PathBuf::from("/")));
        let mut seen = Vec::new();
        places.retain(|(_, p)| {
            let new = !seen.contains(p);
            seen.push(p.clone());
            new
        });
        let mut d = Self {
            purpose,
            dir_input: dir.to_string_lossy().into_owned(),
            dir,
            name,
            show_all: false,
            show_hidden: false,
            entries: Vec::new(),
            error: None,
            places,
        };
        d.refresh();
        d
    }

    fn enter(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.dir_input = self.dir.to_string_lossy().into_owned();
        self.refresh();
    }

    fn refresh(&mut self) {
        self.entries.clear();
        self.error = None;
        let rd = match std::fs::read_dir(&self.dir) {
            Ok(rd) => rd,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') && !self.show_hidden {
                continue;
            }
            let path = e.path();
            let is_dir = path.is_dir();
            if !is_dir && !self.show_all && !self.purpose.offers(&name) {
                continue;
            }
            let size = if is_dir { 0 } else { e.metadata().map_or(0, |m| m.len()) };
            let project = is_dir && self.purpose.picks_projects() && crate::project_dir::is_project(&path);
            self.entries.push(Entry { path, name, is_dir, project, size });
        }
        self.entries
            .sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    }

    /// The file the confirm button would pick.
    fn target(&self) -> Option<PathBuf> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        let mut path = if Path::new(name).is_absolute() { PathBuf::from(name) } else { self.dir.join(name) };
        let ext = self.purpose.extension();
        if self.purpose.saving() && !ext.is_empty() && !self.purpose.offers(name) {
            path.set_file_name(format!("{name}{ext}"));
        }
        Some(path)
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<Event> {
        let mut event = None;
        let modal = egui::Modal::new(egui::Id::new("file_dialog")).show(ctx, |ui| {
            ui.set_width(620.0);
            ui.label(RichText::new(self.purpose.title()).heading().color(theme::SELECTED));
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if super::icons::button(ui, super::icons::Icon::Up).on_hover_text("Parent folder").clicked()
                    && let Some(p) = self.dir.parent().map(Path::to_path_buf)
                {
                    self.enter(p);
                }
                let edit = ui.add(egui::TextEdit::singleline(&mut self.dir_input).desired_width(f32::INFINITY));
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    self.enter(PathBuf::from(self.dir_input.trim()));
                }
            });
            ui.add_space(2.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(120.0);
                    theme::caption(ui, "PLACES");
                    let mut go = None;
                    for (label, path) in &self.places {
                        if ui.selectable_label(&self.dir == path, label).clicked() {
                            go = Some(path.clone());
                        }
                    }
                    if let Some(p) = go {
                        self.enter(p);
                    }
                    ui.add_space(8.0);
                    let kinds = self.purpose.kinds();
                    if ui.checkbox(&mut self.show_all, "All files").on_hover_text(format!("Not just {kinds}")).changed()
                        || ui.checkbox(&mut self.show_hidden, "Hidden files").changed()
                    {
                        self.refresh();
                    }
                });
                self.list(ui, &mut event);
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Name");
                let edit = ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(380.0));
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Cancel").clicked() {
                        event = Some(Event::Cancelled);
                    }
                    let target = self.target();
                    let ok = ui.add_enabled(target.is_some(), egui::Button::new(self.purpose.confirm()));
                    if ok.clicked() || enter {
                        event = self.confirm(target);
                    }
                });
            });
            if let Some(e) = &self.error {
                ui.colored_label(theme::RECORD, e);
            }
        });
        if modal.should_close() && event.is_none() {
            event = Some(Event::Cancelled);
        }
        event
    }

    fn list(&mut self, ui: &mut egui::Ui, event: &mut Option<Event>) {
        let mut enter = None;
        let mut confirm = None;
        egui::Frame::new().fill(theme::INSET).inner_margin(3).show(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink(false).max_height(320.0).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                    for e in &self.entries {
                        let selected = (!e.is_dir || e.project) && self.name == e.name;
                        let resp = if e.project {
                            let kind = RichText::new("project").color(theme::TEXT_WEAK);
                            ui.add(egui::Button::selectable(selected, e.name.as_str()).right_text(kind))
                        } else if e.is_dir {
                            ui.selectable_label(false, RichText::new(format!("{}/", e.name)).color(theme::SELECTED))
                        } else {
                            let size = RichText::new(human_size(e.size)).color(theme::TEXT_WEAK);
                            ui.add(egui::Button::selectable(selected, e.name.as_str()).right_text(size))
                        };
                        if resp.clicked() && (!e.is_dir || e.project) {
                            self.name = e.name.clone();
                            if matches!(self.purpose, Purpose::LoadSample(_)) {
                                *event = Some(Event::Preview(e.path.clone()));
                            }
                        }
                        if resp.double_clicked() {
                            if e.is_dir && !e.project {
                                enter = Some(e.path.clone());
                            } else {
                                confirm = Some(e.path.clone());
                            }
                        }
                    }
                    if self.entries.is_empty() && self.error.is_none() {
                        ui.label(RichText::new("No matching files.").color(theme::TEXT_WEAK));
                    }
                });
            });
        });
        if let Some(d) = enter {
            self.enter(d);
        }
        if let Some(p) = confirm {
            *event = self.confirm(Some(p));
        }
    }

    fn confirm(&mut self, target: Option<PathBuf>) -> Option<Event> {
        let path = target?;
        let project = self.purpose.picks_projects() && crate::project_dir::is_project(&path);
        if path.is_dir() && !project {
            self.name.clear();
            self.enter(path);
            return None;
        }
        if !self.purpose.saving() && !path.exists() {
            self.error = Some(format!("{} does not exist", path.display()));
            return None;
        }
        Some(Event::Picked(self.purpose, path))
    }
}

fn human_size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1 << 20) as f64),
        b if b >= 1 << 10 => format!("{} KB", b >> 10),
        b => format!("{b} B"),
    }
}
