//! Pictures of the app drawn without a window or a sound card, on the GPU
//! or its software stand-in, to look at changes to the interface:
//! `cargo test --release shots -- --ignored` writes them to `target/shots`,
//! or where `NOISE_SHOTS` says.

use super::{App, View, sampler};
use crate::project::{ModuleKind, OUTPUT_ID};
use egui_kittest::Harness;
use std::path::PathBuf;

/// The app on the demo song, drawn at the size the window opens at, after
/// `setup` has picked what to show.
fn shot(name: &str, setup: impl FnOnce(&mut App)) {
    shot_at(name, [1400.0, 900.0], setup);
}

/// `shot` with the window at `size`, to show more of the lower frame.
fn shot_at(name: &str, size: [f32; 2], setup: impl FnOnce(&mut App)) {
    let mut harness = Harness::builder().with_size(size).wgpu().build_eframe(|cc| App::start(cc, None, false));
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
    shot("neighbours", |app| {
        // Near the top of the third slot: the second one's end shows above.
        (app.slot, app.cursor.line) = (2, 3);
    });
    shot("automation", |app| {
        // An envelope repeating every beat.
        // The first envelope, which the panel shows first.
        let env = app.pattern_mut().automation.first_mut().expect("an envelope in the demo's first pattern");
        env.every = 1.0;
        (env.points, env.steps, env.curve) = super::automation::lfo_points(0, 4.0, 0.5);
        (app.lower, app.show_lower) = (super::Lower::Automation, true);
        super::automation::pick(app, &[1, 2]);
    });
    shot("busy", |app| {
        // A job that waits, part way through, until the picture is taken.
        super::jobs::spawn(app, "Rendering song.wav", |progress| {
            for _ in 0..50 {
                progress.set(0.42);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Box::new(|_: &mut App| {})
        });
    });
    shot("instrument", |app| {
        // A synth with effects of its own and a macro moving two of them.
        let p = &mut app.project;
        let inst = p
            .modules
            .iter()
            .find(|m| m.kind.plays_sound() && !m.kind.holds_samples() && !p.chain(m.id).effects.is_empty());
        let inst = inst.expect("a synth with effects").id;
        let fx = p.chain(inst).effects[0];
        p.map_macro(inst, 0, inst, 0);
        p.map_macro(inst, 0, fx, 0);
        p.module_mut(inst).unwrap().macro_mut(0).name = "Brightness".into();
        p.module_mut(inst).unwrap().macro_mut(1).value = 0.4;
        // A Modulator moving two parameters of the effect, one upside down.
        let lfo = p.add_module(ModuleKind::Modulator, [0.0, 0.0]).unwrap();
        p.add_control(lfo, fx, 1);
        p.add_control(lfo, fx, 2);
        p.module_mut(lfo).unwrap().controls[1].depth = -0.6;
        app.selected_module = Some(inst);
        app.view = View::Sampler;
        app.sampler.synth_tab = sampler::SynthTab::Synth;
    });
    shot_at("modulator", [1400.0, 1300.0], |app| {
        // A Modulator moving two parameters of an effect, selected in the
        // module list.
        let p = &mut app.project;
        let fx = p.modules.iter().find(|m| m.kind == ModuleKind::Distortion).expect("a Distortion").id;
        let lfo = p.add_module(ModuleKind::Modulator, [0.0, 0.0]).unwrap();
        p.add_control(lfo, fx, 1);
        p.add_control(lfo, fx, 2);
        p.module_mut(lfo).unwrap().controls[1].depth = -0.6;
        app.selected_module = Some(lfo);
        app.show_chain(crate::project::Owner::Module(OUTPUT_ID));
        app.lower_height = 640.0;
    });
    shot("send_track", |app| {
        // Last Light's hall, which most of its instruments send to.
        app.open_demo(0);
        let hall = app.project.sends[0].module;
        app.show_send(hall);
    });
    shot_at("sends", [2300.0, 900.0], |app| {
        // An instrument whose chain ends with a Send.
        app.open_demo(0);
        let p = &app.project;
        let sends = |id: u8| p.chain(id).effects.iter().any(|&e| p.module(e).unwrap().kind == ModuleKind::Send);
        let inst = p.modules.iter().find(|m| m.kind.plays_sound() && sends(m.id)).expect("one that sends").id;
        app.selected_module = Some(inst);
        app.view = View::Sampler;
        app.sampler.synth_tab = sampler::SynthTab::Synth;
    });
    shot("mixer_sends", |app| {
        app.open_demo(0);
        app.view = View::Mixer;
    });
}
