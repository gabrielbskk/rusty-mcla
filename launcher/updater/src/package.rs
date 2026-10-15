//! The game package: `game-<ver>.zip` (the game program and its small files) installed into `<portable root>/game/`.
//!
//! Order of events, so that nothing is touched before everything is checked:
//! 1. download (or take a local file), size and SHA-256 checked against the manifest;
//! 2. extract into `game.new/` with zip-slip protection and size limits (a failure here deletes `game.new/` and stops);
//! 3. write `game.new/package.json` (what is installed) and `game.new/manifest.json` (the release's manifest; the install pipeline
//!    uses it so the built data always matches the installed game);
//! 4. back up the save when this is an update;
//! 5. swap: `game/` becomes `game.old/`, `game.new/` becomes `game/`; if the second rename fails the first is undone.
//!
//! `game.old/` stays until the next package is installed (or until [`cleanup_after_start`] is called once the game has started well).

use crate::error::{Kind, Result, UpdateError};
use crate::http::{sha256_file, Http};
use crate::manifest::ReleaseManifest;
use crate::releases::Release;
use crate::{backup, now_unix, Cancel, Phase, Progress, ProgressFn};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

pub const GAME_DIR: &str = "game";
pub const NEW_DIR: &str = "game.new";
pub const OLD_DIR: &str = "game.old";
pub const TEMP_DIR: &str = ".update";
pub const RECORD_FILE: &str = "package.json";
pub const MANIFEST_COPY: &str = "manifest.json";
pub const GAME_EXE: &str = "mcla.exe";

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_entries: usize,
    pub max_total: u64,
    pub max_file: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_entries: 5000, max_total: 4 * 1024 * 1024 * 1024, max_file: 2 * 1024 * 1024 * 1024 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageRecord {
    pub version: String,
    pub sha256: String,
    pub size: u64,
    pub data_version: u32,
    pub installed_unix: i64,
    /// `download` or `local`
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct Options {
    /// Back up the save before swapping an existing game folder.
    pub backup_save: bool,
    pub keep_backups: usize,
    pub limits: Limits,
}

impl Default for Options {
    fn default() -> Self {
        Options { backup_save: true, keep_backups: backup::KEEP_DEFAULT, limits: Limits::default() }
    }
}

pub fn game_dir(root: &Path) -> PathBuf {
    root.join(GAME_DIR)
}

pub fn read_record(root: &Path) -> Option<PackageRecord> {
    let t = std::fs::read(game_dir(root).join(RECORD_FILE)).ok()?;
    serde_json::from_slice(&t).ok()
}

/// The release manifest saved with the installed game (validated again; a manifest that no longer validates is ignored).
pub fn read_manifest_copy(root: &Path, cfg: &crate::config::ReleaseConfig) -> Option<ReleaseManifest> {
    let t = std::fs::read_to_string(game_dir(root).join(MANIFEST_COPY)).ok()?;
    ReleaseManifest::parse(&t, cfg).ok()
}

fn zip_err(detail: impl Into<String>) -> UpdateError {
    UpdateError::new(Kind::Zip, detail)
}

const RESERVED: [&str; 22] = ["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];

/// The relative path an archive entry may be extracted to, or an error. Rejects absolute paths, drive letters, `..`, alternate data
/// streams (`:`), device names, trailing dots / spaces and control characters. Backslashes count as separators.
pub fn safe_entry_path(name: &str) -> Result<PathBuf> {
    if name.is_empty() || name.len() > 260 || name.contains('\0') {
        return Err(zip_err(format!("bad entry name: {}", name.chars().take(60).collect::<String>())));
    }
    let n = name.replace('\\', "/");
    if n.starts_with('/') || n.contains(':') {
        return Err(zip_err(format!("absolute or drive path in archive: {}", name.chars().take(60).collect::<String>())));
    }
    let mut out = PathBuf::new();
    let mut depth = 0;
    for comp in n.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            return Err(zip_err(format!("path traversal in archive: {}", name.chars().take(60).collect::<String>())));
        }
        if comp.ends_with('.') || comp.ends_with(' ') || comp.chars().any(|c| c.is_control() || "<>\"|?*".contains(c)) {
            return Err(zip_err(format!("unusable file name in archive: {}", comp.chars().take(60).collect::<String>())));
        }
        let stem = comp.split('.').next().unwrap_or("").to_ascii_lowercase();
        if RESERVED.contains(&stem.as_str()) {
            return Err(zip_err(format!("device name in archive: {comp}")));
        }
        out.push(comp);
        depth += 1;
        if depth > 32 {
            return Err(zip_err("path nested too deeply"));
        }
    }
    if out.as_os_str().is_empty() {
        return Err(zip_err("empty entry name"));
    }
    // belt and braces: nothing but normal components
    if out.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(zip_err("path is not a plain relative path"));
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extracted {
    pub files: usize,
    pub bytes: u64,
}

/// Extracts `zip_path` into the (new, empty) folder `dest`. Everything is validated first, then written; the declared size of each
/// entry is a hard cap while writing, so a lying archive cannot fill the disk.
pub fn extract_zip(zip_path: &Path, dest: &Path, limits: &Limits, cancel: &Cancel, progress: &dyn Fn(u64, u64)) -> Result<Extracted> {
    let file = std::fs::File::open(zip_path).map_err(|e| UpdateError::io(zip_path.display(), &e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| zip_err(format!("not a readable zip: {e}")))?;
    if archive.len() > limits.max_entries {
        return Err(UpdateError::new(Kind::TooLarge, format!("the package has {} entries (limit {})", archive.len(), limits.max_entries)));
    }
    // pass 1: names and sizes
    let mut total: u64 = 0;
    let mut seen = std::collections::HashSet::new();
    let mut plan: Vec<(usize, PathBuf, bool, u64)> = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let f = archive.by_index(i).map_err(|e| zip_err(format!("entry {i}: {e}")))?;
        let rel = safe_entry_path(f.name())?;
        if let Some(mode) = f.unix_mode() {
            let kind = mode & 0o170000;
            if kind == 0o120000 {
                return Err(zip_err(format!("symbolic link in archive: {}", rel.display())));
            }
            if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                return Err(zip_err(format!("special file in archive: {}", rel.display())));
            }
        }
        let key = rel.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            return Err(zip_err(format!("the same path twice in the archive: {}", rel.display())));
        }
        let is_dir = f.is_dir();
        let size = f.size();
        if !is_dir {
            if size > limits.max_file {
                return Err(UpdateError::new(Kind::TooLarge, format!("{} is {size} bytes (limit {})", rel.display(), limits.max_file)));
            }
            total = total.saturating_add(size);
            if total > limits.max_total {
                return Err(UpdateError::new(Kind::TooLarge, format!("the package unpacks to more than {} bytes", limits.max_total)));
            }
        }
        plan.push((i, rel, is_dir, size));
    }
    if !plan.iter().any(|(_, rel, dir, _)| !*dir && rel.to_string_lossy().eq_ignore_ascii_case(GAME_EXE)) {
        return Err(zip_err(format!("the package has no {GAME_EXE} at its top level")));
    }
    // pass 2: write
    std::fs::create_dir_all(dest).map_err(|e| UpdateError::io(dest.display(), &e))?;
    let mut done: u64 = 0;
    let mut files = 0usize;
    progress(0, total);
    for (i, rel, is_dir, size) in plan {
        if cancel.is_cancelled() {
            return Err(UpdateError::new(Kind::Cancelled, "cancelled"));
        }
        let target = dest.join(&rel);
        if !target.starts_with(dest) {
            return Err(zip_err("entry would leave the destination folder"));
        }
        if is_dir {
            std::fs::create_dir_all(&target).map_err(|e| UpdateError::io(target.display(), &e))?;
            continue;
        }
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p).map_err(|e| UpdateError::io(p.display(), &e))?;
        }
        let mut f = archive.by_index(i).map_err(|e| zip_err(format!("entry {}: {e}", rel.display())))?;
        let mut out = std::fs::File::create(&target).map_err(|e| UpdateError::io(target.display(), &e))?;
        let mut buf = vec![0u8; 256 * 1024];
        let mut written: u64 = 0;
        loop {
            let n = f.read(&mut buf).map_err(|e| zip_err(format!("{}: {e}", rel.display())))?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > size {
                return Err(zip_err(format!("{} is larger than it says (zip bomb?)", rel.display())));
            }
            out.write_all(&buf[..n]).map_err(|e| UpdateError::io(target.display(), &e))?;
            done += n as u64;
            if cancel.is_cancelled() {
                return Err(UpdateError::new(Kind::Cancelled, "cancelled"));
            }
        }
        if written != size {
            return Err(zip_err(format!("{} is shorter than it says", rel.display())));
        }
        progress(done, total);
        files += 1;
    }
    progress(total, total);
    Ok(Extracted { files, bytes: done })
}

