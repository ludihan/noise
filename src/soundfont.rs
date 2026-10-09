//! Soundfonts loaded into a Sampler as its sample slots: SF2 and SF3
//! through the `soundfont` crate, with SF3's Ogg Vorbis
//! samples decoded by `lewton`, and SFZ through `xsynth-soundfonts`.
//!
//! What a Sampler can play comes along: the samples with their key and
//! velocity ranges, root key, tuning, volume, panning, loop and exclusive
//! class (as a mute group), and the volume envelope of the first zone as
//! the Sampler's ADSR. Modulators, filters and LFOs are left out.

use crate::project::SampleSlot;
use crate::sample::Sample;
use soundfont::raw::{GeneratorAmount, GeneratorType, SampleLink};
use soundfont::{SoundFont2, Zone};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Arc;

/// The extensions of the soundfonts that load into a Sampler.
pub const EXTENSIONS: [&str; 3] = ["sf2", "sf3", "sfz"];

/// Whether `path` names a soundfont.
pub fn is_soundfont(path: &Path) -> bool {
    path.extension().is_some_and(|e| EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Whether `path` is an SFZ file, which holds one instrument; SF2 and SF3
/// files hold presets to pick from.
pub fn is_sfz(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("sfz"))
}

/// What a soundfont's instrument becomes in a Sampler.
pub struct Instrument {
    pub name: String,
    pub slots: Vec<SampleSlot>,
    /// Attack, decay, sustain and release, as the Sampler's parameters.
    pub envelope: Option<[f32; 4]>,
}

/// A preset of an SF2 or SF3 file, in bank and program order.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub bank: u16,
    pub program: u16,
    pub name: String,
}

/// Loads the instrument of an SFZ file, or preset `preset` (an index into
/// `presets`) of an SF2 or SF3 file.
pub fn load(path: &Path, preset: usize) -> Result<Instrument, String> {
    if is_sfz(path) { load_sfz(path) } else { load_sf2(path, preset) }
}

/// The presets of an SF2 or SF3 file.
pub fn presets(path: &Path) -> Result<Vec<Preset>, String> {
    let (sf, _) = open_sf2(path)?;
    Ok(sf
        .presets
        .iter()
        .map(|p| Preset { bank: p.header.bank, program: p.header.preset, name: p.header.name.clone() })
        .collect())
}

fn open_sf2(path: &Path) -> Result<(SoundFont2, File), String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    // The parser asserts on some malformed files rather than failing.
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| SoundFont2::load(&mut file)))
        .map_err(|_| "not a valid soundfont".to_string())?
        .map_err(|e| format!("not a valid soundfont ({e:?})"))?;
    Ok((parsed.sort_presets(), file))
}

// ---------------------------------------------------------------- SF2 / SF3

/// A zone's generators: its key and velocity ranges, the instrument or
/// sample it points to, and the other amounts by generator number.
#[derive(Clone, Default)]
struct Gens {
    keys: Option<(u8, u8)>,
    vels: Option<(u8, u8)>,
    link: Option<usize>,
    amounts: HashMap<u16, i32>,
}

impl Gens {
    fn of(zone: &Zone) -> Gens {
        let mut g = Gens::default();
        for item in &zone.gen_list {
            let Ok(ty) = item.ty.into_result() else { continue };
            match (ty, &item.amount) {
                (GeneratorType::KeyRange, GeneratorAmount::Range(r)) => g.keys = Some((r.low, r.high)),
                (GeneratorType::VelRange, GeneratorAmount::Range(r)) => g.vels = Some((r.low, r.high)),
                (GeneratorType::Instrument | GeneratorType::SampleID, GeneratorAmount::U16(v)) => {
                    g.link = Some(*v as usize)
                }
                (_, GeneratorAmount::I16(v)) => {
                    g.amounts.insert(ty as u16, *v as i32);
                }
                (_, GeneratorAmount::U16(v)) => {
                    g.amounts.insert(ty as u16, *v as i32);
                }
                _ => {}
            }
        }
        g
    }

