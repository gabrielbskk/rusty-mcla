//! `mcla-updater`: the update system of the Rusty MCLA launcher, as a library with no UI and no Tauri code.
//!
//! * [`manifest`]: `manifest.json` (schema 1), a strict validator, and the merge with the install crate's compiled-in manifest.
//! * [`releases`]: GitHub Releases client (latest, list), conditional requests (ETag), cache file, rate-limit / offline handling.
//! * [`notes`]: release notes, GitHub markdown rendered locally to sanitised HTML.
//! * [`plan`]: what a release means for the installed launcher / game / data (semver).
//! * [`package`]: the game package (download, verify, extract with zip-slip protection, atomic swap, rollback).
//! * [`selfupdate`]: launcher self-update for a portable exe (verify hash + minisign signature, rename swap, restart).
//! * [`backup`]: save backup before an update.
//!
//! Everything downloaded is untrusted data: every size is bounded, every URL is checked against [`config::ReleaseConfig`],
//! every file is hashed before anything is touched.

pub mod backup;
pub mod config;
pub mod error;
pub mod http;
pub mod manifest;
pub mod notes;
pub mod package;
pub mod plan;
pub mod releases;
pub mod selfupdate;
pub mod version;

pub use error::{Kind, Result, UpdateError};
pub use mcla_install::Cancel;

use serde::Serialize;
use std::sync::Arc;

/// What a long job is doing, for the progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Check,
    Backup,
    DownloadGame,
    VerifyGame,
    ExtractGame,
    SwapGame,
    DownloadLauncher,
    VerifyLauncher,
    ApplyLauncher,
}

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
}

pub type ProgressFn = Arc<dyn Fn(Progress) + Send + Sync>;

pub fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}

/// Unix seconds now.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Writes through a temporary file and a rename, so a crash never leaves half a file.
pub fn write_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

pub fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}
