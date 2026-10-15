//! Where things live: the per-user folder, the settings file, the data folder. The only platform code of the crate.

use std::path::{Path, PathBuf};

/// Folder name under the per-user data directory.
pub const APP_DIR: &str = "RustyMCLA";

/// `%LOCALAPPDATA%\RustyMCLA` (Windows), `~/Library/Application Support/RustyMCLA` (macOS), `$XDG_DATA_HOME` or `~/.local/share` + `/RustyMCLA`.
/// `RUSTY_MCLA_USER_DIR` replaces it (tests and checks never touch the real settings, saves and logs; the launcher honours it too).
pub fn user_data_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("RUSTY_MCLA_USER_DIR") {
        return PathBuf::from(p);
    }
    let base = if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
    };
    base.unwrap_or_else(std::env::temp_dir).join(APP_DIR)
}

pub fn logs_dir() -> PathBuf {
    user_data_dir().join("logs")
}

/// The settings file: `--settings <file>`, else `MCLA_SETTINGS`, else `<user data dir>/settings.toml`.
pub fn settings_path(flag: Option<&str>) -> PathBuf {
    if let Some(f) = flag.filter(|f| !f.is_empty()) {
        return PathBuf::from(f);
    }
    if let Some(e) = std::env::var_os("MCLA_SETTINGS").filter(|e| !e.is_empty()) {
        return PathBuf::from(e);
    }
    user_data_dir().join("settings.toml")
}

/// The portable root as seen from the game exe: `game/mcla.exe` sits one level below it.
pub fn install_root(exe_dir: &Path) -> PathBuf {
    exe_dir.parent().map_or_else(|| exe_dir.join(".."), Path::to_path_buf)
}

/// Where the data folder came from, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataSource {
    /// `--data <dir>` (or a bare folder argument)
    Flag,
    /// `MCLA_DATA`
    Env,
    /// `paths.data_dir` of settings.toml
    Settings,
    /// `<exe dir>/../data/cache`
    InstallRoot,
    /// `<exe dir>/data/cache`
    BesideExe,
    /// the developer's old extraction folder. TODO-remove-before-release (docs/LAUNCHER.md, open list)
    DevFallback,
    /// nothing found: `path` is the first place that was looked at, for the error message
    NotFound,
}

pub struct DataDir {
    pub path: PathBuf,
    pub source: DataSource,
}

pub struct DataInputs<'a> {
    pub flag: Option<&'a str>,
    pub env: Option<&'a str>,
    /// `paths.data_dir`; empty = not set
    pub settings: &'a str,
    pub exe_dir: &'a Path,
    pub dev_fallback: Option<&'a Path>,
}

/// The data folder: `--data`, `MCLA_DATA`, `paths.data_dir`, `<exe dir>/../data/cache`, `<exe dir>/data/cache`, the dev
/// fallback. The first three are taken as given (a typo there is reported with that path, not silently replaced);
/// the others must exist as folders. A relative `paths.data_dir` is taken from the install root.
pub fn resolve_data_dir(i: &DataInputs) -> DataDir {
    let given = |s: Option<&str>| s.filter(|s| !s.trim().is_empty()).map(PathBuf::from);
    if let Some(p) = given(i.flag) {
        return DataDir { path: p, source: DataSource::Flag };
    }
    if let Some(p) = given(i.env) {
        return DataDir { path: p, source: DataSource::Env };
    }
    if let Some(p) = given(Some(i.settings)) {
        let p = if p.is_absolute() { p } else { install_root(i.exe_dir).join(p) };
        return DataDir { path: p, source: DataSource::Settings };
    }
    let first = install_root(i.exe_dir).join("data").join("cache");
    let candidates = [
        (first.clone(), DataSource::InstallRoot),
        (i.exe_dir.join("data").join("cache"), DataSource::BesideExe),
    ];
    for (p, source) in candidates {
        if p.is_dir() {
            return DataDir { path: p, source };
        }
    }
    if let Some(p) = i.dev_fallback.filter(|p| p.is_dir()) {
        return DataDir { path: p.to_path_buf(), source: DataSource::DevFallback };
    }
    DataDir { path: first, source: DataSource::NotFound }
}

/// The screenshots folder: `paths.screenshots` (relative = from the install root), else `<install root>/screenshots`.
pub fn screenshots_dir(setting: &str, exe_dir: &Path) -> PathBuf {
    let root = install_root(exe_dir);
    if setting.trim().is_empty() {
        return root.join("screenshots");
    }
    let p = PathBuf::from(setting);
    if p.is_absolute() {
        p
    } else {
        root.join(p)
    }
}