/// Moves `game.new/` into place. `game/` becomes `game.old/` (an older `game.old/` is deleted first). If putting the new folder in place
/// fails, the old one is put back and the error is returned: either the swap happened completely or nothing changed.
pub fn swap_in(root: &Path) -> Result<()> {
    let (game, new, old) = (root.join(GAME_DIR), root.join(NEW_DIR), root.join(OLD_DIR));
    if old.exists() {
        std::fs::remove_dir_all(&old).map_err(|e| UpdateError::io(format!("remove {}", old.display()), &e))?;
    }
    let had_game = game.exists();
    if had_game {
        std::fs::rename(&game, &old).map_err(|e| UpdateError::io(format!("the game folder is in use (is the game running?) {}", game.display()), &e))?;
    }
    if let Err(e) = std::fs::rename(&new, &game) {
        if had_game {
            if let Err(e2) = std::fs::rename(&old, &game) {
                log::error!("rollback failed: could not put {} back: {e2}", old.display());
            }
        }
        return Err(UpdateError::io(format!("could not put the new game folder in place {}", new.display()), &e));
    }
    Ok(())
}

/// Puts `game.old/` back as `game/` (the failed new one is kept as `game.bad/`). For a manual "go back".
pub fn restore_previous(root: &Path) -> Result<()> {
    let (game, old, bad) = (root.join(GAME_DIR), root.join(OLD_DIR), root.join("game.bad"));
    if !old.is_dir() {
        return Err(UpdateError::new(Kind::Missing, "there is no previous game folder"));
    }
    if bad.exists() {
        let _ = std::fs::remove_dir_all(&bad);
    }
    if game.exists() {
        std::fs::rename(&game, &bad).map_err(|e| UpdateError::io(game.display(), &e))?;
    }
    std::fs::rename(&old, &game).map_err(|e| UpdateError::io(old.display(), &e))
}

