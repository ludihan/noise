//! Backups of the song every few minutes while it has unsaved changes: in
//! `backups` in the data folder, named after the song and the time, the
//! newest few of each song kept. Their samples go in `backups/samples`,
//! each named by the hash of its audio, as in a project folder, so each is
//! written once however many backups play it, and a backup never depends
//! on files a song may delete.

use super::App;
use crate::project::Project;
use crate::project_dir::{self, Hashes};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// How long unsaved changes wait for a backup.
const EVERY: Duration = Duration::from_secs(180);
/// Backups kept of each song.
const KEEP: usize = 10;

pub struct Backups {
    /// When the song was last backed up, or last had no unsaved changes.
    last: Instant,
}

impl Default for Backups {
    fn default() -> Self {
        Backups { last: Instant::now() }
    }
}

/// The folder backups go in.
pub fn dir() -> Option<PathBuf> {
    crate::paths::data_dir().map(|d| d.join("backups"))
}

impl App {
    /// Backs the song up when it has had unsaved changes for a while, but
    /// not in the middle of a drag.
    pub fn backup_tick(&mut self) {
        if !self.modified {
            self.backups.last = Instant::now();
            return;
        }
        if self.backups.last.elapsed() < EVERY || self.gesture_open {
            return;
        }
        self.backups.last = Instant::now();
        let Some(dir) = dir() else { return };
        let stem = if self.untitled && !self.project.title.is_empty() {
            self.project.title.clone()
        } else {
            let name = Path::new(&self.path).file_name().map_or("song".into(), |n| n.to_string_lossy().into_owned());
            name.trim_end_matches(".json").trim_end_matches(".noise").to_string()
        };
        // Written on a thread, from a copy, as samples may need writing.
        let (song, mut hashes) = (self.project.clone(), self.hashes.clone());
        super::jobs::spawn(self, "Backing up the song", move |_| {
            let done = write(&song, &clean(&stem), &dir, &mut hashes, now_utc());
            Box::new(move |app: &mut App| match done {
                Ok(path) => app.set_status(format!("Backed up the song to {}", path.display())),
                Err(e) => app.set_status(format!("Could not back up the song to {}: {e}", dir.display())),
            })
        });
    }
}

/// `name` with only characters safe in a file name.
fn clean(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || " -_".contains(c) { c } else { '_' }).collect()
}

/// Writes a backup of `project` as `<stem> <stamp>.noise.json` in `dir`,
/// with each sample it plays in the samples folder there unless it is
/// already, and removes the oldest past `KEEP`.
fn write(project: &Project, stem: &str, dir: &Path, hashes: &mut Hashes, stamp: String) -> Result<PathBuf, String> {
    let samples = dir.join(project_dir::SAMPLES);
    std::fs::create_dir_all(&samples).map_err(|e| e.to_string())?;
    let mut p = project.clone();
    for m in &mut p.modules {
        for slot in &mut m.samples {
            let Some(data) = &slot.data else { continue };
            let file = samples.join(project_dir::sample_file(hashes, data));
            if !file.exists() {
                // Written under another name first, so the file is whole.
                let part = file.with_extension("wav.part");
                data.save(&part)?;
                std::fs::rename(&part, &file).map_err(|e| e.to_string())?;
            }
            slot.path = Some(file.to_string_lossy().into_owned());
            slot.unsaved = false;
        }
    }
    let path = dir.join(format!("{stem} {stamp}.noise.json"));
    let json = serde_json::to_string_pretty(&p).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    prune(stem, dir);
    Ok(path)
}

/// Removes all but the newest `KEEP` backups of `stem`, and the samples no
/// backup left plays: those in the shared folder, and those in the folder
/// of `stem`'s own that earlier versions kept.
fn prune(stem: &str, dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let prefix = format!("{stem} ");
    let backup = |p: &Path| p.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".noise.json"));
    let all: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| backup(p)).collect();
    let mut mine: Vec<&PathBuf> =
        all.iter().filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with(&prefix))).collect();
    // The stamps sort by time.
    mine.sort();
    let old: Vec<&PathBuf> = mine[..mine.len().saturating_sub(KEEP)].to_vec();
    for path in &old {
        let _ = std::fs::remove_file(path);
    }
    let kept: Vec<String> =
        all.iter().filter(|p| !old.contains(p)).filter_map(|p| std::fs::read_to_string(p).ok()).collect();
    for folder in [dir.join(project_dir::SAMPLES), dir.join(format!("{stem} samples"))] {
        let Ok(samples) = std::fs::read_dir(&folder) else { continue };
        for file in samples.flatten().map(|e| e.path()) {
            let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            // A path in JSON escapes only quotes, backslashes and control
            // characters.
            let quoted = serde_json::to_string(&name).unwrap_or_default();
            if !kept.iter().any(|k| k.contains(quoted.trim_matches('"'))) {
                let _ = std::fs::remove_file(file);
            }
        }
    }
}

/// The time now in UTC, as "2026-10-08 19-47-03", which sorts by time.
fn now_utc() -> String {
    let secs = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    stamp(secs)
}

fn stamp(secs: u64) -> String {
    let (days, rest) = ((secs / 86400) as i64, secs % 86400);
    // Howard Hinnant's days-to-civil.
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + (month <= 2) as i64;
    format!("{year:04}-{month:02}-{day:02} {:02}-{:02}-{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ModuleKind, SampleSlot};
    use crate::sample::Sample;

    #[test]
    fn stamps_are_dates() {
        assert_eq!(stamp(0), "1970-01-01 00-00-00");
        assert_eq!(stamp(1_791_486_423), "2026-10-08 19-07-03");
    }

    #[test]
    fn backups_keep_the_newest_and_write_each_sample_once() {
        let dir = std::env::temp_dir().join(format!("noise-backups-{}", std::process::id()));
        let mut p = Project::empty();
        let id = p.add_module(ModuleKind::Sampler, [0.0; 2]).unwrap();
        let sample = Sample { name: "s".into(), sample_rate: 8000.0, channels: 1, frames: vec![[0.5; 2]; 10] };
        let slot = SampleSlot { unsaved: true, ..SampleSlot::new(sample, None) };
        p.module_mut(id).unwrap().samples.push(slot);
        let mut hashes = Hashes::default();
        for k in 0..KEEP + 2 {
            write(&p, "song", &dir, &mut hashes, stamp(k as u64)).unwrap();
        }
        let names = |d: &Path| {
            let mut v: Vec<String> =
                std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            v.sort();
            v
        };
        let files = names(&dir);
        let backups: Vec<&String> = files.iter().filter(|f| f.ends_with(".noise.json")).collect();
        assert_eq!(backups.len(), KEEP);
        assert_eq!(backups[0], &format!("song {}.noise.json", stamp(2)), "the oldest two are gone");
        assert_eq!(
            names(&dir.join("samples")),
            [format!("{}.wav", p.module(id).unwrap().samples[0].data.as_ref().unwrap().hash())],
            "the sample is written once, by its hash"
        );
        // A backup opens with its sample.
        let (opened, warnings) = Project::load(&dir.join(backups[KEEP - 1]).to_string_lossy()).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(opened.module(id).unwrap().samples[0].data.is_some());
        // The song's own copy still has its sample to save.
        assert!(p.module(id).unwrap().samples[0].unsaved);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
