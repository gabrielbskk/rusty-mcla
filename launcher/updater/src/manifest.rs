//! `manifest.json`, the release's description of itself (schema 1). One release publishes `RustyMCLA.exe`, `game-<ver>.zip` and this file.
//!
//! ```text
//! { "schema": 1, "version": "0.2.0", "min_launcher_version": "0.1.0", "notes_url": "https://github.com/OWNER/rusty-mcla/releases/tag/v0.2.0",
//!   "launcher": { "version", "asset", "size", "sha256", "signature" },       // signature = the whole RustyMCLA.exe.minisig text
//!   "game":     { "version", "asset", "size", "sha256", "data_version" },
//!   "iso": {..}, "movies": [..], "ffmpeg": {..}, "space": {..} }             // exactly the install crate's `Manifest` parts
//! ```
//!
//! The install parts (disc identity, ffmpeg pin, space numbers) are the install crate's own types, so there is one definition: the
//! compiled-in `install_manifest.json` is the default and the release's copy overrides it ([`ReleaseManifest::to_install_manifest`]).
//! A manifest is **untrusted data**: [`ReleaseManifest::parse`] refuses an unknown schema, a missing or malformed hash, a URL that is not
//! https on the allow-list, and absurd sizes, before anything is downloaded because of it.

use crate::config::ReleaseConfig;
use crate::error::{Kind, Result, UpdateError};
use crate::version;
use mcla_install::manifest::{FfmpegSpec, IsoSpec, Movie, SpaceSpec};
use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
pub const MIN_LAUNCHER_BYTES: u64 = 100 * 1024;
pub const MAX_LAUNCHER_BYTES: u64 = 256 * 1024 * 1024;
pub const MIN_GAME_BYTES: u64 = 1024;
pub const MAX_GAME_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const MAX_FFMPEG_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_SIGNATURE_BYTES: usize = 2048;
pub const MAX_DATA_VERSION: u32 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherAsset {
    pub version: String,
    /// Asset name in the release (`RustyMCLA.exe`).
    pub asset: String,
    pub size: u64,
    pub sha256: String,
    /// The complete text of the minisign signature file (`RustyMCLA.exe.minisig`) made offline with the release key.
    pub signature: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameAsset {
    pub version: String,
    pub asset: String,
    pub size: u64,
    pub sha256: String,
    /// Bumped when this game needs differently built data (then the launcher offers a rebuild from the disc).
    pub data_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub schema: u32,
    /// The release's version (the tag without `v`).
    pub version: String,
    /// The game package of this release needs at least this launcher.
    pub min_launcher_version: String,
    pub notes_url: String,
    pub launcher: LauncherAsset,
    pub game: GameAsset,
    pub iso: IsoSpec,
    pub movies: Vec<Movie>,
    pub ffmpeg: FfmpegSpec,
    pub space: SpaceSpec,
}

fn bad(msg: impl Into<String>) -> UpdateError {
    UpdateError::new(Kind::Manifest, msg)
}

fn check_sha(what: &str, s: &str) -> Result<()> {
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(bad(format!("{what}: sha256 must be 64 hex digits")));
    }
    if s.bytes().all(|b| b == b'0') {
        return Err(bad(format!("{what}: sha256 is all zeros")));
    }
    Ok(())
}

fn check_size(what: &str, n: u64, min: u64, max: u64) -> Result<()> {
    if n < min || n > max {
        return Err(bad(format!("{what}: size {n} is outside {min}..={max}")));
    }
    Ok(())
}

/// A plain file name: no separators, no `..`, no control characters, sensible length.
pub fn check_asset_name(what: &str, s: &str) -> Result<()> {
    let ok = !s.is_empty() && s.len() <= 100 && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(bad(format!("{what}: not a plain file name: {}", s.chars().take(60).collect::<String>())))
    }
}

