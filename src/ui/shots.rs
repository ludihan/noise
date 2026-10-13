//! Pictures of the app drawn without a window or a sound card, on the GPU
//! or its software stand-in, to look at changes to the interface:
//! `cargo test --release shots -- --ignored` writes them to `target/shots`,
//! or where `NOISE_SHOTS` says.

use super::{App, View, sampler};
use crate::project::ModuleKind;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use std::path::PathBuf;

/// The demo song the pictures show, Clockwork Rain, of `DEMOS`.
const DEMO: &str = "Clockwork Rain";

/// The slot playing pattern `name` of the song open.
fn slot_of(app: &App, name: &str) -> usize {
    let pattern = app.project.patterns.iter().position(|p| p.name == name).expect("a pattern by that name");
    app.project.order.iter().position(|s| s.pattern == pattern).expect("a slot playing it")
}

/// Opens the demo song on its busiest pattern, Storm 2, the cursor where
/// its lines are fullest.
fn open(app: &mut App) {
    app.open_demo(super::DEMOS.iter().position(|d| d.0 == DEMO).expect("the demo"));
    (app.slot, app.cursor.line) = (slot_of(app, "Storm 2"), 32);
    app.status = "Ready".into();
}

/// The app on the demo song, drawn at the size the window opens at, after
/// `setup` has picked what to show.
fn shot(name: &str, setup: impl FnOnce(&mut App)) {
    shot_at(name, [1400.0, 900.0], setup);
}

