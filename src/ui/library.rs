//! Instrument presets: an instrument with its
//! samples, voice modulation and phrases, and the effects of its own
//! chain. Factory ones are the demo song's instruments, built into the
//! program; the user's are kept in `instruments/` in the data folder, each
//! a JSON file with its samples as WAV files in a folder beside it.

use super::presets::{file_name, names};
use super::{App, theme};
use crate::project::{Module, OUTPUT_ID, Project};
use crate::sample::Sample;
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstrumentPreset {
    pub name: String,
    /// The instrument; a Sampler's slots point at WAV files beside the
    /// preset, relative to its folder.
    pub instrument: Module,
    /// The effects of its own chain, in order.
    pub effects: Vec<Module>,
}

impl InstrumentPreset {
    /// Instrument `id` of `project` and its own chain. MultiSynths, which
    /// only pass notes to other instruments, can't be presets.
    pub fn of(project: &Project, id: u8) -> Option<Self> {
        let m = project.module(id).filter(|m| m.kind.is_instrument() && !m.kind.notes_only())?;
        let effects = project.chain(id).effects.iter().filter_map(|&e| project.module(e).cloned()).collect();
        Some(Self { name: m.name.clone(), instrument: m.clone(), effects })
    }

    /// Adds the instrument and its chain to `project`, sending to the
    /// output. Its samples are written next to the song when it is saved.
    /// Returns the new instrument.
    pub fn add_to(&self, project: &mut Project, pos: [f32; 2]) -> Option<u8> {
        let id = project.add_module(self.instrument.kind, pos)?;
        let m = project.module_mut(id)?;
        m.take_sound(&self.instrument);
        for slot in &mut m.samples {
            slot.path = None;
            slot.unsaved = slot.data.is_some();
        }
        for (k, effect) in self.effects.iter().enumerate() {
            let e = project.chain_insert(id, k, effect.kind)?;
            project.module_mut(e)?.take_sound(effect);
        }
        if self.effects.is_empty() {
            project.connect(id, OUTPUT_ID);
        }
        Some(id)
    }
}

/// The factory presets: the demo songs' instruments, made once.
pub fn factory() -> &'static [InstrumentPreset] {
    static FACTORY: OnceLock<Vec<InstrumentPreset>> = OnceLock::new();
    FACTORY.get_or_init(|| {
        let mut list: Vec<InstrumentPreset> = Vec::new();
        for (_, make) in super::DEMOS {
            let demo = make();
            list.extend(demo.modules.iter().filter_map(|m| InstrumentPreset::of(&demo, m.id)));
        }
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    })
}

/// The factory device presets of `kind`: the settings of the demo song's
/// modules of that kind, by their names.
pub fn factory_devices(kind: crate::project::ModuleKind) -> Vec<(String, super::presets::Preset)> {
    static DEMOS: OnceLock<Vec<Project>> = OnceLock::new();
    let demos = DEMOS.get_or_init(|| super::DEMOS.iter().map(|(_, make)| make()).collect());
    let mut list: Vec<(String, super::presets::Preset)> = demos
        .iter()
        .flat_map(|demo| demo.modules.iter())
        .filter(|m| m.kind == kind)
        .map(|m| (m.name.clone(), super::presets::Preset::of(m)))
        .collect();
    list.sort_by(|a, b| a.0.cmp(&b.0));
    list
}

/// Where the user's instrument presets are kept.
pub fn folder() -> Option<PathBuf> {
    super::presets::base().map(|d| d.join("instruments"))
}