    /// This zone's generators over its list's global zone's.
    fn over(mut self, global: &Gens) -> Gens {
        self.keys = self.keys.or(global.keys);
        self.vels = self.vels.or(global.vels);
        for (k, v) in &global.amounts {
            self.amounts.entry(*k).or_insert(*v);
        }
        self
    }

    fn get(&self, ty: GeneratorType) -> Option<i32> {
        self.amounts.get(&(ty as u16)).copied()
    }
}

/// A list's zones, and its global zone: a first zone pointing nowhere.
fn split(zones: &[Zone]) -> (Gens, Vec<Gens>) {
    let all: Vec<Gens> = zones.iter().map(Gens::of).collect();
    match all.first() {
        Some(first) if first.link.is_none() => {
            (first.clone(), all[1..].iter().filter(|g| g.link.is_some()).cloned().collect())
        }
        _ => (Gens::default(), all.into_iter().filter(|g| g.link.is_some()).collect()),
    }
}

fn intersect(a: Option<(u8, u8)>, b: Option<(u8, u8)>) -> Option<(u8, u8)> {
    let (a, b) = (a.unwrap_or((0, 127)), b.unwrap_or((0, 127)));
    let r = (a.0.max(b.0), a.1.min(b.1));
    (r.0 <= r.1).then_some(r)
}

/// Seconds from SF2 timecents.
fn seconds(timecents: i32) -> f32 {
    2f32.powf(timecents as f32 / 1200.0)
}

/// A gain from centibels of attenuation.
fn attenuation(cb: i32) -> f32 {
    10f32.powf(-(cb.max(0) as f32) / 200.0)
}

/// A MIDI note as the tracker's: the same number, as the engine plays
/// notes at their MIDI pitch (the tracker calls MIDI 48 C-4), up to
/// B-9.
fn note(midi: i32) -> u8 {
    midi.clamp(0, 119) as u8
}

/// One sample zone of the preset, with everything summed up.
struct Region {
    sample: usize,
    keys: (u8, u8),
    vels: (u8, u8),
    inst: Gens,
    preset: Gens,
}

impl Region {
    /// An instrument level amount, plus the preset's, which adds to it.
    fn sum(&self, ty: GeneratorType, default: i32) -> i32 {
        self.inst.get(ty).unwrap_or(default) + self.preset.get(ty).unwrap_or(0)
    }

    /// An amount only instruments set, as the sample's addresses are.
    fn own(&self, ty: GeneratorType, default: i32) -> i32 {
        self.inst.get(ty).unwrap_or(default)
    }
}

/// The audio of an SF2 or SF3 file and the samples decoded from it.
struct Samples<'a> {
    headers: &'a [soundfont::raw::SampleHeader],
    smpl: Vec<u8>,
    sm24: Option<Vec<u8>>,
    decoded: HashMap<usize, Arc<Sample>>,
}

fn is_vorbis(link: SampleLink) -> bool {
    matches!(
        link,
        SampleLink::VorbisMonoSample
            | SampleLink::VorbisRightSample
            | SampleLink::VorbisLeftSample
            | SampleLink::VorbisLinkedSample
    )
}

