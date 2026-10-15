//! The install manifest: the numbers that identify the supported disc, the data version, the pinned ffmpeg build.
//! They live in `data/install_manifest.json` (compiled in as the default); a release can ship a newer `manifest.json`
//! and pass it to [`Manifest::from_json`] without a new launcher build.

use crate::error::{ErrorKind, InstallError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCHEMA: u32 = 1;
const EMBEDDED: &str = include_str!("../data/install_manifest.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema: u32,
    /// Bumped when a game release needs differently built data; the launcher then offers a rebuild.
    pub data_version: u32,
    pub iso: IsoSpec,
    pub movies: Vec<Movie>,
    pub ffmpeg: FfmpegSpec,
    pub space: SpaceSpec,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsoSpec {
    pub name: String,
    pub size: u64,
    pub sha256: String,
    /// Title id of `default.xex` (hex, 8 digits). Empty = not checked.
    pub title_id: String,
    /// The archive whose table is used for the key and the entry count.
    pub game_archive: String,
    pub rpf_entries: u32,
    /// Files that must be on the disc.
    pub required_files: Vec<String>,
    /// Archives whose `audio/X360/sfx` tree is converted to `audio_pc`, besides the game archive's own (the config files come from the game archive).
    pub audio_archives: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Movie {
    /// Output name (`<name>.mcv`, `<name>.mp3`).
    pub name: String,
    /// File on the disc.
    pub file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FfmpegSpec {
    pub version: String,
    /// Licence of the pinned build (shown to the user before the download).
    pub license: String,
    /// Keyed by `<os>-<arch>`, for example `windows-x86_64`.
    pub downloads: BTreeMap<String, Download>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Download {
    pub url: String,
    pub size: u64,
    pub sha256: String,
    /// Path of the ffmpeg executable inside the archive.
    pub member: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceSpec {
    /// Bytes of the built data folder (cache + audio_pc + video_pc), measured.
    pub data_bytes: u64,
    /// Extra headroom the preflight asks for (temp files, ffmpeg).
    pub headroom_bytes: u64,
    /// Longest relative path under `data/` (characters), measured; used by the long-path check.
    pub max_rel_path: u32,
}

impl Manifest {
    pub fn embedded() -> Manifest {
        // The embedded file is checked by a unit test.
        Manifest::from_json(EMBEDDED).expect("embedded install manifest is valid")
    }

    pub fn from_json(text: &str) -> Result<Manifest> {
        let mut v: serde_json::Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| InstallError::new(ErrorKind::Manifest, e.to_string()))?;
        // A release's manifest.json (launcher/manifest.example.json) keeps the data version in `game.data_version`; accept it as is.
        if v.get("data_version").is_none() {
            if let Some(d) = v.pointer("/game/data_version").cloned() {
                v["data_version"] = d;
            }
        }
        let m: Manifest = serde_json::from_value(v).map_err(|e| InstallError::new(ErrorKind::Manifest, e.to_string()))?;
        if m.schema != SCHEMA {
            return Err(InstallError::new(ErrorKind::Manifest, format!("manifest schema {} (this build reads {})", m.schema, SCHEMA)));
        }
        Ok(m)
    }

    pub fn load(path: &std::path::Path) -> Result<Manifest> {
        let t = std::fs::read_to_string(path).map_err(|e| InstallError::io(path.display(), &e))?;
        Manifest::from_json(&t)
    }

    /// Bytes the preflight wants free in the install folder's drive.
    pub fn required_free_bytes(&self) -> u64 {
        self.space.data_bytes + self.space.headroom_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_manifest_parses() {
        let m = Manifest::embedded();
        assert_eq!(m.schema, SCHEMA);
        assert_eq!(m.iso.sha256.len(), 64);
        assert!(m.iso.rpf_entries > 0);
        assert!(m.ffmpeg.downloads.values().all(|d| d.sha256.len() == 64 && d.url.starts_with("https://")));
    }

    #[test]
    fn newer_schema_is_refused() {
        let mut v: serde_json::Value = serde_json::from_str(EMBEDDED).unwrap();
        v["schema"] = 99.into();
        assert_eq!(Manifest::from_json(&v.to_string()).unwrap_err().kind, ErrorKind::Manifest);
    }
}
