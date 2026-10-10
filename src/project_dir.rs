//! Songs on disk as project folders: the song in `project.json`, and in
//! `samples/` every sample it plays, as a WAV file named by a hash of its
//! audio, so the same audio is stored once however many slots play it.
//! Saving writes the samples the folder lacks and deletes those nothing
//! plays any more. A project exports to a `.noise` file, a zip archive of
//! its folder, which imports back as a folder.

use crate::project::Project;
use crate::sample::Sample;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The song's file in a project folder.
pub const SONG_FILE: &str = "project.json";
/// The folder in a project folder that its samples are in.
pub const SAMPLES: &str = "samples";
/// The extension of a project exported to a file: a zip archive.
pub const ARCHIVE: &str = "noise";

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Whether `dir` is a project folder.
pub fn is_project(dir: &Path) -> bool {
    dir.join(SONG_FILE).is_file()
}

/// Whether `path` names a project exported to a file.
pub fn is_archive(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(ARCHIVE))
}

/// The song file at `path`: a project folder's, or `path` itself, a song
/// file (a project's, or a song saved on its own before projects were
/// folders).
pub fn song_file(path: &Path) -> PathBuf {
    if path.is_dir() { path.join(SONG_FILE) } else { path.to_path_buf() }
}

/// The project folder that `path` opens: the folder, or the one a project's
/// song file is in; `None` for a song saved on its own.
pub fn folder_of(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        Some(path.to_path_buf())
    } else if path.file_name().is_some_and(|n| n == SONG_FILE) {
        path.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

/// Opens the song at `path`: a project folder, or a song file.
pub fn open(path: &Path) -> Result<(Project, Vec<String>), String> {
    Project::load(&song_file(path).to_string_lossy())
}

/// The hashes of samples' audio, kept for the audio they were taken from,
/// so saving doesn't hash the same audio again.
#[derive(Clone, Default)]
pub struct Hashes(HashMap<usize, (Arc<Sample>, String)>);

impl Hashes {
    /// The hash of `data`'s audio.
    pub fn of(&mut self, data: &Arc<Sample>) -> String {
        // Holding the audio keeps its address from being reused.
        let (_, hash) = self.0.entry(Arc::as_ptr(data) as usize).or_insert_with(|| (data.clone(), data.hash()));
        hash.clone()
    }

    /// Forgets audio that nothing else holds any more.
    pub fn forget_unused(&mut self) {
        self.0.retain(|_, (data, _)| Arc::strong_count(data) > 1);
    }
}

/// The name of the file a sample with audio `data` is kept in.
pub fn sample_file(hashes: &mut Hashes, data: &Arc<Sample>) -> String {
    format!("{}.wav", hashes.of(data))
}

/// What saving did: the samples it wrote, and those it deleted because
/// nothing plays them any more.
#[derive(Debug, PartialEq)]
pub struct Saved {
    pub written: usize,
    pub removed: usize,
}

/// Saves `project` as the project folder `dir`: its song file, and in
/// `samples/` each sample it plays that the folder lacks, named by its
/// hash; the samples nothing plays any more are deleted. The project's
/// slots then point at their files. `dir` has to be new, empty or a
/// project folder already, so saving never deletes anything else.
pub fn save(project: &mut Project, dir: &Path, hashes: &mut Hashes) -> Result<Saved, String> {
    let dir = std::path::absolute(dir).map_err(err)?;
    if dir.exists() && !is_project(&dir) && std::fs::read_dir(&dir).map_err(err)?.next().is_some() {
        return Err(format!("{} has other files in it and isn't a project folder", dir.display()));
    }
    let samples = dir.join(SAMPLES);
    std::fs::create_dir_all(&samples).map_err(err)?;
    // The song as written: its samples where they are in the folder.
    let mut song = project.clone();
    let (mut keep, mut written) = (HashSet::new(), 0);
    for (m, copy) in project.modules.iter_mut().zip(&mut song.modules) {
        for (slot, saved) in m.samples.iter_mut().zip(&mut copy.samples) {
            // A sample whose file was missing keeps pointing where it was.
            let Some(data) = &slot.data else { continue };
            let file = sample_file(hashes, data);
            let path = samples.join(&file);
            if !path.exists() {
                // Written under another name first, so the file is whole.
                let part = samples.join(format!("{file}.part"));
                data.save(&part)?;
                std::fs::rename(&part, &path).map_err(err)?;
                written += 1;
            }
            saved.path = Some(format!("{SAMPLES}/{file}"));
            slot.path = Some(path.to_string_lossy().into_owned());
            slot.unsaved = false;
            keep.insert(file);
        }
    }
    let json = serde_json::to_string_pretty(&song).map_err(err)?;
    let part = dir.join(format!("{SONG_FILE}.part"));
    std::fs::write(&part, json).map_err(err)?;
    std::fs::rename(&part, dir.join(SONG_FILE)).map_err(err)?;
    let mut removed = 0;
    for entry in std::fs::read_dir(&samples).map_err(err)?.flatten() {
        if !keep.contains(entry.file_name().to_string_lossy().as_ref()) && entry.path().is_file() {
            std::fs::remove_file(entry.path()).map_err(err)?;
            removed += 1;
        }
    }
    hashes.forget_unused();
    Ok(Saved { written, removed })
}

/// Writes `project` to `path`, a zip archive, as a project folder named
/// `name`: its song file and every sample it plays, named by its hash, so
/// importing the archive makes the same folder.
pub fn export(project: &Project, name: &str, path: &Path, hashes: &mut Hashes) -> Result<(), String> {
    // Written under another name first, so the archive is whole.
    let mut part = path.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let written = write_zip(project, name, &part, hashes).and_then(|()| std::fs::rename(&part, path).map_err(err));
    if written.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    written
}

fn write_zip(project: &Project, name: &str, path: &Path, hashes: &mut Hashes) -> Result<(), String> {
    let name: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':') { '_' } else { c }).collect();
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(std::fs::File::create(path).map_err(err)?));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut song = project.clone();
    let mut done = HashSet::new();
    for m in &mut song.modules {
        for slot in &mut m.samples {
            let Some(data) = &slot.data else { continue };
            let file = sample_file(hashes, data);
            if done.insert(file.clone()) {
                zip.start_file(format!("{name}/{SAMPLES}/{file}"), options).map_err(err)?;
                zip.write_all(&data.wav()?).map_err(err)?;
            }
            slot.path = Some(format!("{SAMPLES}/{file}"));
        }
    }
    zip.start_file(format!("{name}/{SONG_FILE}"), options).map_err(err)?;
    zip.write_all(serde_json::to_string_pretty(&song).map_err(err)?.as_bytes()).map_err(err)?;
    zip.finish().map_err(err)?;
    Ok(())
}