impl Samples<'_> {
    /// Sample `i`, decoded once, as mono.
    fn get(&mut self, i: usize) -> Result<Arc<Sample>, String> {
        if let Some(s) = self.decoded.get(&i) {
            return Ok(s.clone());
        }
        let h = self.headers.get(i).ok_or("a zone points to a missing sample")?;
        let (start, end) = (h.start as usize, h.end as usize);
        let sample = if is_vorbis(h.sample_type) {
            // SF3: the addresses are bytes of the sample's Ogg stream.
            let bytes = self.smpl.get(start..end).ok_or_else(|| format!("sample {} is out of the file", h.name))?;
            let mut s = Sample::from_vorbis(Cursor::new(bytes), h.name.clone())?;
            s.sample_rate = h.sample_rate as f32;
            s
        } else {
            let words =
                self.smpl.get(2 * start..2 * end).ok_or_else(|| format!("sample {} is out of the file", h.name))?;
            let low = self.sm24.as_ref().and_then(|b| b.get(start..end));
            let data: Vec<f32> = words
                .as_chunks::<2>()
                .0
                .iter()
                .enumerate()
                .map(|(k, w)| match low {
                    Some(low) => i32::from_le_bytes([0, low[k], w[0], w[1]]) as f32 / 2147483648.0,
                    None => i16::from_le_bytes([w[0], w[1]]) as f32 / 32768.0,
                })
                .collect();
            Sample::from_interleaved(h.name.clone(), h.sample_rate as f32, 1, &data)?
        };
        let sample = Arc::new(sample);
        self.decoded.insert(i, sample.clone());
        Ok(sample)
    }

    /// The loop of sample `i` in its own frames.
    fn sample_loop(&self, i: usize) -> (i64, i64) {
        let h = &self.headers[i];
        if is_vorbis(h.sample_type) {
            // SF3 stores loops from the sample's start.
            (h.loop_start as i64, h.loop_end as i64)
        } else {
            (h.loop_start as i64 - h.start as i64, h.loop_end as i64 - h.start as i64)
        }
    }
}

