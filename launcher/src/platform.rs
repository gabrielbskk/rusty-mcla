//! Everything that differs per operating system lives here, so the rest of the launcher has no `cfg(windows)`.
//! Windows is the only target built today; Linux and macOS add their branches in this file only.

use std::path::{Path, PathBuf};

/// Folder name used under the per-user data directory.
pub const APP_DIR: &str = "RustyMCLA";

/// `%LOCALAPPDATA%\RustyMCLA` (Windows), `~/.local/share/RustyMCLA` (Linux), `~/Library/Application Support/RustyMCLA` (macOS).
pub fn user_data_dir() -> PathBuf {
    // developer / test override, so checks never touch the real settings, saves and logs
    if let Some(p) = std::env::var_os("RUSTY_MCLA_USER_DIR") {
        return PathBuf::from(p);
    }
    dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join(APP_DIR)
}

/// The folder the launcher exe sits in: the portable root (game, data, tools and screenshots live beside it).
pub fn portable_root() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn os_name() -> &'static str {
    std::env::consts::OS
}

/// Opens an http(s) URL in the default browser. The caller has already checked the scheme; the URL is one argument, never a shell line.
pub fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let program = "explorer.exe";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    std::process::Command::new(program).arg(url).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Opens a folder in the system file manager. The caller passes only folders the launcher itself resolved.
pub fn open_folder(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let program = "explorer.exe";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    std::process::Command::new(program).arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Shows a file in the system file manager with the file selected (Windows); elsewhere its folder is opened.
/// The caller passes only a path the launcher itself wrote.
pub fn reveal_file(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // `explorer /select,"<path>"` must reach explorer exactly like this, so the argument is passed raw
        return std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", path.display())).spawn().map(|_| ()).map_err(|e| e.to_string());
    }
    #[cfg(not(target_os = "windows"))]
    {
        match path.parent() {
            Some(dir) => open_folder(dir),
            None => Err("no folder".into()),
        }
    }
}
