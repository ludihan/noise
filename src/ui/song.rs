//! Songs and their files: new, open, save, import and export, rendering,
//! the file dialog and its confirmations, and loading samples.

use super::*;
use crate::audio::Rendered;

impl App {
    /// Opens demo song `i` of `DEMOS` as a new, unsaved song.
    pub(super) fn open_demo(&mut self, i: usize) {
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

    pub(super) fn new_project(&mut self) {
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
    pub(super) fn song_replaced(&mut self) {
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
    pub(super) fn save(&mut self) {
        self.save_then(None);
    }

    /// Saves a copy of the song on a thread, then does `then`: what was
    /// waiting for it to be saved, such as quitting.
    pub(super) fn save_then(&mut self, then: Option<Pending>) {
        let (mut song, path, mut hashes) = (self.project.clone(), self.path.clone(), self.hashes.clone());
        // Edits made while it saves make the song unsaved again.
        self.modified = false;
        jobs::spawn(self, format!("Saving {}", file_name(&path)), move |_| {
            let saved = project_dir::save(&mut song, std::path::Path::new(&path), &mut hashes);
            Box::new(move |app: &mut App| {
                app.hashes = hashes;
                app.saved(path, &song, saved, then);
            })
        });
    }

    /// Takes in what saving `song` to `path` did: its samples now point at
    /// their files (those still in the song), and what waited for it goes
    /// on, or asks again if the song was changed meanwhile.
    fn saved(
        &mut self,
        path: String,
        song: &Project,
        saved: Result<project_dir::Saved, String>,
        then: Option<Pending>,
    ) {
        let saved = match saved {
            Ok(saved) => saved,
            Err(e) => {
                self.modified = true;
                self.status = format!("Save failed: {e}");
                return;
            }
        };
        let written: Vec<&SampleSlot> = song.modules.iter().flat_map(|m| &m.samples).collect();
        for slot in self.project.modules.iter_mut().flat_map(|m| &mut m.samples) {
            let same =
                |s: &&&SampleSlot| s.data.as_ref().zip(slot.data.as_ref()).is_some_and(|(a, b)| Arc::ptr_eq(a, b));
            if let Some(s) = written.iter().find(same) {
                (slot.path, slot.unsaved) = (s.path.clone(), false);
            }
        }
        if path == self.path {
            self.untitled = false;
        }
        let mut done = Vec::new();
        if saved.written > 0 {
            done.push(format!("{} samples written", saved.written));
        }
        if saved.removed > 0 {
            done.push(format!("{} no longer played deleted", saved.removed));
        }
        let done = if done.is_empty() { String::new() } else { format!(" ({})", done.join(", ")) };
        self.status = format!("Saved {path}{done}");
        match then {
            Some(p) if self.modified => self.confirm = Some(p),
            Some(p) => self.perform(p),
            None => {}
        }
    }

    /// Opens the song at `path`, a project folder or a song file, reading it
    /// on a thread.
    pub(super) fn open(&mut self, path: String) {
        let name = file_name(&path);
        jobs::spawn(self, format!("Opening {name}"), move |_| {
            let read = project_dir::open(std::path::Path::new(&path));
            Box::new(move |app: &mut App| app.opened(path, read))
        });
    }

    /// Puts the song read from `path` in place, or says why it couldn't be.
    pub(super) fn opened(&mut self, path: String, read: Result<(Project, Vec<String>), String>) {
        match read {
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
    pub(super) fn import(&mut self, path: &str) {
        let path = path.to_string();
        jobs::spawn(self, format!("Importing {}", file_name(&path)), move |_| {
            let read = project_dir::import_beside(std::path::Path::new(&path)).and_then(|dir| {
                let dir = dir.to_string_lossy().into_owned();
                project_dir::open(std::path::Path::new(&dir)).map(|read| (dir, read))
            });
            Box::new(move |app: &mut App| match read {
                Ok((dir, read)) => app.opened(dir, Ok(read)),
                Err(e) => app.status = format!("Import failed: {e}"),
            })
        });
    }

    /// Writes the song, with every sample it plays, to the `.noise` file at
    /// `path`, as a project folder named after the song.
    pub(super) fn export_project(&mut self, path: &str) {
        let (name, path, project) = (self.song_name(), path.to_string(), self.project.clone());
        // The hashes go with it, and come back with any it worked out.
        let mut hashes = std::mem::take(&mut self.hashes);
        jobs::spawn(self, format!("Exporting {}", file_name(&path)), move |_| {
            let done = project_dir::export(&project, &name, std::path::Path::new(&path), &mut hashes);
            Box::new(move |app: &mut App| {
                app.hashes = hashes;
                app.status = match done {
                    Ok(()) => format!("Exported the project to {path}"),
                    Err(e) => format!("Export failed: {e}"),
                };
            })
        });
    }

    /// Does `p`, first asking about unsaved changes if there are any.
    pub(super) fn request(&mut self, p: Pending) {
        if self.modified {
            self.confirm = Some(p);
        } else {
            self.perform(p);
        }
    }

    pub(super) fn perform(&mut self, p: Pending) {
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
    pub(super) fn confirm_dialog(&mut self, ctx: &egui::Context) {
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
                self.save_then(Some(pending));
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

    pub(super) fn handle_dropped_files(&mut self, ctx: &egui::Context) {
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
    pub(super) fn render_format(&self) -> audio::RenderFormat {
        let device = self.audio.as_ref().map_or(44100, |a| a.sample_rate);
        let rate = if self.render_rate == 0 { device } else { self.render_rate };
        audio::RenderFormat { sample_rate: rate, depth: self.render_depth }
    }

    pub(super) fn export(&mut self, path: &str) {
        let (path, project, format) = (path.to_string(), Arc::new(self.project.clone()), self.render_format());
        jobs::spawn(self, format!("Rendering {}", file_name(&path)), move |progress| {
            let done = audio::export_wav(project, &path, format, &mut |f| progress.set(f));
            Box::new(move |app: &mut App| {
                app.status = match done {
                    Ok(Rendered::Done) => format!("Rendered {path}"),
                    Ok(Rendered::Cancelled) => "Render cancelled".to_string(),
                    Ok(Rendered::TooLong) => format!("Rendered {path}, cut off after {}", too_long()),
                    Err(e) => format!("Render failed: {e}"),
                };
            })
        });
    }

    pub(super) fn export_stems(&mut self, path: &str) {
        let (path, project, format) = (path.to_string(), self.project.clone(), self.render_format());
        jobs::spawn(self, format!("Rendering stems of {}", file_name(&path)), move |progress| {
            let done = audio::export_stems(&project, &path, format, &mut |f| progress.set(f));
            Box::new(move |app: &mut App| {
                app.status = match done {
                    Ok((files, Rendered::Cancelled)) => format!("Render cancelled after {} stems", files.len()),
                    Ok((files, Rendered::Done)) => format!("Rendered {} stems next to {path}", files.len()),
                    Ok((files, Rendered::TooLong)) => {
                        format!("Rendered {} stems next to {path}, cut off after {}", files.len(), too_long())
                    }
                    Err(e) => format!("Render failed: {e}"),
                };
            })
        });
    }

    /// The render dialog: the sample rate and format, before the file
    /// to render the song or its stems to.
    pub(super) fn render_dialog(&mut self, ctx: &egui::Context) {
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
    pub(super) fn song_dir(&self) -> Option<std::path::PathBuf> {
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
    pub(super) fn pick_backup(&mut self) {
        let Some(dir) = backup::dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        self.file_dialog = Some(files::FileDialog::new(files::Purpose::OpenSong, dir, String::new(), self.song_dir()));
    }

    pub(super) fn file_dialog(&mut self, ctx: &egui::Context) {
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
                        let then = self.after_save.take();
                        self.save_then(then);
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

    pub(super) fn preview_file(&mut self, path: &std::path::Path) {
        if !crate::sample::is_audio(path) {
            return;
        }
        match Sample::load(path) {
            Ok(s) => self.preview(Arc::new(s), None),
            Err(e) => self.status = format!("Could not load {}: {e}", path.display()),
        }
    }

    /// Saves to the song's file, or asks for one if it hasn't been saved yet.
    pub(super) fn save_or_ask(&mut self) {
        if self.untitled {
            self.pick_file(files::Purpose::SaveSong);
        } else {
            self.save();
        }
    }

    pub(super) fn song_name(&self) -> String {
        std::path::Path::new(&self.path).file_name().unwrap_or_default().to_string_lossy().into_owned()
    }
}

/// What a render cut off at its longest says it was cut off after.
fn too_long() -> String {
    format!("{} hours: does the song ever end?", audio::MAX_RENDER_SECONDS / 3600)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;

    /// Runs frames until `app` has no jobs left.
    fn finish_jobs(harness: &mut Harness<'_, App>) {
        let start = std::time::Instant::now();
        while !harness.state().jobs.is_empty() {
            assert!(start.elapsed().as_secs() < 60, "the jobs never ended");
            harness.step();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn saving_runs_beside_the_window_and_keeps_edits_made_meanwhile() {
        let dir = std::env::temp_dir().join(format!("noise-save-{}", std::process::id()));
        let mut harness = Harness::builder().build_eframe(|cc| App::start(cc, None, false));
        let app = harness.state_mut();
        (app.path, app.untitled) = (dir.to_string_lossy().into_owned(), false);
        app.save();
        assert!(!app.jobs.is_empty(), "it saves on a thread");
        // An edit while it saves leaves the song unsaved.
        app.project.title = "Changed".into();
        app.mark();
        finish_jobs(&mut harness);
        let app = harness.state();
        assert!(dir.join(project_dir::SONG_FILE).exists(), "{}", app.status);
        assert!(app.modified, "the edit made meanwhile isn't saved");
        let slots: Vec<&SampleSlot> = app.project.modules.iter().flat_map(|m| &m.samples).collect();
        assert!(
            !slots.is_empty() && slots.iter().all(|s| !s.unsaved && s.path.is_some()),
            "samples point at their files"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