fn read_chunk(file: &mut File, chunk: soundfont::raw::SampleChunk) -> Result<Vec<u8>, String> {
    let mut buf = vec![0; chunk.len as usize];
    file.seek(SeekFrom::Start(chunk.offset)).map_err(|e| e.to_string())?;
    file.read_exact(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn load_sf2(path: &Path, preset: usize) -> Result<Instrument, String> {
    use GeneratorType as G;
    let (sf, mut file) = open_sf2(path)?;
    let p = sf.presets.get(preset).ok_or("the soundfont has no such preset")?;
    let smpl = read_chunk(&mut file, sf.sample_data.smpl.ok_or("the soundfont has no samples")?)?;
    // 24-bit audio only counts when it has a byte for every sample.
    let sm24 = match sf.sample_data.sm24 {
        Some(c) if c.len as usize >= smpl.len() / 2 => Some(read_chunk(&mut file, c)?),
        _ => None,
    };

    let mut regions = Vec::new();
    let (pglobal, pzones) = split(&p.zones);
    for pz in pzones {
        let pg = pz.over(&pglobal);
        let Some(inst) = pg.link.and_then(|i| sf.instruments.get(i)) else { continue };
        let (iglobal, izones) = split(&inst.zones);
        for iz in izones {
            let ig = iz.over(&iglobal);
            let (Some(keys), Some(vels)) = (intersect(ig.keys, pg.keys), intersect(ig.vels, pg.vels)) else { continue };
            let Some(sample) = ig.link.filter(|&s| s < sf.sample_headers.len()) else { continue };
            regions.push(Region { sample, keys, vels, inst: ig.clone(), preset: pg.clone() });
        }
    }
    if regions.is_empty() {
        return Err(match p.header.name.trim() {
            "" => "the preset has no samples".into(),
            name => format!("the preset {name} has no samples"),
        });
    }

    let mut samples = Samples { headers: &sf.sample_headers, smpl, sm24, decoded: HashMap::new() };
    // A stereo pair is two zones, left and right, over the same notes: they
    // become one stereo sample.
    let partner = |r: &Region, side: fn(SampleLink) -> bool| {
        let h = &sf.sample_headers[r.sample];
        let link = h.sample_link as usize;
        (side(h.sample_type) && link != r.sample)
            .then(|| regions.iter().find(|o| o.sample == link && o.keys == r.keys && o.vels == r.vels))
            .flatten()
    };
    let mut slots = Vec::new();
    for r in &regions {
        let right_of_pair = partner(r, |t| t.is_right()).is_some();
        if right_of_pair {
            continue;
        }
        let left = samples.get(r.sample)?;
        let data = match partner(r, |t| !t.is_right() && !t.is_mono()) {
            Some(right) => {
                let right = samples.get(right.sample)?;
                let frames = left.frames.iter().zip(&right.frames).map(|(l, r)| [l[0], r[0]]).collect();
                Arc::new(Sample { name: left.name.clone(), sample_rate: left.sample_rate, channels: 2, frames })
            }
            None => left,
        };
        let stereo = data.channels == 2;

        // Address offsets move the sample's start, end and loop.
        let start = r.own(G::StartAddrsOffset, 0) as i64 + 32768 * r.own(G::StartAddrsCoarseOffset, 0) as i64;
        let end = r.own(G::EndAddrsOffset, 0) as i64 + 32768 * r.own(G::EndAddrsCoarseOffset, 0) as i64;
        let len = data.len() as i64;
        let (from, to) = (start.clamp(0, len), (len + end).clamp(0, len));
        if from >= to {
            continue;
        }
        let data = if (from, to) == (0, len) {
            data
        } else {
            Arc::new(data.with_frames(data.frames[from as usize..to as usize].to_vec()))
        };
        let (ls, le) = samples.sample_loop(r.sample);
        let ls = ls + r.own(G::StartloopAddrsOffset, 0) as i64 + 32768 * r.own(G::StartloopAddrsCoarseOffset, 0) as i64
            - from;
        let le =
            le + r.own(G::EndloopAddrsOffset, 0) as i64 + 32768 * r.own(G::EndloopAddrsCoarseOffset, 0) as i64 - from;
        let n = data.len() as i64;
        let (ls, le) = (ls.clamp(0, n), le.clamp(0, n));
        let looped = r.own(G::SampleModes, 0) & 1 == 1 && ls < le;

        let h = &sf.sample_headers[r.sample];
        let root = match r.own(G::OverridingRootKey, -1) {
            k if (0..=127).contains(&k) => k,
            _ if h.origpitch <= 127 => h.origpitch as i32,
            _ => 60,
        };
        let class = r.own(G::ExclusiveClass, 0);
        let mut slot = SampleSlot { name: h.name.clone(), data: Some(data), unsaved: true, ..SampleSlot::default() };
        slot.keys = [note(r.keys.0 as i32), note(r.keys.1 as i32)];
        slot.velocities = [r.vels.0, r.vels.1];
        slot.base_note = note(root);
        slot.transpose = r.sum(G::CoarseTune, 0);
        slot.finetune = r.sum(G::FineTune, 0) + h.pitchadj as i32;
        slot.volume = attenuation(r.sum(G::InitialAttenuation, 0)).min(4.0);
        slot.panning = if stereo { 0.0 } else { (r.sum(G::Pan, 0) as f32 / 500.0).clamp(-1.0, 1.0) };
        slot.loop_mode = looped as u8;
        (slot.loop_start, slot.loop_end) = if looped { (ls as usize, le as usize) } else { (0, n as usize) };
        slot.mute_group = if class > 0 { ((class - 1) % 15 + 1) as u8 } else { 0 };
        slots.push(slot);
    }

    let first = &regions[0];
    let envelope = [
        seconds(first.sum(G::AttackVolEnv, -12000)),
        seconds(first.sum(G::DecayVolEnv, -12000)),
        attenuation(first.sum(G::SustainVolEnv, 0)),
        seconds(first.sum(G::ReleaseVolEnv, -12000)),
    ];
    Ok(Instrument { name: p.header.name.clone(), slots, envelope: Some(envelope) })
}

// ---------------------------------------------------------------- SFZ

fn load_sfz(path: &Path) -> Result<Instrument, String> {
    use xsynth_soundfonts::LoopMode;
    let regions = xsynth_soundfonts::sfz::parse_soundfont(path).map_err(|e| e.to_string())?;
    if regions.is_empty() {
        return Err("the SFZ file has no regions with samples that exist".into());
    }
    let mut loaded: HashMap<std::path::PathBuf, Arc<Sample>> = HashMap::new();
    let mut slots = Vec::new();
    for r in &regions {
        let data = match loaded.get(&r.sample_path) {
            Some(d) => d.clone(),
            None => {
                let d =
                    Arc::new(Sample::load(&r.sample_path).map_err(|e| format!("{}: {e}", r.sample_path.display()))?);
                loaded.insert(r.sample_path.clone(), d.clone());
                d
            }
        };
        let offset = (r.offset as usize).min(data.len().saturating_sub(1));
        let data = if offset == 0 { data } else { Arc::new(data.with_frames(data.frames[offset..].to_vec())) };
        let n = data.len();
        let (lo, hi) = (*r.keyrange.start() as i32, *r.keyrange.end() as i32);
        if lo > hi || hi < 0 {
            continue;
        }
        // A sample cut by an offset is written next to the song when saved.
        let path = (offset == 0).then(|| r.sample_path.to_string_lossy().into_owned());
        let name = r.sample_path.file_stem().map_or("Sample".into(), |s| s.to_string_lossy().into_owned());
        let mut slot = SampleSlot { name, path, unsaved: offset > 0, data: Some(data), ..SampleSlot::default() };
        slot.keys = [note(lo), note(hi)];
        slot.velocities = [*r.velrange.start(), *r.velrange.end()];
        slot.base_note = note(r.pitch_keycenter as i32);
        slot.finetune = r.tune as i32;
        slot.volume = 10f32.powf(r.volume as f32 / 20.0).min(4.0);
        slot.panning = (r.pan as f32 / 100.0).clamp(-1.0, 1.0);
        slot.oneshot = r.loop_mode == LoopMode::OneShot;
        let looped = matches!(r.loop_mode, LoopMode::LoopContinuous | LoopMode::LoopSustain);
        let (ls, le) = (
            (r.loop_start as usize).saturating_sub(offset).min(n),
            (r.loop_end as usize + 1).saturating_sub(offset).min(n),
        );
        slot.loop_mode = looped as u8;
        // Without loop points the whole sample loops.
        (slot.loop_start, slot.loop_end) = if looped && ls < le && r.loop_end > 0 { (ls, le) } else { (0, n) };
        slots.push(slot);
    }
    let env = &regions[0].ampeg_envelope;
    let envelope = [env.ampeg_attack, env.ampeg_decay, env.ampeg_sustain / 100.0, env.ampeg_release];
    let name = path.file_stem().map_or("SFZ".into(), |s| s.to_string_lossy().into_owned());
    Ok(Instrument { name, slots, envelope: Some(envelope) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("noise-soundfont-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A RIFF chunk: its id, length and data, padded to an even length.
    fn chunk(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn list(kind: &[u8; 4], chunks: &[Vec<u8>]) -> Vec<u8> {
        let mut data = kind.to_vec();
        chunks.iter().for_each(|c| data.extend(c));
        chunk(b"LIST", &data)
    }

    fn name(s: &str) -> Vec<u8> {
        let mut b = s.as_bytes().to_vec();
        b.resize(20, 0);
        b
    }

    fn generator(ty: u16, amount: [u8; 2]) -> Vec<u8> {
        let mut g = ty.to_le_bytes().to_vec();
        g.extend(amount);
        g
    }

    /// A soundfont with one preset of one instrument: a looped sine on
    /// C-5 (MIDI 72) for the notes up to MIDI 71, and a stereo pair above.
    /// With `vorbis`, the sine is that Ogg Vorbis stream, as in SF3.
    fn tiny_sf2(vorbis: Option<&[u8]>) -> Vec<u8> {
        let (mut smpl, mut header) = (Vec::new(), Vec::new());
        let mut entry = |n: &str, addresses: [u32; 4], link: u16, kind: u16, root: u8| {
            header.extend(name(n));
            for v in addresses.into_iter().chain([22050]) {
                header.extend(v.to_le_bytes());
            }
            header.extend([root, 0]);
            header.extend(link.to_le_bytes());
            header.extend(kind.to_le_bytes());
        };
        let pcm = |smpl: &mut Vec<u8>| {
            let start = smpl.len() as u32 / 2;
            for k in 0..100 {
                smpl.extend((((k as f32 * 0.3).sin() * 10000.0) as i16).to_le_bytes());
            }
            smpl.extend([0; 92]);
            [start, start + 100, start + 10, start + 90]
        };
        match vorbis {
            // SF3 addresses the stream's bytes, and loops from its start.
            Some(ogg) => {
                smpl.extend(ogg);
                smpl.resize(smpl.len().next_multiple_of(2), 0);
                entry("sine", [0, ogg.len() as u32, 10, 90], 0, 0x11, 72);
            }
            None => entry("sine", pcm(&mut smpl), 0, 1, 72),
        }
        entry("left", pcm(&mut smpl), 2, 4, 60);
        entry("right", pcm(&mut smpl), 1, 2, 60);
        header.extend(name("EOS"));
        header.extend([0; 26]);

        let phdr =
            [name("Tiny"), vec![0; 4], vec![0, 0], vec![0; 12], name("EOP"), vec![0; 4], vec![1, 0], vec![0; 12]]
                .concat();
        let pbag = [0u16, 0, 1, 0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        let pgen = [generator(41, [0, 0]), generator(0, [0, 0])].concat();
        let inst = [name("Inst"), vec![0, 0], name("EOI"), vec![3, 0]].concat();
        let ibag = [0u16, 0, 4, 0, 7, 0, 10, 0].iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
        let igen = [
            generator(43, [0, 71]),
            generator(54, [1, 0]),
            generator(48, [60, 0]),
            generator(53, [0, 0]),
            generator(43, [72, 127]),
            generator(17, [0x0C, 0xFE]),
            generator(53, [1, 0]),
            generator(43, [72, 127]),
            generator(17, [0xF4, 0x01]),
            generator(53, [2, 0]),
            generator(0, [0, 0]),
        ]
        .concat();
        let mods = vec![0; 10];
        let info =
            list(b"INFO", &[chunk(b"ifil", &[2, 0, 1, 0]), chunk(b"isng", b"EMU8000\0"), chunk(b"INAM", b"Tiny\0\0")]);
        let sdta = list(b"sdta", &[chunk(b"smpl", &smpl)]);
        let pdta = list(
            b"pdta",
            &[
                chunk(b"phdr", &phdr),
                chunk(b"pbag", &pbag),
                chunk(b"pmod", &mods),
                chunk(b"pgen", &pgen),
                chunk(b"inst", &inst),
                chunk(b"ibag", &ibag),
                chunk(b"imod", &mods),
                chunk(b"igen", &igen),
                chunk(b"shdr", &header),
            ],
        );
        let body = [b"sfbk".to_vec(), info, sdta, pdta].concat();
        chunk(b"RIFF", &body)
    }

    #[test]
    fn sf2_presets_become_sampler_zones() {
        let dir = temp_dir("sf2");
        let path = dir.join("tiny.sf2");
        std::fs::write(&path, tiny_sf2(None)).unwrap();
        assert_eq!(presets(&path).unwrap(), [Preset { bank: 0, program: 0, name: "Tiny".into() }]);
        let inst = load(&path, 0).unwrap();
        assert_eq!(inst.name, "Tiny");
        assert_eq!(inst.slots.len(), 2, "the stereo pair is one slot");
        let sine = &inst.slots[0];
        assert_eq!((sine.keys, sine.base_note, sine.loop_mode), ([0, 71], 72, 1));
        assert_eq!((sine.loop_start, sine.loop_end, sine.len()), (10, 90, 100));
        assert!((sine.volume - 10f32.powf(-0.3)).abs() < 1e-4, "6 dB down");
        assert!(sine.unsaved && sine.path.is_none(), "written next to the song when saved");
        let pair = &inst.slots[1];
        assert_eq!((pair.keys, pair.base_note, pair.panning), ([72, 119], 60, 0.0));
        assert_eq!(pair.data.as_ref().unwrap().channels, 2);
        assert!(load(&path, 3).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sf3_samples_are_decoded() {
        let dir = temp_dir("sf3");
        let path = dir.join("tiny.sf3");
        let ogg = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/sine.ogg")).unwrap();
        std::fs::write(&path, tiny_sf2(Some(&ogg))).unwrap();
        let inst = load(&path, 0).unwrap();
        let sine = &inst.slots[0];
        let data = sine.data.as_ref().unwrap();
        // A quarter of a second of a 440 Hz sine at 22050 Hz, at -18 dB.
        assert_eq!(data.sample_rate, 22050.0);
        assert!((data.len() as i64 - 5512).abs() < 64, "{} frames", data.len());
        let peak = data.frames.iter().map(|f| f[0].abs()).fold(0.0, f32::max);
        assert!(peak > 0.1, "decoded audio, peak {peak}");
        assert_eq!((sine.loop_start, sine.loop_end), (10, 90));
        assert_eq!(inst.slots[1].data.as_ref().unwrap().channels, 2, "PCM samples alongside");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn broken_soundfonts_are_errors() {
        let dir = temp_dir("broken");
        let path = dir.join("broken.sf2");
        std::fs::write(&path, b"RIFF\x04\0\0\0sfbk").unwrap();
        assert!(presets(&path).is_err());
        let mut bytes = tiny_sf2(None);
        bytes.truncate(bytes.len() / 2);
        std::fs::write(&path, bytes).unwrap();
        assert!(load(&path, 0).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sfz_regions_become_sampler_zones() {
        let dir = temp_dir("sfz");
        let sample = Sample::from_interleaved("tone".into(), 44100.0, 1, &[0.5; 1000]).unwrap();
        std::fs::create_dir_all(dir.join("samples")).unwrap();
        sample.save(&dir.join("samples/tone one.wav")).unwrap();
        let sfz = "#define $ROOT 60\n<control> default_path=samples/\n<global> volume=-6\n\
                   <group> lokey=c4 hikey=b4 pitch_keycenter=$ROOT loop_mode=loop_continuous loop_start=100 loop_end=899\n\
                   <region> sample=tone one.wav lovel=1 hivel=64\n<region> sample=tone one.wav lovel=65 tune=-20 pan=-50\n";
        std::fs::write(dir.join("tone.sfz"), sfz).unwrap();
        let inst = load(&dir.join("tone.sfz"), 0).unwrap();
        assert_eq!(inst.name, "tone");
        assert_eq!(inst.slots.len(), 2);
        let (soft, loud) = (&inst.slots[0], &inst.slots[1]);
        assert_eq!((soft.keys, soft.base_note, soft.velocities), ([60, 71], 60, [1, 64]));
        assert_eq!((soft.loop_mode, soft.loop_start, soft.loop_end), (1, 100, 900));
        assert!((soft.volume - 0.501).abs() < 0.01);
        assert!(!soft.unsaved && soft.path.is_some(), "kept as the file it came from");
        assert_eq!((loud.velocities, loud.finetune, loud.panning), ([65, 127], -20, -0.5));
        assert!(Arc::ptr_eq(soft.data.as_ref().unwrap(), loud.data.as_ref().unwrap()), "loaded once");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
