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

use clap::Parser;

/// A music tracker with modular synths.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// A project folder, song file or exported .noise project to open
    #[arg(value_name = "SONG")]
    song: Option<String>,
    /// Render a song to WAV without opening a window
    #[arg(long, num_args = 2, value_names = ["SONG", "OUT.WAV"], conflicts_with = "song")]
    export: Option<Vec<String>>,
    /// Render the demo song Last Light to WAV without opening a window
    #[arg(long, value_name = "OUT.WAV", num_args = 0..=1, default_missing_value = "demo.wav", conflicts_with_all = ["song", "export"])]
    demo_wav: Option<String>,
}

fn main() -> eframe::Result {
    let args = Args::parse();
    // The wavetables take a moment to make: before any sound, not during.
    dsp::wavetable::warm_up();

    let render = match (args.export, args.demo_wav) {
        (Some(export), _) => {
            let [input, output] = <[String; 2]>::try_from(export).expect("clap takes two");
            let song = project_dir::open(std::path::Path::new(&input)).map_err(|e| format!("{input}: {e}"));
            Some((song, output))
        }
        (_, Some(output)) => Some((Ok((project::Project::demo(), Vec::new())), output)),
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

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1400.0, 900.0]).with_title("noise"),
        ..Default::default()
    };
    eframe::run_native("noise", options, Box::new(|cc| Ok(Box::new(ui::App::new(cc, args.song)))))
}
