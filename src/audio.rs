//! Device output and input via cpal, and offline rendering to WAV.

use crate::dsp::Frame;
use crate::engine::{Cmd, Engine, Garbage, Shared, Tape};
use crate::project::Project;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, Stream, StreamConfig};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};

pub struct Audio {
    _stream: Stream,
    pub sample_rate: u32,
}

pub fn start(
    project: Arc<Project>,
    rx: Receiver<Cmd>,
    garbage: Sender<Garbage>,
    shared: Arc<Shared>,
) -> Result<Audio, String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let sr = config.sample_rate;
    let engine = Engine::new(sr as f32, project, Some(rx), Some(garbage), shared);

    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, engine),
        SampleFormat::I16 => build::<i16>(&device, config, engine),
        SampleFormat::U16 => build::<u16>(&device, config, engine),
        SampleFormat::I32 => build::<i32>(&device, config, engine),
        f => return Err(format!("unsupported sample format {f}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Audio { _stream: stream, sample_rate: sr })
}

fn build<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: StreamConfig,
    mut engine: Engine,
) -> Result<Stream, String> {
    let channels = config.channels as usize;
    let mut buf: Vec<Frame> = Vec::with_capacity(8192);
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let frames = data.len() / channels;
                buf.resize(frames, [0.0; 2]);
                engine.render(&mut buf);
                for (out, f) in data.chunks_mut(channels).zip(&buf) {
                    for (ch, s) in out.iter_mut().enumerate() {
                        let v = if ch == 0 {
                            f[0]
                        } else if ch == 1 {
                            f[1]
                        } else {
                            0.0
                        };
                        *s = T::from_sample(v.clamp(-1.0, 1.0));
                    }
                }
            },
            |err| eprintln!("audio error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}

/// The sound device's input, open while the sample recorder records it.
pub struct Input {
    _stream: Stream,
    pub sample_rate: u32,
    /// The device's name, to show.
    pub name: String,
}

/// Opens the default input device, sending what it hears to `tape`.
pub fn open_input(tape: Arc<Tape>) -> Result<Input, String> {
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("no input device")?;
    let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "Input".into());
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let sr = config.sample_rate;
    let stream = match format {
        SampleFormat::F32 => build_input::<f32>(&device, config, tape),
        SampleFormat::I16 => build_input::<i16>(&device, config, tape),
        SampleFormat::U16 => build_input::<u16>(&device, config, tape),
        SampleFormat::I32 => build_input::<i32>(&device, config, tape),
        f => return Err(format!("unsupported sample format {f}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Input { _stream: stream, sample_rate: sr, name })
}

fn build_input<T: SizedSample>(device: &cpal::Device, config: StreamConfig, tape: Arc<Tape>) -> Result<Stream, String>
where
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let mut buf: Vec<Frame> = Vec::with_capacity(8192);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                // A mono input goes to both sides.
                buf.clear();
                buf.extend(data.chunks(channels).map(|f| {
                    let l = <f32 as FromSample<T>>::from_sample_(f[0]);
                    [l, f.get(1).map_or(l, |&r| <f32 as FromSample<T>>::from_sample_(r))]
                }));
                tape.push(&buf);
            },
            |err| eprintln!("audio input error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}

/// Plays the song once, offline, and then up to `tail` seconds more for
/// notes and effects to die away. With `trim`, silence at the end of the
/// tail is dropped.
#[cfg(test)]
pub fn render(project: Arc<Project>, sr: u32, tail: f32, trim: bool) -> Vec<Frame> {
    render_with(project, sr, tail, trim, &mut |_| true).unwrap_or_default()
}

/// `render`, telling `going` how far through the song it is (0..1) as it
/// goes; `None` if `going` says to stop.
pub fn render_with(
    project: Arc<Project>,
    sr: u32,
    tail: f32,
    trim: bool,
    going: &mut dyn FnMut(f32) -> bool,
) -> Option<Vec<Frame>> {
    let mut engine = Engine::new(sr as f32, project, None, None, Arc::new(Shared::default()));
    engine.play_song();
    let mut samples: Vec<Frame> = Vec::new();
    let mut block = vec![[0.0; 2]; 512];
    // Ten minutes is plenty for a pattern-based song; stop runaway renders.
    let limit = sr as usize * 600;
    while !engine.song_ended && samples.len() < limit {
        engine.render(&mut block);
        samples.extend_from_slice(&block);
        // About ten times a second of audio.
        if samples.len().is_multiple_of(block.len() * 8) && !going(engine.song_progress()) {
            return None;
        }
    }
    let played = samples.len();
    engine.handle(Cmd::Stop);
    for _ in 0..(sr as f32 * tail / block.len() as f32) as usize {
        engine.render(&mut block);
        samples.extend_from_slice(&block);
    }
    if trim {
        // Below -80 dB counts as silence.
        let last = samples.iter().rposition(|f| f[0].abs().max(f[1].abs()) > 1e-4).map_or(0, |i| i + 1);
        samples.truncate(last.max(played));
    }
    Some(samples)
}

/// How a render is written: its sample rate and sample format.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderFormat {
    pub sample_rate: u32,
    pub depth: BitDepth,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BitDepth {
    Int16,
    #[default]
    Int24,
    Float32,
}

impl BitDepth {
    pub const ALL: [BitDepth; 3] = [BitDepth::Int16, BitDepth::Int24, BitDepth::Float32];

    pub fn name(self) -> &'static str {
        match self {
            BitDepth::Int16 => "16 bit",
            BitDepth::Int24 => "24 bit",
            BitDepth::Float32 => "32 bit float",
        }
    }
}

impl RenderFormat {
    /// CD quality: 44.1 kHz, 16 bit.
    pub const CD: RenderFormat = RenderFormat { sample_rate: 44100, depth: BitDepth::Int16 };
}

/// Renders the song to the WAV file at `path`, telling `going` how far it
/// has got; `Ok(false)` if `going` stopped it, with nothing written.
pub fn export_wav(
    project: Arc<Project>,
    path: &str,
    format: RenderFormat,
    going: &mut dyn FnMut(f32) -> bool,
) -> Result<bool, String> {
    let Some(frames) = render_with(project, format.sample_rate, 2.0, false, going) else { return Ok(false) };
    write_wav(path, &frames, format).map(|()| true)
}

/// Renders each instrument of the song to its own file: the instrument
/// soloed, with its own effects and its share of the effects it feeds.
/// The files are named after `path`, with the instrument's number and
/// name, and are all as long as the song, so they line up. Returns the
/// files written, as far as it got before `going` stopped it.
pub fn export_stems(
    project: &Project,
    path: &str,
    format: RenderFormat,
    going: &mut dyn FnMut(f32) -> bool,
) -> Result<Vec<String>, String> {
    let base = path.strip_suffix(".wav").unwrap_or(path);
    let instruments: Vec<(u8, String)> =
        project.modules.iter().filter(|m| m.kind.plays_sound() && !m.mute).map(|m| (m.id, m.name.clone())).collect();
    let mut written = Vec::new();
    let count = instruments.len().max(1) as f32;
    for (k, (id, name)) in instruments.into_iter().enumerate() {
        let mut stem = project.clone();
        for m in &mut stem.modules {
            m.solo = m.id == id;
        }
        let name: String =
            name.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect();
        let file = format!("{base} {id:02X} {name}.wav");
        // Each stem is its share of the whole.
        let mut part = |f: f32| going((k as f32 + f) / count);
        let Some(frames) = render_with(Arc::new(stem), format.sample_rate, 2.0, false, &mut part) else { break };
        write_wav(&file, &frames, format)?;
        written.push(file);
    }
    Ok(written)
}

/// Writes `samples` as a stereo WAV file in `format`. Integer formats are
/// dithered (TPDF), so quiet tails fade out rather than break up.
fn write_wav(path: &str, samples: &[Frame], format: RenderFormat) -> Result<(), String> {
    let (bits, sample_format) = match format.depth {
        BitDepth::Int16 => (16, hound::SampleFormat::Int),
        BitDepth::Int24 => (24, hound::SampleFormat::Int),
        BitDepth::Float32 => (32, hound::SampleFormat::Float),
    };
    let spec = hound::WavSpec { channels: 2, sample_rate: format.sample_rate, bits_per_sample: bits, sample_format };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| e.to_string())?;
    let full = ((1i64 << (bits - 1)) - 1) as f32;
    let mut rng = crate::rng::Rng(0x2545_f491);
    let mut noise = move || rng.unit() - 0.5;
    for s in samples {
        for &ch in s {
            let ch = ch.clamp(-1.0, 1.0);
            let r = match format.depth {
                BitDepth::Float32 => w.write_sample(ch),
                _ => {
                    let x = (ch * full + noise() + noise()).round().clamp(-full - 1.0, full) as i32;
                    w.write_sample(x)
                }
            };
            r.map_err(|e| e.to_string())?;
        }
    }
    w.finalize().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_say_how_far_they_are_and_can_be_stopped() {
        let mut seen = Vec::new();
        let frames = render_with(Arc::new(Project::demo()), 4000, 0.5, false, &mut |f| {
            seen.push(f);
            true
        });
        assert!(frames.is_some());
        assert!(seen.windows(2).all(|w| w[1] >= w[0]), "it only goes forward");
        assert!(seen.last().unwrap() > &0.95, "and gets to the end: {:?}", seen.last());
        // Stopped part way, it writes nothing.
        let dir = std::env::temp_dir().join(format!("noise-cancel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("song.wav");
        let done = export_wav(Arc::new(Project::demo()), path.to_str().unwrap(), RenderFormat::CD, &mut |f| f < 0.2);
        assert_eq!(done, Ok(false));
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_demo_plays_without_clipping() {
        let frames = render(Arc::new(Project::demo()), 8000, 1.0, false);
        let peak = frames.iter().fold(0f32, |a, f| a.max(f[0].abs()).max(f[1].abs()));
        assert!(peak > 0.3 && peak < 0.95, "{peak}");
        assert!(frames.len() > 8000 * 40, "the whole song: {}", frames.len());
    }

    #[test]
    fn renders_are_written_in_the_format_asked_for() {
        let dir = std::env::temp_dir().join(format!("noise-render-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let frames: Vec<Frame> = (0..100).map(|i| [i as f32 / 100.0, -0.5]).collect();
        for depth in BitDepth::ALL {
            let path = dir.join(format!("{depth:?}.wav"));
            let path = path.to_str().unwrap();
            write_wav(path, &frames, RenderFormat { sample_rate: 48000, depth }).unwrap();
            let spec = hound::WavReader::open(path).unwrap().spec();
            assert_eq!(spec.sample_rate, 48000);
            let back = crate::sample::Sample::load(std::path::Path::new(path)).unwrap();
            assert_eq!(back.len(), 100);
            let err = back.frames.iter().zip(&frames).fold(0f32, |m, (a, b)| m.max((a[0] - b[0]).abs()));
            assert!(err < 1e-3, "{depth:?}: {err}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn stems_hold_one_instrument_each() {
        use crate::project::{Cell, ModuleKind, Note, OUTPUT_ID};
        let mut p = Project::empty();
        let (a, b) =
            (p.add_module(ModuleKind::Generator, [0.0; 2]).unwrap(), p.add_module(ModuleKind::Fm, [0.0; 2]).unwrap());
        p.connect(a, OUTPUT_ID);
        p.connect(b, OUTPUT_ID);
        // The Generator plays the first half of the pattern, the FM the second.
        p.patterns[0].tracks[0][0] = Cell { note: Some(Note::On(48)), module: Some(a), ..Cell::default() };
        p.patterns[0].tracks[0][32] = Cell { note: Some(Note::On(48)), module: Some(b), ..Cell::default() };
        let dir = std::env::temp_dir().join(format!("noise-stems-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sr = 8000;
        let format = RenderFormat { sample_rate: sr, depth: BitDepth::Int16 };
        let files = export_stems(&p, dir.join("song.wav").to_str().unwrap(), format, &mut |_| true).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files[0].ends_with(&format!("song {a:02X} Generator.wav")), "{}", files[0]);
        let read = |f: &str| crate::sample::Sample::load(std::path::Path::new(f)).unwrap();
        let (sa, sb) = (read(&files[0]), read(&files[1]));
        assert_eq!(sa.len(), sb.len(), "they line up");
        let half = sa.len() / 3;
        let loud = |s: &crate::sample::Sample, r: std::ops::Range<usize>| s.frames[r].iter().any(|f| f[0].abs() > 0.05);
        assert!(loud(&sa, 0..half) && !loud(&sb, 0..half), "only the Generator at first");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn selections_render_with_their_tail_trimmed() {
        let demo = Project::demo();
        // The first four lines of the kick track: one kick.
        let song = Arc::new(demo.excerpt(1, (0, 3), (0, 0)));
        let sr = 8000;
        let frames = render(song, sr, 4.0, true);
        let four_lines = (sr as f32 * 60.0 / (demo.bpm * demo.lpb as f32)) as usize;
        assert!(frames.len() >= four_lines, "at least the lines: {}", frames.len());
        assert!(frames.len() < four_lines + 2 * sr as usize, "the silent tail is trimmed: {}", frames.len());
        assert!(frames[..four_lines].iter().any(|f| f[0].abs() > 0.1), "the kick is there");
    }
}
