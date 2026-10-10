//! The menu bar along the top of the window.

use super::*;

impl App {
    /// The menu bar along the top of the window.
    pub(super) fn menu_bar(&mut self, ui: &mut egui::Ui) {
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
}

/// A menu entry with its shortcut on the right; true when clicked.
fn menu_item(ui: &mut egui::Ui, label: &str, shortcut: &str) -> bool {
    ui.add(egui::Button::new(label).shortcut_text(shortcut)).clicked()
}