/// To be called once the game has started successfully after an update: the previous game folder is no longer needed.
pub fn cleanup_after_start(root: &Path) {
    let old = root.join(OLD_DIR);
    if old.exists() {
        match std::fs::remove_dir_all(&old) {
            Ok(()) => log::info!("removed {}", old.display()),
            Err(e) => log::warn!("could not remove {}: {e}", old.display()),
        }
    }
}

/// Removes leftovers of an interrupted update (`game.new/`, `.update/`). Safe at every start: nothing in them is ever in use.
pub fn clean_stale(root: &Path) {
    for d in [NEW_DIR, TEMP_DIR] {
        let p = root.join(d);
        if p.exists() {
            match std::fs::remove_dir_all(&p) {
                Ok(()) => log::info!("removed leftover {}", p.display()),
                Err(e) => log::warn!("could not remove leftover {}: {e}", p.display()),
            }
        }
    }
}

fn tick(progress: &ProgressFn, phase: Phase, done: u64, total: u64) {
    progress(Progress { phase, done, total });
}

/// Installs a package file that has already been downloaded (or chosen by the user). Blocking: run it on a worker thread.
/// `source` is stored in `package.json` (`download` / `local`).
pub fn install_from_zip(root: &Path, user_dir: &Path, zip: &Path, m: &ReleaseManifest, source: &str, opts: &Options, cancel: &Cancel, progress: &ProgressFn) -> Result<PackageRecord> {
    tick(progress, Phase::VerifyGame, 0, m.game.size);
    let (hash, size) = sha256_file(zip, Some(cancel))?;
    if size != m.game.size {
        return Err(UpdateError::new(Kind::Size, format!("the package is {size} bytes, the manifest says {}", m.game.size)));
    }
    if !hash.eq_ignore_ascii_case(&m.game.sha256) {
        return Err(UpdateError::new(Kind::Hash, format!("package sha256 is {hash}, the manifest says {}", m.game.sha256)));
    }
    tick(progress, Phase::VerifyGame, size, size);

    let new = root.join(NEW_DIR);
    if new.exists() {
        std::fs::remove_dir_all(&new).map_err(|e| UpdateError::io(new.display(), &e))?;
    }
    let result = (|| -> Result<PackageRecord> {
        let p2 = progress.clone();
        extract_zip(zip, &new, &opts.limits, cancel, &move |d, t| tick(&p2, Phase::ExtractGame, d, t))?;
        let record = PackageRecord { version: m.game.version.clone(), sha256: hash.clone(), size, data_version: m.game.data_version, installed_unix: now_unix(), source: source.to_string() };
        let write = |name: &str, data: Vec<u8>| std::fs::write(new.join(name), data).map_err(|e| UpdateError::io(new.join(name).display(), &e));
        write(RECORD_FILE, serde_json::to_vec_pretty(&record).map_err(|e| UpdateError::new(Kind::Io, e.to_string()))?)?;
        write(MANIFEST_COPY, serde_json::to_vec_pretty(m).map_err(|e| UpdateError::new(Kind::Io, e.to_string()))?)?;
        if cancel.is_cancelled() {
            return Err(UpdateError::new(Kind::Cancelled, "cancelled"));
        }
        // an update (not a first install): the save is backed up before anything is replaced
        if opts.backup_save && game_dir(root).exists() {
            tick(progress, Phase::Backup, 0, 1);
            backup::require(user_dir, "before-update", opts.keep_backups)?;
            tick(progress, Phase::Backup, 1, 1);
        }
        tick(progress, Phase::SwapGame, 0, 1);
        swap_in(root)?;
        tick(progress, Phase::SwapGame, 1, 1);
        Ok(record)
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&new);
    }
    result
}

