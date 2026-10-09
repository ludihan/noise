//! Device presets: a module's settings saved under a name,
//! in `presets/<kind>/<name>.json` in the data folder, to load into any
//! module of the same kind.

use super::{App, theme};
use crate::project::{Modulation, Module, ModuleKind};
use eframe::egui;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    pub kind: ModuleKind,
    pub params: Vec<f32>,
    /// An instrument's voice modulation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modulation: Option<Modulation>,
}

impl Preset {
    pub fn of(m: &Module) -> Preset {
        Preset {
            kind: m.kind,
            params: m.params.clone(),
            modulation: m.kind.has_modulation().then(|| m.modulation.clone()),
        }
    }

    /// Gives `m` these settings; parameters the preset lacks keep their
    /// defaults, and values outside a parameter's range are brought in.
    pub fn apply(&self, m: &mut Module) {
        if self.kind != m.kind {
            return;
        }
        for (i, spec) in m.kind.params().iter().enumerate() {
            m.params[i] = self.params.get(i).copied().unwrap_or(spec.default).clamp(spec.min, spec.max);
        }
        if let Some(md) = &self.modulation {
            m.modulation = md.clone();
        }
    }
}

/// Where presets of `kind` are kept, under `base` (the config folder's
/// `noise` folder).
pub fn folder(base: &Path, kind: ModuleKind) -> PathBuf {
    base.join("presets").join(kind.name())
}

/// The folder presets are kept under: the data folder. Earlier versions
/// kept them in the config folder; they are moved over the first time.
pub fn base() -> Option<PathBuf> {
    let data = crate::paths::data_dir()?;
    if let Some(config) = crate::paths::config_dir() {
        crate::paths::migrate(&config.join("presets"), &data.join("presets"));
    }
    Some(data)
}

/// The names of the presets in `dir`, sorted.
pub fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "json"))
                .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|n| n.to_lowercase());
    names
}

/// A name that is safe as a file name.
pub fn file_name(name: &str) -> String {
    name.trim().chars().map(|c| if c.is_alphanumeric() || " -_.()".contains(c) { c } else { '_' }).collect()
}

pub fn save(dir: &Path, name: &str, preset: &Preset) -> Result<(), String> {
    let name = file_name(name);
    if name.is_empty() {
        return Err("A preset needs a name".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(preset).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(format!("{name}.json")), json).map_err(|e| e.to_string())
}

pub fn load(dir: &Path, name: &str) -> Result<Preset, String> {
    let text = std::fs::read_to_string(dir.join(format!("{name}.json"))).map_err(|e| e.to_string())?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// The Presets menu of module `id`: load one, save the settings as one,
/// or go back to the defaults.
pub fn menu(app: &mut App, ui: &mut egui::Ui, id: u8) {
    let Some(m) = app.project.module(id).cloned() else { return };
    let Some(base) = base() else {
        ui.label("There is no config folder to keep presets in.");
        return;
    };
    let dir = folder(&base, m.kind);
    // The factory's, from the demo song, then the user's.
    let factory = super::library::factory_devices(m.kind);
    if !factory.is_empty() {
        ui.menu_button("Factory", |ui| {
            for (name, preset) in &factory {
                if ui.button(name).clicked() {
                    preset.apply(app.project.module_mut(id).unwrap());
                    app.set_status(format!("Loaded the factory preset {name}"));
                    app.mark();
                    ui.close();
                }
            }
        });
    }
    let names = names(&dir);
    if names.is_empty() {
        ui.label(egui::RichText::new(format!("No {} presets of yours yet", m.kind.name())).color(theme::TEXT_WEAK));
    }
    for name in names {
        if ui.button(&name).clicked() {
            match load(&dir, &name) {
                Ok(p) => {
                    p.apply(app.project.module_mut(id).unwrap());
                    app.set_status(format!("Loaded the preset {name}"));
                    app.mark();
                }
                Err(e) => app.set_status(format!("Could not load the preset {name}: {e}")),
            }
            ui.close();
        }
    }
    ui.separator();
    // The name typed for a new preset, kept while the menu is open.
    let key = egui::Id::new(("preset_name", id));
    let mut name: String = ui.data(|d| d.get_temp(key)).unwrap_or_else(|| m.name.clone());
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut name).desired_width(120.0).hint_text("Name"));
        if ui.button("Save").on_hover_text(format!("Keep these settings in {}", dir.display())).clicked() {
            match save(&dir, &name, &Preset::of(&m)) {
                Ok(()) => app.set_status(format!("Saved the preset {}", name.trim())),
                Err(e) => app.set_status(format!("Could not save the preset: {e}")),
            }
            ui.close();
        }
    });
    ui.data_mut(|d| d.insert_temp(key, name));
    if ui.button("Reset to Defaults").clicked() {
        let defaults = Module::new(id, m.kind, m.pos);
        let target = app.project.module_mut(id).unwrap();
        target.params = defaults.params;
        target.modulation = defaults.modulation;
        app.mark();
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip_and_fit_their_module() {
        let base = std::env::temp_dir().join(format!("noise-presets-{}", std::process::id()));
        let dir = folder(&base, ModuleKind::Filter);
        assert!(names(&dir).is_empty());
        let mut filter = Module::new(1, ModuleKind::Filter, [0.0; 2]);
        filter.params[1] = 440.0;
        save(&dir, "Dark/Low", &Preset::of(&filter)).unwrap();
        assert_eq!(names(&dir), ["Dark_Low"], "no slashes in file names");
        let p = load(&dir, "Dark_Low").unwrap();
        let mut other = Module::new(2, ModuleKind::Filter, [0.0; 2]);
        p.apply(&mut other);
        assert_eq!(other.params, filter.params);
        // A preset from an older version with fewer parameters.
        let short = Preset { kind: ModuleKind::Filter, params: vec![1.0, 99999.0], modulation: None };
        short.apply(&mut other);
        assert_eq!(other.params[0], 1.0);
        assert_eq!(other.params[1], 20000.0, "kept in range");
        assert_eq!(other.params[2], ModuleKind::Filter.params()[2].default, "missing ones are defaults");
        // Another kind's preset does nothing.
        let mut eq = Module::new(3, ModuleKind::Eq, [0.0; 2]);
        let before = eq.params.clone();
        p.apply(&mut eq);
        assert_eq!(eq.params, before);
        std::fs::remove_dir_all(base).unwrap();
    }
}
