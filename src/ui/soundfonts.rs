//! Loading soundfonts into a Sampler, and the window that picks a preset
//! of an SF2 or SF3 file.

use super::{App, theme};
use crate::soundfont::{self, Preset};
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

/// A soundfont being read on a thread of its own, as decoding a large one
/// takes a while: where it goes and what comes back.
pub struct Loading {
    module: u8,
    path: PathBuf,
    result: Receiver<Result<soundfont::Instrument, String>>,
}

/// The presets of a soundfont, to pick the one loaded into `module`.
pub struct PresetPicker {
    module: u8,
    path: PathBuf,
    presets: Vec<Preset>,
    filter: String,
    selected: usize,
}

/// Loads the soundfont at `path` into Sampler `id`: an SFZ file at once,
/// an SF2 or SF3 file after its preset is picked when it has several.
pub fn open(app: &mut App, id: u8, path: &Path) -> bool {
    if soundfont::is_sfz(path) {
        return load(app, id, path, 0);
    }
    match soundfont::presets(path) {
        Ok(presets) if presets.len() == 1 => load(app, id, path, 0),
        Ok(presets) if !presets.is_empty() => {
            app.preset_picker = Some(PresetPicker {
                module: id,
                path: path.to_path_buf(),
                presets,
                filter: String::new(),
                selected: 0,
            });
            true
        }
        Ok(_) => {
            app.set_status(format!("{} has no presets", path.display()));
            false
        }
        Err(e) => {
            app.set_status(format!("Could not load {}: {e}", path.display()));
            false
        }
    }
}

/// Starts reading preset `preset` of the soundfont at `path` into Sampler
/// `id`, on a thread; `poll` puts it in place when it is ready.
fn load(app: &mut App, id: u8, path: &Path, preset: usize) -> bool {
    let (tx, rx) = channel();
    let file = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(soundfont::load(&file, preset));
    });
    let name = path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    app.set_status(format!("Loading {name}…"));
    app.soundfont_loads.push(Loading { module: id, path: path.to_path_buf(), result: rx });
    true
}

/// Puts the soundfonts that finished loading in place.
pub fn poll(app: &mut App) {
    let mut k = 0;
    while k < app.soundfont_loads.len() {
        let done = match app.soundfont_loads[k].result.try_recv() {
            Ok(result) => Some(result),
            Err(std::sync::mpsc::TryRecvError::Empty) => None,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("the loader stopped".into())),
        };
        match done {
            Some(result) => {
                let job = app.soundfont_loads.swap_remove(k);
                match result {
                    Ok(inst) => apply(app, job.module, &job.path, inst),
                    Err(e) => app.set_status(format!("Could not load {}: {e}", job.path.display())),
                }
            }
            None => k += 1,
        }
    }
}

/// Replaces Sampler `id`'s samples with those of `inst`, read from the
/// soundfont at `path`, and sets its envelope and name from it.
fn apply(app: &mut App, id: u8, path: &Path, inst: soundfont::Instrument) {
    let Some(m) = app.project.module_mut(id) else { return };
    let count = inst.slots.len();
    m.samples = inst.slots;
    m.name = inst.name.clone();
    if let Some(env) = inst.envelope {
        // Attack, decay, sustain and release follow volume, pan and transpose.
        for (k, v) in env.into_iter().enumerate() {
            let spec = &m.kind.params()[3 + k];
            m.params[3 + k] = v.clamp(spec.min, spec.max);
        }
    }
    app.sampler.sample = 0;
    app.sampler.slice = None;
    app.set_status(format!("Loaded {} ({count} samples) from {}", inst.name, path.display()));
    app.mark();
}

/// The preset picker, while a soundfont waits for one.
pub fn window(app: &mut App, ctx: &egui::Context) {
    poll(app);
    let Some(picker) = &mut app.preset_picker else { return };
    let file = picker.path.file_name().map_or(String::new(), |n| n.to_string_lossy().into_owned());
    let mut open = true;
    let mut chosen = None;
    egui::Window::new(format!("Load Preset from {file}"))
        .open(&mut open)
        .default_size([360.0, 420.0])
        .pivot(egui::Align2::CENTER_CENTER)
        .default_pos(ctx.content_rect().center())
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                theme::caption(ui, "FIND");
                ui.add(
                    egui::TextEdit::singleline(&mut picker.filter)
                        .desired_width(f32::INFINITY)
                        .hint_text("Preset name"),
                );
            });
            let filter = picker.filter.to_lowercase();
            let shown: Vec<usize> = (0..picker.presets.len())
                .filter(|&i| picker.presets[i].name.to_lowercase().contains(&filter))
                .collect();
            egui::Frame::new().fill(theme::INSET).inner_margin(3).show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).max_height(320.0).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                        for &i in &shown {
                            let p = &picker.presets[i];
                            let number = RichText::new(format!("{:03}:{:03}", p.bank, p.program))
                                .monospace()
                                .color(theme::TEXT_WEAK);
                            let resp = ui.add(
                                egui::Button::selectable(i == picker.selected, p.name.as_str()).right_text(number),
                            );
                            if resp.clicked() {
                                picker.selected = i;
                            }
                            if resp.double_clicked() {
                                chosen = Some(i);
                            }
                        }
                        if shown.is_empty() {
                            ui.label(RichText::new("No matching presets.").color(theme::TEXT_WEAK));
                        }
                    });
                });
            });
            ui.horizontal(|ui| {
                if ui.button("Load").on_hover_text("Replace the Sampler's samples with this preset's").clicked() {
                    chosen = Some(picker.selected);
                }
                ui.label(RichText::new(format!("{} presets", picker.presets.len())).color(theme::TEXT_WEAK));
            });
        });
    if let Some(i) = chosen {
        let (id, path) = (picker.module, picker.path.clone());
        app.preset_picker = None;
        load(app, id, &path, i);
    } else if !open {
        app.preset_picker = None;
    }
}
