//! Audio sample data loaded from WAV, FLAC and Ogg Vorbis files.

use crate::dsp::Frame;
use std::path::Path;

pub struct Sample {
    pub name: String,
    pub sample_rate: f32,
    /// Channels in the file. Frames are always stereo; mono files are copied
    /// to both channels.
    pub channels: u16,
    pub frames: Vec<Frame>,
}

impl std::fmt::Debug for Sample {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sample")
            .field("name", &self.name)
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("frames", &self.frames.len())
            .finish()
    }
}

/// The extensions of the audio files samples load from.
pub const EXTENSIONS: [&str; 4] = ["wav", "flac", "ogg", "oga"];

/// Whether `path` names an audio file samples load from.
pub fn is_audio(path: &Path) -> bool {
    path.extension().is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

impl Sample {
    pub fn load(path: &Path) -> Result<Sample, String> {
        let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        let name = path.file_stem().map_or("sample".into(), |s| s.to_string_lossy().into_owned());
        match ext.as_str() {
            "flac" => Self::load_flac(path, name),
            "ogg" | "oga" => {
                let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
                Self::from_vorbis(std::io::BufReader::new(file), name)
            }
            _ => Self::load_wav(path, name),
        }
    }

    /// A sample from interleaved audio, `channels` values a frame.
    pub fn from_interleaved(name: String, sample_rate: f32, channels: usize, data: &[f32]) -> Result<Sample, String> {
        let channels = channels.max(1);
        if data.len() < channels {
            return Err("the file contains no audio".into());
        }
        let frames =
            data.chunks_exact(channels).map(|c| if channels == 1 { [c[0], c[0]] } else { [c[0], c[1]] }).collect();
        Ok(Sample { name, sample_rate, channels: channels.min(2) as u16, frames })
    }

    fn load_flac(path: &Path, name: String) -> Result<Sample, String> {
        let mut reader = claxon::FlacReader::open(path).map_err(|e| e.to_string())?;
        let info = reader.streaminfo();
        let scale = 1.0 / (1i64 << (info.bits_per_sample.max(1) - 1)) as f32;
        let data: Vec<f32> = reader
            .samples()
            .map(|s| s.map(|s| s as f32 * scale))
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?;
        Self::from_interleaved(name, info.sample_rate as f32, info.channels as usize, &data)
    }

    /// Decodes an Ogg Vorbis stream, as files and SF3 soundfonts hold.
    pub fn from_vorbis<R: std::io::Read + std::io::Seek>(reader: R, name: String) -> Result<Sample, String> {
        let mut ogg = lewton::inside_ogg::OggStreamReader::new(reader).map_err(|e| e.to_string())?;
        let (channels, rate) = (ogg.ident_hdr.audio_channels as usize, ogg.ident_hdr.audio_sample_rate);
        let mut data = Vec::new();
        while let Some(packet) = ogg.read_dec_packet_itl().map_err(|e| e.to_string())? {
            data.extend(packet.into_iter().map(|s: i16| s as f32 / 32768.0));
        }
        // The last page's granule position says where the audio ends; the
        // packets run on past it.
        if let Some(end) = ogg.get_last_absgp() {
            data.truncate((end as usize).saturating_mul(channels.max(1)).min(data.len()).max(channels.max(1)));
        }
        Self::from_interleaved(name, rate as f32, channels, &data)
    }

    fn load_wav(path: &Path, name: String) -> Result<Sample, String> {
        let mut reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
        let spec = reader.spec();
        let channels = spec.channels.max(1) as usize;
        let data: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => {
                reader.samples::<f32>().collect::<Result<_, _>>().map_err(|e| e.to_string())?
            }
            hound::SampleFormat::Int => {
                let scale = 1.0 / (1i64 << (spec.bits_per_sample.max(1) - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|s| s as f32 * scale))
                    .collect::<Result<_, _>>()
                    .map_err(|e| e.to_string())?
            }
        };
        Self::from_interleaved(name, spec.sample_rate as f32, channels, &data)
    }

    /// A copy with the same name, rate and channels but other audio.
    pub fn with_frames(&self, frames: Vec<Frame>) -> Sample {
        Sample { name: self.name.clone(), sample_rate: self.sample_rate, channels: self.channels, frames }
    }

    /// Writes the sample as a 32-bit float WAV file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let file = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?);
        self.write_wav(file)
    }

    /// The sample as a 32-bit float WAV file's bytes.
    pub fn wav(&self) -> Result<Vec<u8>, String> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        self.write_wav(&mut bytes)?;
        Ok(bytes.into_inner())
    }

    fn write_wav<W: std::io::Write + std::io::Seek>(&self, out: W) -> Result<(), String> {
        let spec = hound::WavSpec {
            channels: self.channels.clamp(1, 2),
            sample_rate: self.sample_rate as u32,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::new(out, spec).map_err(|e| e.to_string())?;
        for f in &self.frames {
            for &v in &f[..spec.channels as usize] {
                w.write_sample(v).map_err(|e| e.to_string())?;
            }
        }
        w.finalize().map_err(|e| e.to_string())
    }

    /// A hash of the audio as `save` writes it (its rate, channels and
    /// frames, not its name), as 32 hex digits: FNV-1a of 128 bits, so it
    /// is the same on every run and build. Samples that would save to the
    /// same file get the same hash.
    pub fn hash(&self) -> String {
        const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013B;
        let mut h: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
        let mut feed = |bytes: &[u8]| {
            for &b in bytes {
                h = (h ^ b as u128).wrapping_mul(PRIME);
            }
        };
        let channels = self.channels.clamp(1, 2);
        feed(&(self.sample_rate as u32).to_le_bytes());
        feed(&channels.to_le_bytes());
        for f in &self.frames {
            for v in &f[..channels as usize] {
                feed(&v.to_le_bytes());
            }
        }
        format!("{h:032x}")
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn seconds(&self) -> f32 {
        self.len() as f32 / self.sample_rate
    }
}

/// `frames` made `ratio` times as long without changing their pitch, by
/// waveform-similarity overlap-add: windows of 50 ms laid down at the new
/// spacing, each taken from near where it would be in time, moved by up
/// to a quarter window to where it best continues the one before, so the
/// waveforms line up and nothing beats or flutters.
pub fn time_stretch(frames: &[Frame], ratio: f64, sample_rate: f32) -> Vec<Frame> {
    let out_len = (frames.len() as f64 * ratio).round() as usize;
    let n = ((sample_rate * 0.05) as usize / 2 * 2).max(64);
    if frames.len() < 2 * n || ratio <= 0.0 {
        // Too short to stretch: as it was, cut or padded to the length.
        let mut out = frames.to_vec();
        out.resize(out_len, [0.0; 2]);
        return out;
    }
    let half = n / 2;
    let hop_in = half as f64 / ratio;
    let tolerance = n / 4;
    let window: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos()).collect();
    let mono: Vec<f32> = frames.iter().map(|f| f[0] + f[1]).collect();
    let last = frames.len() - n;
    let mut out = vec![[0.0f32; 2]; out_len + n];
    let mut weight = vec![0.0f32; out_len + n];
    // How well the input at `pos` matches `target`, over every `step`th
    // frame.
    let score = |target: usize, pos: usize, step: usize| -> f32 {
        (0..n).step_by(step).map(|k| mono[target + k] * mono[pos + k]).sum()
    };
    let mut prev = 0usize;
    let mut k = 0;
    while k * half < out_len {
        let nominal = ((k as f64 * hop_in) as usize).min(last);
        let pos = if k == 0 {
            0
        } else {
            // Where the last window would have gone on to.
            let target = (prev + half).min(last);
            let (lo, hi) = (nominal.saturating_sub(tolerance), (nominal + tolerance).min(last));
            // Coarse, then fine around the best.
            let mut best = (nominal, f32::MIN);
            for pos in (lo..=hi).step_by(4) {
                let s = score(target, pos, 4);
                if s > best.1 {
                    best = (pos, s);
                }
            }
            let (lo, hi) = (best.0.saturating_sub(4).max(lo), (best.0 + 4).min(hi));
            best.1 = f32::MIN;
            for pos in lo..=hi {
                let s = score(target, pos, 1);
                if s > best.1 {
                    best = (pos, s);
                }
            }
            best.0
        };
        let at = k * half;
        for j in 0..n {
            let w = window[j];
            out[at + j][0] += frames[pos + j][0] * w;
            out[at + j][1] += frames[pos + j][1] * w;
            weight[at + j] += w;
        }
        prev = pos;
        k += 1;
    }
    out.truncate(out_len);
    for (f, w) in out.iter_mut().zip(&weight) {
        if *w > 1e-3 {
            *f = [f[0] / w, f[1] / w];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stretching_keeps_the_pitch() {
        let sr = 44100.0;
        let tone: Vec<Frame> =
            (0..sr as usize).map(|i| [(std::f32::consts::TAU * 440.0 * i as f32 / sr).sin() * 0.5; 2]).collect();
        let crossings = |x: &[Frame]| x.windows(2).filter(|w| w[0][0] < 0.0 && w[1][0] >= 0.0).count() as f32;
        for ratio in [0.5, 0.8, 1.5, 2.0] {
            let out = time_stretch(&tone, ratio, sr);
            assert_eq!(out.len(), (sr as f64 * ratio).round() as usize);
            // As many cycles a second as before, and about as loud.
            let hz = crossings(&out) / (out.len() as f32 / sr);
            assert!((hz - 440.0).abs() < 6.0, "{ratio}: {hz} Hz");
            let peak = out[2000..out.len() - 2000].iter().fold(0f32, |m, f| m.max(f[0].abs()));
            assert!((0.4..0.55).contains(&peak), "{ratio}: peak {peak}");
        }
    }

    #[test]
    fn flac_and_ogg_load() {
        for name in ["sine.flac", "sine.ogg"] {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(name);
            assert!(is_audio(&path));
            let s = Sample::load(&path).unwrap();
            assert_eq!((s.name.as_str(), s.sample_rate, s.channels), ("sine", 22050.0, 1));
            assert!((s.len() as i64 - 5512).abs() < 64, "{name}: {} frames", s.len());
            let peak = s.frames.iter().map(|f| f[0].abs()).fold(0.0, f32::max);
            assert!(peak > 0.1 && peak <= 1.0, "{name}: peak {peak}");
        }
    }
}