fn check_version(what: &str, s: &str) -> Result<semver::Version> {
    let v = version::parse(s).ok_or_else(|| bad(format!("{what}: not a version: {}", s.chars().take(40).collect::<String>())))?;
    if s.trim() != s || s.starts_with('v') {
        return Err(bad(format!("{what}: write the version without a leading v or spaces: {s}")));
    }
    Ok(v)
}

fn check_text(what: &str, s: &str, max: usize) -> Result<()> {
    if s.len() > max || s.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err(bad(format!("{what}: too long or has control characters")));
    }
    Ok(())
}

impl ReleaseManifest {
    /// Parses and fully validates a manifest.
    pub fn parse(text: &str, cfg: &ReleaseConfig) -> Result<ReleaseManifest> {
        if text.len() > MAX_MANIFEST_BYTES {
            return Err(UpdateError::new(Kind::TooLarge, format!("manifest is {} bytes (limit {MAX_MANIFEST_BYTES})", text.len())));
        }
        let text = text.trim_start_matches('\u{feff}');
        let raw: serde_json::Value = serde_json::from_str(text).map_err(|e| bad(format!("not valid JSON: {e}")))?;
        match raw.get("schema").and_then(|v| v.as_u64()) {
            Some(s) if s == SCHEMA as u64 => {}
            Some(s) => return Err(bad(format!("manifest schema {s} (this launcher reads {SCHEMA}); update the launcher"))),
            None => return Err(bad("manifest has no schema number")),
        }
        let m: ReleaseManifest = serde_json::from_value(raw).map_err(|e| bad(format!("missing or wrong field: {e}")))?;
        m.validate(cfg)?;
        Ok(m)
    }