/// Downloads the game package of `release` (verified while it streams), then installs it.
pub async fn download_and_install(http: &Http, release: &Release, m: &ReleaseManifest, root: &Path, user_dir: &Path, opts: &Options, cancel: &Cancel, progress: &ProgressFn) -> Result<PackageRecord> {
    let asset = release.asset(&m.game.asset).ok_or_else(|| UpdateError::new(Kind::Missing, format!("release {} has no {}", release.tag, m.game.asset)))?;
    if asset.size != 0 && asset.size != m.game.size {
        return Err(UpdateError::new(Kind::Size, format!("the release lists {} as {} bytes, the manifest says {}", asset.name, asset.size, m.game.size)));
    }
    let tmp_dir = root.join(TEMP_DIR);
    let tmp = tmp_dir.join(format!("{}.part", m.game.asset));
    let p = progress.clone();
    let dl = http
        .download(&asset.url, &tmp, m.game.size, &m.game.sha256, cancel, &move |d, t| tick(&p, Phase::DownloadGame, d, t))
        .await;
    if let Err(e) = dl {
        let _ = std::fs::remove_dir_all(&tmp_dir);
        return Err(e);
    }
    let (root2, user2, tmp2, m2, opts2, cancel2, progress2) = (root.to_path_buf(), user_dir.to_path_buf(), tmp.clone(), m.clone(), opts.clone(), cancel.clone(), progress.clone());
    let r = tokio::task::spawn_blocking(move || install_from_zip(&root2, &user2, &tmp2, &m2, "download", &opts2, &cancel2, &progress2))
        .await
        .unwrap_or_else(|e| Err(UpdateError::new(Kind::Io, format!("installer thread failed: {e}"))));
    let _ = std::fs::remove_dir_all(&tmp_dir);
    r
}