/// `shot` with the window at `size`, to show more of the lower frame.
fn shot_at(name: &str, size: [f32; 2], setup: impl FnOnce(&mut App)) {
    let mut harness = Harness::builder().with_size(size).wgpu().build_eframe(|cc| App::start(cc, None, false));
    open(harness.state_mut());
    setup(harness.state_mut());
    harness.run_steps(4);
    let image = harness.render().expect("render");
    let dir = std::env::var_os("NOISE_SHOTS")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/shots"), PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

/// The instrument list's + menu, open on its Factory Instruments, the
/// longest list in a menu, to see that it fits the window.
fn factory_menu_shot() {
    let mut harness =
        Harness::builder().with_size([1400.0, 900.0]).wgpu().build_eframe(|cc| App::start(cc, None, false));
    open(harness.state_mut());
    harness.run_steps(2);
    // The + furthest right is the instrument list's.
    let plus = harness.query_all_by_label("+").max_by(|a, b| a.rect().right().total_cmp(&b.rect().right()));
    plus.expect("the instrument list's +").click();
    harness.run_steps(2);
    harness.get_by_label_contains("Factory Instruments").click();
    harness.run_steps(4);
    let image = harness.render().expect("render");
    let dir = std::env::var_os("NOISE_SHOTS")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/shots"), PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    image.save(dir.join("factory_instruments.png")).unwrap();
}

#[test]
#[ignore = "draws pictures to look at; run with --ignored"]
fn shots() {
    factory_menu_shot();
    shot("pattern", |_| {});
    shot("mixer", |app| app.view = View::Mixer);
    shot_at("mixer_effects", [3000.0, 900.0], |app| {
        // Every section, the effects' strips shown too.
        app.ctx.data_mut(|d| d.insert_temp(eframe::egui::Id::new("mixer_effects"), true));
        app.view = View::Mixer;
    });
    shot("mixer_long_chain", |app| {
        // A chain longer than a strip lists, one of its effects switched off.
        let inst = app.project.modules.iter().find(|m| m.kind == ModuleKind::Sampler).expect("a Sampler").id;
        for _ in 0..4 {
            let at = app.project.chain(inst).effects.len();
            app.project.chain_insert(inst, at, ModuleKind::Eq);
        }
        let fx = app.project.chain(inst).effects[1];
        app.project.module_mut(fx).unwrap().bypass = true;
        app.view = View::Mixer;
    });
    shot("neighbours", |app| {
        // Near the top of Storm 2: Storm's end shows above.
        app.cursor.line = 3;
    });
    shot("automation", |app| {
        // An envelope of Build repeating every beat: the first, which the
        // panel shows first.
        app.slot = slot_of(app, "Build");
        let env = app.pattern_mut().automation.first_mut().expect("an envelope in Build");
        env.every = 1.0;
        (env.points, env.steps, env.curve) = super::automation::lfo_points(0, 4.0, 0.5);
        (app.lower, app.show_lower, app.lower_height) = (super::Lower::Automation, true, 300.0);
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
        app.select(Some(inst));
        app.view = View::Sampler;
        app.sampler.synth_tab = sampler::SynthTab::Synth;
    });
    shot_at("modulator", [1400.0, 1300.0], |app| {
        // A Modulator moving two parameters of an effect, selected, in the
        // chain of that effect.
        let p = &mut app.project;
        let fx = p.modules.iter().find(|m| m.kind == ModuleKind::Distortion).expect("a Distortion").id;
        let lfo = p.add_module(ModuleKind::Modulator, [0.0, 0.0]).unwrap();
        p.add_control(lfo, fx, 1);
        p.add_control(lfo, fx, 2);
        p.module_mut(lfo).unwrap().controls[1].depth = -0.6;
        app.select(Some(lfo));
        app.chain_view.reveal(lfo);
        app.show_chain(app.project.owner_of(fx).expect("the Distortion in a chain"));
        app.lower_height = 640.0;
    });
    shot("send_track", |app| {
        // The send track several instruments send to.
        let shared = app.project.sends[0].module;
        app.show_send(shared);
    });
    shot_at("sends", [2300.0, 900.0], |app| {
        // An instrument whose chain ends with a Send.
        let p = &app.project;
        let sends = |id: u8| p.chain(id).effects.iter().any(|&e| p.module(e).unwrap().kind == ModuleKind::Send);
        let inst = p.modules.iter().find(|m| m.kind.plays_sound() && sends(m.id)).expect("one that sends").id;
        app.select(Some(inst));
        app.view = View::Sampler;
        app.sampler.synth_tab = sampler::SynthTab::Synth;
    });
    shot("folded", |app| {
        // A synth's effects folded to their names.
        let p = &app.project;
        let synth = |m: &&crate::project::Module| m.kind.plays_sound() && !m.kind.holds_samples();
        let inst = p.modules.iter().find(|m| synth(m) && p.chain(m.id).effects.len() > 2);
        let inst = inst.expect("a synth with effects").id;
        let fx = app.project.chain(inst).effects.clone();
        app.chain_view.fold(&fx);
        app.select(Some(inst));
        app.view = View::Sampler;
        app.sampler.synth_tab = sampler::SynthTab::Synth;
    });
    shot("sampler", |app| {
        // A Sampler's waveform.
        let kit = app.project.modules.iter().find(|m| m.kind == ModuleKind::Sampler).expect("a Sampler").id;
        app.select(Some(kit));
        app.view = View::Sampler;
    });
    shot("keyzones", |app| {
        let kit = app.project.modules.iter().find(|m| m.kind == ModuleKind::Sampler).expect("a Sampler").id;
        app.select(Some(kit));
        app.view = View::Sampler;
        app.sampler.tab = sampler::Tab::Keyzones;
    });
    shot("track_fx", |app| {
        app.show_track_fx(0);
        app.lower_height = 270.0;
    });
    shot("new_song", |app| {
        // A song without instruments, which offers to start one.
        (app.project, app.slot, app.cursor) = (crate::project::Project::empty(), 0, Default::default());
        app.select(None);
        app.mark();
    });
    shot("palette", |app| {
        app.palette.open = true;
        app.palette.preset = Some("rev".into());
    });
    shot("scale", |app| {
        // C major over a song that isn't in it: its other notes show dull.
        app.project.scale = crate::project::Scale { key: 0, kind: 1 };
        app.chord = 1;
    });
}

/// The README's pictures, `docs/*.png`, drawn while the song plays
/// pattern `at`, from its top to the line given: this needs a sound card,
/// so point it at a silent one.
fn doc_shot(name: &str, at: (&str, usize), setup: impl FnOnce(&mut App)) {
    let mut harness =
        Harness::builder().with_size([1400.0, 900.0]).wgpu().build_eframe(|cc| App::start(cc, None, true));
    let app = harness.state_mut();
    open(app);
    app.scope_view = super::ScopeView::Tracks;
    (app.slot, app.cursor.line) = (slot_of(app, at.0), 0);
    setup(app);
    app.toggle_play();
    let until = (app.slot, at.1);
    let start = std::time::Instant::now();
    while harness.state().play_position() < until {
        assert!(start.elapsed().as_secs() < 60, "the song never got there");
        harness.step();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    harness.run_steps(2);
    let image = harness.render().expect("render");
    let dir = std::env::var_os("NOISE_SHOTS")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/shots/docs"), PathBuf::from);
    std::fs::create_dir_all(&dir).unwrap();
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore = "plays the demo songs on the sound card; run with --ignored"]
fn docs() {
    // Storm 2, the busiest pattern.
    doc_shot("main", ("Storm 2", 48), |_| {});
    doc_shot("mixer", ("Storm 2", 40), |app| app.view = View::Mixer);
    doc_shot("sampler", ("Drill", 56), |app| {
        // The break, chopped apart in Drill.
        let brk = app.project.modules.iter().find(|m| m.name == "Rain Break").expect("the break").id;
        app.select(Some(brk));
        app.view = View::Sampler;
    });
    doc_shot("automation", ("Build", 64), |app| {
        // Build, with five envelopes.
        (app.lower, app.show_lower, app.lower_height) = (super::Lower::Automation, true, 270.0);
        super::automation::pick(app, &[]);
    });
}