    pub fn validate(&self, cfg: &ReleaseConfig) -> Result<()> {
        if self.schema != SCHEMA {
            return Err(bad(format!("manifest schema {}", self.schema)));
        }
        let v = check_version("version", &self.version)?;
        let min = check_version("min_launcher_version", &self.min_launcher_version)?;
        let lv = check_version("launcher.version", &self.launcher.version)?;
        check_version("game.version", &self.game.version)?;
        if min > lv {
            return Err(bad("min_launcher_version is newer than the launcher in the same release"));
        }
        let _ = v;
        cfg.check_url(&self.notes_url).map_err(|e| bad(format!("notes_url: {}", e.detail)))?;

        check_asset_name("launcher.asset", &self.launcher.asset)?;
        check_sha("launcher", &self.launcher.sha256)?;
        check_size("launcher", self.launcher.size, MIN_LAUNCHER_BYTES, MAX_LAUNCHER_BYTES)?;
        if self.launcher.signature.is_empty() || self.launcher.signature.len() > MAX_SIGNATURE_BYTES {
            return Err(bad("launcher.signature: missing or too long"));
        }
        minisign_verify::Signature::decode(&self.launcher.signature).map_err(|e| bad(format!("launcher.signature does not decode: {e}")))?;

        check_asset_name("game.asset", &self.game.asset)?;
        if !self.game.asset.to_ascii_lowercase().ends_with(".zip") {
            return Err(bad("game.asset must be a .zip"));
        }
        check_sha("game", &self.game.sha256)?;
        check_size("game", self.game.size, MIN_GAME_BYTES, MAX_GAME_BYTES)?;
        if self.game.data_version == 0 || self.game.data_version > MAX_DATA_VERSION {
            return Err(bad(format!("game.data_version {} is outside 1..={MAX_DATA_VERSION}", self.game.data_version)));
        }

        // install parts
        check_sha("iso", &self.iso.sha256)?;
        check_size("iso", self.iso.size, 1_000_000_000, 40_000_000_000)?;
        check_text("iso.name", &self.iso.name, 300)?;
        if !self.iso.title_id.is_empty() && !(self.iso.title_id.len() == 8 && self.iso.title_id.bytes().all(|b| b.is_ascii_hexdigit())) {
            return Err(bad("iso.title_id must be 8 hex digits or empty"));
        }
        if self.iso.rpf_entries == 0 || self.iso.rpf_entries > 10_000_000 {
            return Err(bad("iso.rpf_entries is absurd"));
        }
        for f in self.iso.required_files.iter().chain(self.iso.audio_archives.iter()).chain(std::iter::once(&self.iso.game_archive)) {
            check_disc_name("iso file name", f)?;
        }
        if self.iso.required_files.is_empty() || self.iso.required_files.len() > 64 {
            return Err(bad("iso.required_files: empty or too many"));
        }
        if self.movies.len() > 32 {
            return Err(bad("too many movies"));
        }
        for m in &self.movies {
            check_asset_name("movie name", &m.name)?;
            check_disc_name("movie file", &m.file)?;
        }
        check_text("ffmpeg.version", &self.ffmpeg.version, 40)?;
        check_text("ffmpeg.license", &self.ffmpeg.license, 2000)?;
        if self.ffmpeg.license.trim().is_empty() {
            return Err(bad("ffmpeg.license is required"));
        }
        if self.ffmpeg.downloads.len() > 16 {
            return Err(bad("too many ffmpeg downloads"));
        }
        for (key, d) in &self.ffmpeg.downloads {
            let what = format!("ffmpeg.downloads.{key}");
            if key.len() > 40 || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(bad(format!("{what}: bad platform key")));
            }
            cfg.check_url(&d.url).map_err(|e| bad(format!("{what}.url: {}", e.detail)))?;
            check_sha(&what, &d.sha256)?;
            check_size(&what, d.size, 1, MAX_FFMPEG_BYTES)?;
            if d.member.is_empty() || d.member.len() > 200 || d.member.contains("..") || d.member.starts_with('/') || d.member.contains(':') || d.member.contains('\\') {
                return Err(bad(format!("{what}.member: not a safe path inside the archive")));
            }
        }
        if self.space.data_bytes == 0 || self.space.data_bytes > 500_000_000_000 || self.space.headroom_bytes > 100_000_000_000 || self.space.max_rel_path > 4096 {
            return Err(bad("space numbers are absurd"));
        }
        Ok(())
    }

    /// The install crate's manifest with this release's values: the compiled-in manifest is the default, this overrides it.
    pub fn to_install_manifest(&self) -> mcla_install::Manifest {
        mcla_install::Manifest {
            schema: mcla_install::manifest::SCHEMA,
            data_version: self.game.data_version,
            iso: self.iso.clone(),
            movies: self.movies.clone(),
            ffmpeg: self.ffmpeg.clone(),
            space: self.space.clone(),
        }
    }

    /// The version of this release as the tag would show it.
    pub fn tag(&self) -> String {
        format!("v{}", self.version)
    }
}

fn check_disc_name(what: &str, s: &str) -> Result<()> {
    let ok = !s.is_empty() && s.len() <= 120 && !s.contains("..") && !s.starts_with('/') && !s.contains('\\') && !s.contains(':') && s.chars().all(|c| !c.is_control());
    if ok {
        Ok(())
    } else {
        Err(bad(format!("{what}: not a safe name: {}", s.chars().take(60).collect::<String>())))
    }
}

/// Everything `build` needs besides the install parts.
pub struct BuildInput {
    pub version: String,
    pub launcher_version: String,
    pub game_version: String,
    pub min_launcher_version: String,
    pub data_version: u32,
    pub notes_url: String,
    pub launcher_asset: String,
    pub launcher_size: u64,
    pub launcher_sha256: String,
    pub launcher_signature: String,
    pub game_asset: String,
    pub game_size: u64,
    pub game_sha256: String,
}

