mod audio;
mod demos;
mod dsp;
mod engine;
mod midi;
mod paths;
mod project;
mod project_dir;
mod rng;
mod sample;
mod soundfont;
mod ui;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The wavetables take a moment to make: before any sound, not during.
    dsp::wavetable::warm_up();

    // `noise --export <project folder or song file> out.wav` renders without
    // opening a window, as `--demo-wav [out.wav]` does the demo song.
    let demo = || Ok((project::Project::demo(), Vec::new()));
    let render = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["--export", input, output] => Some((
            project_dir::open(std::path::Path::new(input)).map_err(|e| format!("{input}: {e}")),
            output.to_string(),
        )),
        ["--demo-wav"] => Some((demo(), "demo.wav".to_string())),
        ["--demo-wav", output] => Some((demo(), output.to_string())),
        [flag, ..] if flag.starts_with("--") => {
            eprintln!("usage: noise [song]\n       noise --export <song> <out.wav>\n       noise --demo-wav [out.wav]");
            std::process::exit(2);
        }
        _ => None,
    };
    if let Some((song, output)) = render {
        let rendered = song.and_then(|(p, warnings)| {
            for w in warnings {
                eprintln!("warning: {w}");
            }
            audio::export_wav(std::sync::Arc::new(p), &output, audio::RenderFormat::CD, &mut |_| true)
                .map_err(|e| format!("{output}: {e}"))
        });
        match rendered {
            Ok(audio::Rendered::TooLong) => {
                eprintln!("warning: the song was cut off after {} hours", audio::MAX_RENDER_SECONDS / 3600)
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!("noise: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }

    let path = args.into_iter().next();
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1400.0, 900.0]).with_title("noise"),
        ..Default::default()
    };
    eframe::run_native("noise", options, Box::new(|cc| Ok(Box::new(ui::App::new(cc, path)))))
}