/// Unpacks the project in the zip archive `path` into a new folder in
/// `into`, named after the project's folder in the archive (or the
/// archive), with a number added if that name is taken. Only its song file
/// and samples come out, and no name in the archive reaches outside the
/// folder. Returns the folder.
pub fn import(path: &Path, into: &Path) -> Result<PathBuf, String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).map_err(err)?).map_err(err)?;
    // The song file: at the top of the archive, or in a folder at its top.
    let names: Vec<PathBuf> = (0..zip.len()).filter_map(|i| zip.by_index(i).ok()?.enclosed_name()).collect();
    let song = names
        .iter()
        .filter(|n| n.file_name().is_some_and(|f| f == SONG_FILE) && n.components().count() <= 2)
        .min_by_key(|n| n.components().count())
        .ok_or_else(|| format!("{} holds no project", path.display()))?;
    let root = song.parent().unwrap_or(Path::new("")).to_path_buf();
    let stem = path.file_stem().map_or_else(|| "project".into(), |s| s.to_string_lossy().into_owned());
    let name = root.file_name().map_or(stem, |n| n.to_string_lossy().into_owned());
    let dir = free_name(&into.join(name));
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(err)?;
        let Some(inner) = entry.enclosed_name().and_then(|n| n.strip_prefix(&root).ok().map(Path::to_path_buf)) else {
            continue;
        };
        if !entry.is_file() || !(inner == Path::new(SONG_FILE) || inner.starts_with(SAMPLES)) {
            continue;
        }
        let out = dir.join(&inner);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(err)?;
        }
        let mut file = std::fs::File::create(&out).map_err(err)?;
        std::io::copy(&mut entry, &mut file).map_err(err)?;
    }
    Ok(dir)
}

