//! Where noise keeps its files, as the XDG base directories say (and their
//! equivalents elsewhere), through the `directories` crate: settings in
//! the config folder (`~/.config/noise`), presets the user saves in the
//! data folder (`~/.local/share/noise`). Factory presets are built into
//! the program and never written out.

use directories::{ProjectDirs, UserDirs};
use std::path::{Path, PathBuf};

fn project() -> Option<ProjectDirs> {
    ProjectDirs::from("", "", "noise")
}

/// The folder for settings: `$XDG_CONFIG_HOME/noise`.
pub fn config_dir() -> Option<PathBuf> {
    project().map(|p| p.config_dir().to_path_buf())
}

/// The folder for what the user makes and keeps, such as presets:
/// `$XDG_DATA_HOME/noise`.
pub fn data_dir() -> Option<PathBuf> {
    project().map(|p| p.data_dir().to_path_buf())
}

/// The user's home folder.
pub fn home_dir() -> Option<PathBuf> {
    UserDirs::new().map(|u| u.home_dir().to_path_buf())
}

/// The user's music folder (`XDG_MUSIC_DIR`), if they have one.
pub fn music_dir() -> Option<PathBuf> {
    UserDirs::new().and_then(|u| u.audio_dir().map(Path::to_path_buf))
}

/// Where browsing starts when nothing says otherwise: the music folder,
/// or home, rather than wherever the program was started from.
pub fn default_dir() -> PathBuf {
    music_dir().filter(|d| d.is_dir()).or_else(home_dir).or_else(|| std::env::current_dir().ok()).unwrap_or_default()
}

/// Moves `old` to `new` if `old` exists and `new` doesn't yet, for files
/// earlier versions kept elsewhere.
pub fn migrate(old: &Path, new: &Path) {
    if old.exists() && !new.exists() {
        if let Some(parent) = new.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::rename(old, new);
    }
}