/// Produces a manifest for a release: asset facts from `input`, install parts from `install` (the compiled-in manifest unless the
/// caller passes another). The result is not yet validated; `mk-manifest` validates it before writing.
pub fn build(input: BuildInput, install: &mcla_install::Manifest) -> ReleaseManifest {
    ReleaseManifest {
        schema: SCHEMA,
        version: input.version,
        min_launcher_version: input.min_launcher_version,
        notes_url: input.notes_url,
        launcher: LauncherAsset { version: input.launcher_version, asset: input.launcher_asset, size: input.launcher_size, sha256: input.launcher_sha256, signature: input.launcher_signature },
        game: GameAsset { version: input.game_version, asset: input.game_asset, size: input.game_size, sha256: input.game_sha256, data_version: input.data_version },
        iso: install.iso.clone(),
        movies: install.movies.clone(),
        ffmpeg: install.ffmpeg.clone(),
        space: install.space.clone(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A syntactically valid minisign signature file (74 bytes of payload, not a real signature): enough for `Signature::decode`.
    pub const SAMPLE_SIGNATURE: &str = "untrusted comment: signature from minisign secret key\nRUQOFRwjKjE4P0ZNVFtiaXB3foWMk5qhqK+2vcTL0tng5+71AQgPFh0kKzI5QEdOVVxjanF4f4aNlJuiqbC3vsXM09rh6O/2Agk=\ntrusted comment: timestamp:0\nBRIfLDlGU2BteoeUoa67yNXi7wEOGyg1Qk9caXaDkJ2qt8TR3uv4ChckMT5LWGVyf4yZprPAzdrn9AYTIC06Rw==\n";

    pub fn cfg() -> ReleaseConfig {
        ReleaseConfig::embedded("0.1.0")
    }

    pub fn sample() -> ReleaseManifest {
        let input = BuildInput {
            version: "0.2.0".into(),
            launcher_version: "0.2.0".into(),
            game_version: "0.2.0".into(),
            min_launcher_version: "0.1.0".into(),
            data_version: 1,
            notes_url: "https://github.com/OWNER/rusty-mcla/releases/tag/v0.2.0".into(),
            launcher_asset: "RustyMCLA.exe".into(),
            launcher_size: 5_000_000,
            launcher_sha256: "a".repeat(64),
            launcher_signature: SAMPLE_SIGNATURE.into(),
            game_asset: "game-0.2.0.zip".into(),
            game_size: 40_000_000,
            game_sha256: "b".repeat(64),
        };
        build(input, &mcla_install::Manifest::embedded())
    }

    fn with(edit: impl FnOnce(&mut serde_json::Value)) -> Result<ReleaseManifest> {
        let mut v = serde_json::to_value(sample()).unwrap();
        edit(&mut v);
        ReleaseManifest::parse(&v.to_string(), &cfg())
    }

    #[test]
    fn a_good_manifest_parses_and_round_trips() {
        let m = sample();
        let text = serde_json::to_string_pretty(&m).unwrap();
        let back = ReleaseManifest::parse(&text, &cfg()).unwrap();
        assert_eq!(back.game.data_version, 1);
        assert_eq!(back.tag(), "v0.2.0");
    }

    #[test]
    fn the_example_file_is_valid() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../manifest.example.json")).expect("launcher/manifest.example.json");
        let m = ReleaseManifest::parse(&text, &cfg()).expect("example manifest validates");
        assert_eq!(m.schema, 1);
    }

    #[test]
    fn unknown_schema_is_refused() {
        let e = with(|v| v["schema"] = 2.into()).unwrap_err();
        assert_eq!(e.kind, Kind::Manifest);
        assert!(e.detail.contains("schema 2"));
        assert!(with(|v| { v.as_object_mut().unwrap().remove("schema"); }).is_err());
    }

    #[test]
    fn missing_or_bad_hashes_are_refused() {
        assert!(with(|v| { v["game"].as_object_mut().unwrap().remove("sha256"); }).is_err());
        assert!(with(|v| v["game"]["sha256"] = "zz".into()).is_err());
        assert!(with(|v| v["game"]["sha256"] = "0".repeat(64).into()).is_err());
        assert!(with(|v| v["launcher"]["sha256"] = "a".repeat(63).into()).is_err());
        assert!(with(|v| v["iso"]["sha256"] = "".into()).is_err());
        assert!(with(|v| v["ffmpeg"]["downloads"]["windows-x86_64"]["sha256"] = "12".into()).is_err());
    }

    #[test]
    fn urls_must_be_https_and_allowed() {
        assert!(with(|v| v["notes_url"] = "http://github.com/x".into()).is_err());
        assert!(with(|v| v["notes_url"] = "https://evil.example/x".into()).is_err());
        assert!(with(|v| v["ffmpeg"]["downloads"]["windows-x86_64"]["url"] = "http://github.com/x.zip".into()).is_err());
        assert!(with(|v| v["ffmpeg"]["downloads"]["windows-x86_64"]["url"] = "https://evil.example/ffmpeg.zip".into()).is_err());
        assert!(with(|v| v["ffmpeg"]["downloads"]["windows-x86_64"]["url"] = "file:///C:/ffmpeg.zip".into()).is_err());
    }

    #[test]
    fn absurd_sizes_are_refused() {
        assert!(with(|v| v["game"]["size"] = 0.into()).is_err());
        assert!(with(|v| v["game"]["size"] = (MAX_GAME_BYTES + 1).into()).is_err());
        assert!(with(|v| v["launcher"]["size"] = 10.into()).is_err());
        assert!(with(|v| v["launcher"]["size"] = (MAX_LAUNCHER_BYTES + 1).into()).is_err());
        assert!(with(|v| v["ffmpeg"]["downloads"]["windows-x86_64"]["size"] = (MAX_FFMPEG_BYTES + 1).into()).is_err());
        assert!(with(|v| v["space"]["data_bytes"] = u64::MAX.into()).is_err());
    }

    #[test]
    fn names_and_versions_are_checked() {
        assert!(with(|v| v["game"]["asset"] = "../game.zip".into()).is_err());
        assert!(with(|v| v["game"]["asset"] = "game.exe".into()).is_err());
        assert!(with(|v| v["launcher"]["asset"] = "a/b.exe".into()).is_err());
        assert!(with(|v| v["version"] = "v0.2.0".into()).is_err());
        assert!(with(|v| v["game"]["version"] = "0.2".into()).is_err());
        assert!(with(|v| v["min_launcher_version"] = "9.0.0".into()).is_err());
        assert!(with(|v| v["game"]["data_version"] = 0.into()).is_err());
        assert!(with(|v| v["ffmpeg"]["downloads"]["windows-x86_64"]["member"] = "../../evil.exe".into()).is_err());
    }

    #[test]
    fn signature_must_decode() {
        assert!(with(|v| v["launcher"]["signature"] = "".into()).is_err());
        assert!(with(|v| v["launcher"]["signature"] = "not a signature".into()).is_err());
    }

    #[test]
    fn oversized_and_malformed_text_is_refused() {
        let big = " ".repeat(MAX_MANIFEST_BYTES + 1);
        assert_eq!(ReleaseManifest::parse(&big, &cfg()).unwrap_err().kind, Kind::TooLarge);
        assert_eq!(ReleaseManifest::parse("{not json", &cfg()).unwrap_err().kind, Kind::Manifest);
        assert_eq!(ReleaseManifest::parse("[]", &cfg()).unwrap_err().kind, Kind::Manifest);
    }

    #[test]
    fn release_values_override_the_compiled_in_install_manifest() {
        let mut m = sample();
        m.game.data_version = 7;
        m.ffmpeg.version = "9.9.9".into();
        let im = m.to_install_manifest();
        assert_eq!(im.data_version, 7);
        assert_eq!(im.ffmpeg.version, "9.9.9");
        assert_eq!(im.iso.sha256, mcla_install::Manifest::embedded().iso.sha256);
    }

    #[test]
    fn the_install_crate_reads_a_release_manifest_directly() {
        let text = serde_json::to_string(&sample()).unwrap();
        let im = mcla_install::Manifest::from_json(&text).expect("install crate accepts the release manifest");
        assert_eq!(im.data_version, 1);
        assert_eq!(im.iso.title_id, "545407F8");
    }
}
