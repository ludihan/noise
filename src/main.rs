mod audio;
mod clockwork_rain;
mod concrete_hymn;
mod demo;
mod dsp;
mod engine;
mod midi;
mod paths;
mod prism_overdrive;
mod project;
mod project_dir;
mod sample;
mod soundfont;
mod static_heart;
mod ui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The wavetables take a moment to make: before any sound, not during.
    dsp::wavetable::warm_up();

    // `noise --export <project folder or song file> out.wav` renders without
    // opening a window.
    if let [flag, input, output] = args.as_slice()
        && flag == "--export"
    {
        let (p, warnings) = project_dir::open(std::path::Path::new(input)).expect("read project");
        for w in warnings {
            eprintln!("warning: {w}");
        }
        audio::export_wav(std::sync::Arc::new(p), output, audio::RenderFormat::CD, &mut |_| true).expect("write wav");
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("--demo-wav") {
        let out = args.get(1).map_or("demo.wav", String::as_str);
        audio::export_wav(std::sync::Arc::new(project::Project::demo()), out, audio::RenderFormat::CD, &mut |_| true)
            .expect("write wav");
        return Ok(());
    }

    let path = args.into_iter().next();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1400.0, 900.0]).with_title("noise"),
        ..Default::default()
    };
    eframe::run_native("noise", options, Box::new(|cc| Ok(Box::new(ui::App::new(cc, path)))))
}