/// Saves `preset` in `dir` under `name`, with its samples as WAV files in
/// a folder named after it.
pub fn save(dir: &Path, name: &str, preset: &InstrumentPreset) -> Result<(), String> {
    let name = file_name(name);
    if name.is_empty() {
        return Err("A preset needs a name".into());
    }
    let samples = format!("{name} samples");
    let mut preset = InstrumentPreset { name: name.clone(), ..preset.clone() };
    let with_audio = preset.instrument.samples.iter().any(|s| s.data.is_some());
    if with_audio {
        let folder = dir.join(&samples);
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        for (i, slot) in preset.instrument.samples.iter_mut().enumerate() {
            let Some(data) = &slot.data else { continue };
            let file = format!("{i:02} {}.wav", file_name(&slot.name));
            data.save(&folder.join(&file))?;
            slot.path = Some(format!("{samples}/{file}"));
        }
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(&preset).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{name}.json")), json).map_err(|e| e.to_string())
}

/// Loads the preset `name` from `dir`, with its samples.
pub fn load(dir: &Path, name: &str) -> Result<InstrumentPreset, String> {
    let text = std::fs::read_to_string(dir.join(format!("{name}.json"))).map_err(|e| e.to_string())?;
    let mut preset: InstrumentPreset = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    for slot in &mut preset.instrument.samples {
        if let Some(path) = &slot.path {
            let sample = Sample::load(&dir.join(path)).map_err(|e| format!("{path}: {e}"))?;
            slot.data = Some(Arc::new(sample));
        }
    }
    Ok(preset)
}

/// Adds `preset` to the song as a new instrument and selects it.
fn add(app: &mut App, preset: &InstrumentPreset) {
    let y = app.project.modules.iter().filter(|m| m.kind.is_instrument()).map(|m| m.pos[1] + 70.0).fold(20.0, f32::max);
    match preset.add_to(&mut app.project, [40.0, y]) {
        Some(id) => {
            app.selected_module = Some(id);
            app.set_status(format!("Added the instrument {}", preset.name));
            app.mark();
        }
        None => app.set_status("Too many modules"),
    }
}

/// The instrument list's preset menus: the factory's and the user's, to
/// add as a new instrument.
pub fn add_menus(app: &mut App, ui: &mut egui::Ui) {
    ui.menu_button("Factory Instruments", |ui| {
        for preset in factory() {
            let kind = egui::RichText::new(preset.instrument.kind.name()).small().color(theme::TEXT_WEAK);
            if ui.add(egui::Button::new(&preset.name).right_text(kind)).clicked() {
                add(app, preset);
                ui.close();
            }
        }
    });
    ui.menu_button("My Instruments", |ui| {
        let Some(dir) = folder() else {
            ui.label("There is no data folder to keep instruments in.");
            return;
        };
        let names = names(&dir);
        if names.is_empty() {
            ui.label(egui::RichText::new("None yet: right-click an instrument and save it").color(theme::TEXT_WEAK));
        }
        for name in names {
            if ui.button(&name).clicked() {
                match load(&dir, &name) {
                    Ok(preset) => add(app, &preset),
                    Err(e) => app.set_status(format!("Could not load the instrument {name}: {e}")),
                }
                ui.close();
            }
        }
    });
}

/// The part of an instrument's menu that saves it as a preset.
pub fn save_menu(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let Some(preset) = InstrumentPreset::of(&app.project, id) else { return };
    let Some(dir) = folder() else { return };
    let key = egui::Id::new(("instrument_preset_name", id));
    let mut name: String = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| preset.name.clone());
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0).hint_text("Name"));
        if ui
            .button("Save as Preset")
            .on_hover_text(format!("Keep this instrument and its effects in {}", dir.display()))
            .clicked()
        {
            match save(&dir, &name, &preset) {
                Ok(()) => app.set_status(format!("Saved the instrument {}", name.trim())),
                Err(e) => app.set_status(format!("Could not save the instrument: {e}")),
            }
            ui.close();
        }
    });
    ui.data_mut(|d| d.insert_temp(key, name));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_factory_has_the_demos_instruments() {
        let names: Vec<&str> = factory().iter().map(|p| p.name.as_str()).collect();
        for name in ["Felt Piano", "Strings", "Sub Bass", "Bells", "Ripple", "Swell", "Drums"] {
            assert!(names.contains(&name), "{name} in {names:?}");
        }
        let piano = factory().iter().find(|p| p.name == "Felt Piano").unwrap();
        assert_eq!(piano.instrument.samples.len(), 2);
        assert!(piano.instrument.samples.iter().all(|s| s.data.is_some()), "with their audio");
        let strings = factory().iter().find(|p| p.name == "Strings").unwrap();
        assert_eq!(strings.effects.len(), 4, "its chain: ensemble, phaser, EQ and width");
    }

    #[test]
    fn presets_save_load_and_add_with_their_samples_and_chain() {
        let dir = std::env::temp_dir().join(format!("noise-instruments-{}", std::process::id()));
        let piano = factory().iter().find(|p| p.name == "Felt Piano").unwrap();
        save(&dir, "My/Piano", piano).unwrap();
        assert_eq!(names(&dir), ["My_Piano"]);
        let back = load(&dir, "My_Piano").unwrap();
        assert_eq!(back.instrument.samples.len(), 2);
        let (a, b) =
            (back.instrument.samples[0].data.as_ref().unwrap(), piano.instrument.samples[0].data.as_ref().unwrap());
        assert_eq!(a.len(), b.len(), "the audio came back");
        let mut song = Project::empty();
        let id = back.add_to(&mut song, [0.0; 2]).unwrap();
        let m = song.module(id).unwrap();
        assert_eq!((m.name.as_str(), m.samples.len()), ("Felt Piano", 2));
        assert!(m.samples.iter().all(|s| s.unsaved && s.path.is_none()), "written next to the song when saved");
        let chain = song.chain(id);
        assert_eq!(chain.effects.len(), 2, "its EQ and exciter");
        assert_eq!(chain.outputs, [OUTPUT_ID]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