/// Finds which known manifest describes a package file the user chose: its size and SHA-256 must equal a candidate's `game` entry.
/// Candidates come from `manifest.json` beside the file, the cached latest release and the installed game's own copy.
pub fn identify_local(zip: &Path, candidates: &[ReleaseManifest], cancel: &Cancel) -> Result<ReleaseManifest> {
    let (hash, size) = sha256_file(zip, Some(cancel))?;
    if candidates.is_empty() {
        return Err(UpdateError::new(Kind::UnknownPackage, "no manifest.json is known to check this package against; put the release's manifest.json in the same folder"));
    }
    candidates
        .iter()
        .find(|m| m.game.size == size && m.game.sha256.eq_ignore_ascii_case(&hash))
        .cloned()
        .ok_or_else(|| UpdateError::new(Kind::UnknownPackage, format!("this file (sha256 {hash}) is not the game package of any release manifest that is available")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::sample;

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opt = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            w.start_file(*name, opt).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn entry_names_are_checked() {
        for bad in ["../evil.txt", "a/../../evil.txt", "..\\evil.txt", "/abs.txt", "\\abs.txt", "C:/x.txt", "C:\\x.txt", "a/b:stream", "con.txt", "nul", "aux/x", "a/b./c", "a/b /c", "x\0y", "", "a/<b>", "a/b?"] {
            assert!(safe_entry_path(bad).is_err(), "{bad:?} must be refused");
        }
        for good in ["mcla.exe", "data/a.bin", "data\\a.bin", "./x/y.txt", "dir/"] {
            assert!(safe_entry_path(good).is_ok(), "{good:?} must be accepted");
        }
        assert_eq!(safe_entry_path("data\\sub/a.bin").unwrap(), PathBuf::from("data").join("sub").join("a.bin"));
    }

    #[test]
    fn zip_slip_is_rejected_and_nothing_is_written() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("evil.zip");
        make_zip(&z, &[("mcla.exe", b"MZ"), ("../escaped.txt", b"gotcha")]);
        let dest = d.path().join("out").join("game.new");
        let e = extract_zip(&z, &dest, &Limits::default(), &Cancel::new(), &|_, _| {}).unwrap_err();
        assert_eq!(e.kind, Kind::Zip, "{e}");
        assert!(!d.path().join("out").join("escaped.txt").exists());
        assert!(!dest.exists(), "validation happens before the first byte is written");
    }

    #[test]
    fn absolute_and_backslash_traversal_entries_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        for name in ["/etc/x", "C:\\Windows\\x.dll", "a\\..\\..\\x"] {
            let z = d.path().join("e.zip");
            make_zip(&z, &[("mcla.exe", b"MZ"), (name, b"x")]);
            assert_eq!(extract_zip(&z, &d.path().join("o"), &Limits::default(), &Cancel::new(), &|_, _| {}).unwrap_err().kind, Kind::Zip, "{name}");
        }
    }

    #[test]
    fn symlinks_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("l.zip");
        let f = std::fs::File::create(&z).unwrap();
        let mut w = zip::ZipWriter::new(f);
        w.start_file("mcla.exe", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(b"MZ").unwrap();
        w.add_symlink("link", "/etc/passwd", zip::write::SimpleFileOptions::default()).unwrap();
        w.finish().unwrap();
        let e = extract_zip(&z, &d.path().join("o"), &Limits::default(), &Cancel::new(), &|_, _| {}).unwrap_err();
        assert_eq!(e.kind, Kind::Zip);
        assert!(e.detail.contains("symbolic"));
    }

    #[test]
    fn size_and_count_limits_apply() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("big.zip");
        make_zip(&z, &[("mcla.exe", &[7u8; 5000]), ("a.bin", &[1u8; 5000])]);
        let l = Limits { max_entries: 5000, max_total: 8000, max_file: 1 << 30 };
        assert_eq!(extract_zip(&z, &d.path().join("o1"), &l, &Cancel::new(), &|_, _| {}).unwrap_err().kind, Kind::TooLarge);
        let l = Limits { max_entries: 5000, max_total: 1 << 30, max_file: 4000 };
        assert_eq!(extract_zip(&z, &d.path().join("o2"), &l, &Cancel::new(), &|_, _| {}).unwrap_err().kind, Kind::TooLarge);
        let l = Limits { max_entries: 1, max_total: 1 << 30, max_file: 1 << 30 };
        assert_eq!(extract_zip(&z, &d.path().join("o3"), &l, &Cancel::new(), &|_, _| {}).unwrap_err().kind, Kind::TooLarge);
    }

    #[test]
    fn a_package_without_the_game_exe_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("n.zip");
        make_zip(&z, &[("readme.txt", b"hi")]);
        let e = extract_zip(&z, &d.path().join("o"), &Limits::default(), &Cancel::new(), &|_, _| {}).unwrap_err();
        assert_eq!(e.kind, Kind::Zip);
        assert!(e.detail.contains("mcla.exe"));
    }

    #[test]
    fn duplicate_paths_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("dup.zip");
        make_zip(&z, &[("mcla.exe", b"MZ"), ("A.txt", b"1"), ("a.TXT", b"2")]);
        assert_eq!(extract_zip(&z, &d.path().join("o"), &Limits::default(), &Cancel::new(), &|_, _| {}).unwrap_err().kind, Kind::Zip);
    }

    #[test]
    fn a_good_package_extracts_with_nested_folders() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("ok.zip");
        make_zip(&z, &[("mcla.exe", b"MZ-new"), ("data/shaders/a.bin", b"abc"), ("data\\b.bin", b"def")]);
        let out = d.path().join("out");
        let r = extract_zip(&z, &out, &Limits::default(), &Cancel::new(), &|_, _| {}).unwrap();
        assert_eq!(r.files, 3);
        assert_eq!(std::fs::read(out.join("data").join("shaders").join("a.bin")).unwrap(), b"abc");
        assert_eq!(std::fs::read(out.join("data").join("b.bin")).unwrap(), b"def");
    }

    fn manifest_for(zip: &Path) -> ReleaseManifest {
        let (h, s) = sha256_file(zip, None).unwrap();
        let mut m = sample();
        m.game.sha256 = h;
        m.game.size = s;
        m
    }

    fn root_with_old_game() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("game")).unwrap();
        std::fs::write(d.path().join("game").join("mcla.exe"), "OLD").unwrap();
        d
    }

    #[test]
    fn install_swaps_keeps_game_old_and_backs_up_the_save() {
        let d = root_with_old_game();
        let user = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(user.path().join("saves")).unwrap();
        std::fs::write(user.path().join("saves").join("mcla_save.json"), "{\"s\":1}").unwrap();
        let z = d.path().join("pkg.zip");
        let noise: Vec<u8> = (0..4000u32).map(|i| (i.wrapping_mul(2654435761) >> 13) as u8).collect(); // does not compress: the zip stays above the manifest's minimum size
        make_zip(&z, &[("mcla.exe", b"NEW"), ("data.bin", &noise)]);
        let m = manifest_for(&z);
        let rec = install_from_zip(d.path(), user.path(), &z, &m, "local", &Options::default(), &Cancel::new(), &crate::no_progress()).unwrap();
        assert_eq!(rec.version, "0.2.0");
        assert_eq!(std::fs::read(d.path().join("game").join("mcla.exe")).unwrap(), b"NEW");
        assert_eq!(std::fs::read(d.path().join("game.old").join("mcla.exe")).unwrap(), b"OLD");
        assert!(!d.path().join("game.new").exists());
        assert_eq!(read_record(d.path()).unwrap(), rec);
        assert!(read_manifest_copy(d.path(), &crate::manifest::tests::cfg()).is_some());
        let backups: Vec<_> = std::fs::read_dir(user.path().join("backups")).unwrap().collect();
        assert_eq!(backups.len(), 1);
        // a second update replaces game.old
        let z2 = d.path().join("pkg2.zip");
        make_zip(&z2, &[("mcla.exe", b"NEWER")]);
        let m2 = manifest_for(&z2);
        install_from_zip(d.path(), user.path(), &z2, &m2, "local", &Options::default(), &Cancel::new(), &crate::no_progress()).unwrap();
        assert_eq!(std::fs::read(d.path().join("game.old").join("mcla.exe")).unwrap(), b"NEW");
        cleanup_after_start(d.path());
        assert!(!d.path().join("game.old").exists());
    }

    #[test]
    fn wrong_hash_or_size_touches_nothing() {
        let d = root_with_old_game();
        let user = tempfile::tempdir().unwrap();
        let z = d.path().join("pkg.zip");
        make_zip(&z, &[("mcla.exe", b"NEW")]);
        let mut m = manifest_for(&z);
        m.game.sha256 = "c".repeat(64);
        let e = install_from_zip(d.path(), user.path(), &z, &m, "local", &Options::default(), &Cancel::new(), &crate::no_progress()).unwrap_err();
        assert_eq!(e.kind, Kind::Hash);
        let mut m = manifest_for(&z);
        m.game.size += 1;
        assert_eq!(install_from_zip(d.path(), user.path(), &z, &m, "local", &Options::default(), &Cancel::new(), &crate::no_progress()).unwrap_err().kind, Kind::Size);
        assert_eq!(std::fs::read(d.path().join("game").join("mcla.exe")).unwrap(), b"OLD");
        assert!(!d.path().join("game.new").exists() && !d.path().join("game.old").exists());
        assert!(!user.path().join("backups").exists());
    }

    #[test]
    fn a_failing_extraction_leaves_game_untouched() {
        let d = root_with_old_game();
        let user = tempfile::tempdir().unwrap();
        let z = d.path().join("evil.zip");
        make_zip(&z, &[("mcla.exe", b"NEW"), ("../x", b"1")]);
        let m = manifest_for(&z);
        assert_eq!(install_from_zip(d.path(), user.path(), &z, &m, "local", &Options::default(), &Cancel::new(), &crate::no_progress()).unwrap_err().kind, Kind::Zip);
        assert_eq!(std::fs::read(d.path().join("game").join("mcla.exe")).unwrap(), b"OLD");
        assert!(!d.path().join("game.new").exists());
    }

    #[test]
    fn the_swap_rolls_back_when_the_second_rename_fails() {
        let d = root_with_old_game();
        // no game.new at all: the second rename fails after game/ was moved to game.old/
        let e = swap_in(d.path()).unwrap_err();
        assert_eq!(e.kind, Kind::Io);
        assert_eq!(std::fs::read(d.path().join("game").join("mcla.exe")).unwrap(), b"OLD", "the old game is back in place");
        assert!(!d.path().join("game.old").exists());
    }

    #[test]
    fn first_install_has_no_save_backup_and_no_game_old() {
        let d = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(user.path().join("saves")).unwrap();
        std::fs::write(user.path().join("saves").join("mcla_save.json"), "x").unwrap();
        let z = d.path().join("pkg.zip");
        make_zip(&z, &[("mcla.exe", b"NEW")]);
        let m = manifest_for(&z);
        install_from_zip(d.path(), user.path(), &z, &m, "local", &Options::default(), &Cancel::new(), &crate::no_progress()).unwrap();
        assert!(d.path().join("game").join("mcla.exe").is_file());
        assert!(!d.path().join("game.old").exists());
        assert!(!user.path().join("backups").exists(), "nothing to protect on a first install");
    }

    #[test]
    fn a_save_that_cannot_be_backed_up_stops_the_update() {
        let d = root_with_old_game();
        let user = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(user.path().join("saves")).unwrap();
        std::fs::write(user.path().join("saves").join("mcla_save.json"), "x").unwrap();
        std::fs::write(user.path().join("backups"), "a file where the folder should be").unwrap();
        let z = d.path().join("pkg.zip");
        make_zip(&z, &[("mcla.exe", b"NEW")]);
        let m = manifest_for(&z);
        let e = install_from_zip(d.path(), user.path(), &z, &m, "local", &Options::default(), &Cancel::new(), &crate::no_progress()).unwrap_err();
        assert_eq!(e.kind, Kind::Io);
        assert_eq!(std::fs::read(d.path().join("game").join("mcla.exe")).unwrap(), b"OLD");
    }

    #[test]
    fn local_packages_are_identified_by_hash() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("pkg.zip");
        make_zip(&z, &[("mcla.exe", b"NEW")]);
        let m = manifest_for(&z);
        let other = sample();
        assert_eq!(identify_local(&z, &[other.clone(), m.clone()], &Cancel::new()).unwrap().game.sha256, m.game.sha256);
        assert_eq!(identify_local(&z, &[other], &Cancel::new()).unwrap_err().kind, Kind::UnknownPackage);
        assert_eq!(identify_local(&z, &[], &Cancel::new()).unwrap_err().kind, Kind::UnknownPackage);
    }

    #[test]
    fn stale_folders_are_cleaned() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("game.new")).unwrap();
        std::fs::create_dir_all(d.path().join(".update")).unwrap();
        std::fs::create_dir_all(d.path().join("game")).unwrap();
        clean_stale(d.path());
        assert!(!d.path().join("game.new").exists() && !d.path().join(".update").exists());
        assert!(d.path().join("game").exists());
    }
}
