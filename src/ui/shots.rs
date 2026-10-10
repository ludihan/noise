//! Pictures of the app drawn without a window or a sound card, on the GPU
//! or its software stand-in, to look at changes to the interface:
//! `cargo test --release shots -- --ignored` writes them to `target/shots`,
//! or where `NOISE_SHOTS` says.

use super::{App, View, sampler};
use egui_kittest::Harness;
use std::path::PathBuf;

/// The app on the demo song, drawn at the size the window opens at, after
/// `setup` has picked what to show.
fn shot(name: &str, setup: impl FnOnce(&mut App)) {
    let mut harness =
        Harness::builder().with_size([1400.0, 900.0]).wgpu().build_eframe(|cc| App::start(cc, None, false));
    setup(harness.state_mut());
    harness.run_steps(4);
    let image = harness.render().expect("render");
    let dir = std::env::var_os("NOISE_SHOTS")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/shots"), PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore = "draws pictures to look at; run with --ignored"]
fn shots() {
    shot("pattern", |_| {});
    shot("mixer", |app| app.view = View::Mixer);
    shot("instrument", |app| {
        // A synth with effects of its own and a macro moving two of them.
        let p = &mut app.project;
        let inst = p
            .modules
            .iter()
            .find(|m| m.kind.has_macros() && !m.kind.holds_samples() && !p.chain(m.id).effects.is_empty());
        let inst = inst.expect("a synth with effects").id;
        let fx = p.chain(inst).effects[0];
        p.map_macro(inst, 0, inst, 0);
        p.map_macro(inst, 0, fx, 0);
        p.module_mut(inst).unwrap().macro_mut(0).name = "Brightness".into();
        p.module_mut(inst).unwrap().macro_mut(1).value = 0.4;
        app.selected_module = Some(inst);
        app.view = View::Sampler;
        app.sampler.synth_tab = sampler::SynthTab::Synth;
    });
}