/// Unpacks the project in the archive at `path` into a folder beside it, as
/// `import` does.
pub fn import_beside(path: &Path) -> Result<PathBuf, String> {
    import(path, path.parent().unwrap_or(Path::new(".")))
}

/// `path`, or with " 2", " 3" and so on added until nothing has the name.
fn free_name(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    (2..).map(|k| path.with_file_name(format!("{name} {k}"))).find(|p| !p.exists()).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ModuleKind, OUTPUT_ID, SampleSlot};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("noise-projects-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn tone(name: &str, level: f32) -> Sample {
        Sample { name: name.into(), sample_rate: 8000.0, channels: 1, frames: vec![[level; 2]; 100] }
    }

    /// A Sampler with three slots: two with the same audio under other
    /// names, one with audio of its own.
    fn song() -> (Project, u8) {
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Sampler, [0.0; 2]).unwrap();
        p.connect(id, OUTPUT_ID);
        let slots = [tone("a", 0.5), tone("same as a", 0.5), tone("b", 0.25)];
        p.module_mut(id).unwrap().samples =
            slots.into_iter().map(|s| SampleSlot { unsaved: true, ..SampleSlot::new(s, None) }).collect();
        (p, id)
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_hash_names_the_audio_and_nothing_else() {
        let a = tone("a", 0.5);
        assert_eq!(a.hash(), "7bc718d2fd2590a1386ad1d8273c3b59", "the same on every run and build");
        assert_eq!(tone("another name", 0.5).hash(), a.hash());
        assert_ne!(tone("a", 0.25).hash(), a.hash());
        assert_ne!(Sample { sample_rate: 16000.0, ..tone("a", 0.5) }.hash(), a.hash());
    }

    #[test]
    fn saving_keeps_each_sample_once_by_its_hash_and_deletes_the_unused() {
        let dir = temp_dir("save").join("Song");
        let (mut p, id) = song();
        let mut hashes = Hashes::default();
        assert_eq!(save(&mut p, &dir, &mut hashes).unwrap(), Saved { written: 2, removed: 0 });
        let a = format!("{}.wav", tone("a", 0.5).hash());
        let b = format!("{}.wav", tone("b", 0.25).hash());
        let mut both = vec![a.clone(), b.clone()];
        both.sort();
        assert_eq!(files(&dir.join(SAMPLES)), both, "the same audio is kept once");
        assert_eq!(files(&dir), [SONG_FILE, SAMPLES]);
        // The song points at them inside its folder, so it can move.
        let json = std::fs::read_to_string(dir.join(SONG_FILE)).unwrap();
        assert!(json.contains(&format!("\"{SAMPLES}/{a}\"")) && !json.contains(&*dir.to_string_lossy()));
        assert!(p.module(id).unwrap().samples.iter().all(|s| !s.unsaved));
        // It opens with its samples, the slots with the same file sharing it.
        let (opened, warnings) = open(&dir).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let slots = &opened.module(id).unwrap().samples;
        assert!(Arc::ptr_eq(slots[0].data.as_ref().unwrap(), slots[1].data.as_ref().unwrap()));
        assert_eq!(slots[2].data.as_ref().unwrap().frames, tone("b", 0.25).frames);
        // Saving again writes nothing; a sample no slot plays any more, and
        // a stray file, are deleted.
        std::fs::write(dir.join(SAMPLES).join("stray.wav"), "").unwrap();
        p.module_mut(id).unwrap().samples.truncate(2);
        assert_eq!(save(&mut p, &dir, &mut hashes).unwrap(), Saved { written: 0, removed: 2 });
        assert_eq!(files(&dir.join(SAMPLES)), [a]);
        std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_demo_songs_save_as_project_folders_and_open_the_same() {
        let dir = temp_dir("demos");
        for (k, make) in [Project::demo, Project::static_heart, Project::clockwork_rain, Project::prism_overdrive]
            .into_iter()
            .enumerate()
        {
            let folder = dir.join(k.to_string());
            let mut p = make();
            save(&mut p, &folder, &mut Hashes::default()).unwrap();
            let (opened, warnings) = open(&folder).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_eq!(serde_json::to_string(&opened).unwrap(), serde_json::to_string(&p).unwrap());
            let audio = |p: &Project| -> HashSet<String> {
                p.modules.iter().flat_map(|m| &m.samples).filter_map(|s| s.data.as_ref()).map(|d| d.hash()).collect()
            };
            assert_eq!(audio(&opened), audio(&p), "every sample came back");
            assert_eq!(files(&folder.join(SAMPLES)).len(), audio(&p).len(), "each once");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saving_leaves_folders_that_arent_projects_alone() {
        let dir = temp_dir("not-a-project");
        std::fs::write(dir.join("notes.txt"), "mine").unwrap();
        let (mut p, _) = song();
        assert!(save(&mut p, &dir, &mut Hashes::default()).is_err());
        assert_eq!(files(&dir), ["notes.txt"]);
        // An empty folder is fine.
        let empty = dir.join("empty");
        std::fs::create_dir(&empty).unwrap();
        save(&mut p, &empty, &mut Hashes::default()).unwrap();
        assert!(is_project(&empty));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn projects_export_to_zip_archives_and_import_back() {
        let dir = temp_dir("zip");
        let (p, id) = song();
        let mut hashes = Hashes::default();
        let zip = dir.join(format!("Song.{ARCHIVE}"));
        export(&p, "My Song", &zip, &mut hashes).unwrap();
        assert!(is_archive(&zip) && files(&dir) == [format!("Song.{ARCHIVE}")], "only the whole file is left");
        let imported = import(&zip, &dir).unwrap();
        assert_eq!(imported, dir.join("My Song"), "named after the project's folder in the archive");
        assert_eq!(files(&imported.join(SAMPLES)).len(), 2);
        let (opened, warnings) = open(&imported).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let names: Vec<&str> = opened.module(id).unwrap().samples.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["a", "same as a", "b"]);
        assert!(opened.module(id).unwrap().samples.iter().all(|s| s.data.is_some()));
        // Imported again, it gets a name of its own.
        assert_eq!(import(&zip, &dir).unwrap(), dir.join("My Song 2"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn importing_keeps_to_its_own_folder() {
        let dir = temp_dir("unzip");
        let zip = dir.join("odd.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, text) in [("../outside.txt", "no"), ("odd/project.json", "{}"), ("odd/notes.txt", "left out")] {
            w.start_file(name, options).unwrap();
            w.write_all(text.as_bytes()).unwrap();
        }
        w.finish().unwrap();
        let into = dir.join("into");
        std::fs::create_dir(&into).unwrap();
        let imported = import(&zip, &into).unwrap();
        assert_eq!(files(&imported), [SONG_FILE]);
        assert!(!dir.join("outside.txt").exists());
        // An archive without a project is refused.
        let empty = dir.join("empty.zip");
        zip::ZipWriter::new(std::fs::File::create(&empty).unwrap()).finish().unwrap();
        assert!(import(&empty, &into).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
