//! Save backup: copies `<user dir>/saves/mcla_save.json` to `<user dir>/backups/` with a time stamp and a reason. The save manager UI
//! (list, restore) is a later piece of work and uses this same function and naming.

use crate::error::{Kind, Result, UpdateError};
use std::path::{Path, PathBuf};

pub const SAVE_FILE: &str = "mcla_save.json";
pub const KEEP_DEFAULT: usize = 10;
const PREFIX: &str = "mcla_save-";

/// Reasons the launcher uses: `before-update`, `manual`, `before-restore`. Anything else is reduced to `[a-z0-9-]`.
fn clean_reason(r: &str) -> String {
    let s: String = r.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        "backup".into()
    } else {
        s.chars().take(40).collect()
    }
}

/// Copies the save (if there is one) to `backups/mcla_save-<yyyymmdd-hhmmss>-<reason>.json` and keeps the newest `keep` backups.
/// `Ok(None)` = there is no save to back up. The copy is written through a temporary file, so a half file is never named like a backup.
pub fn backup_save(user_dir: &Path, reason: &str, keep: usize) -> Result<Option<PathBuf>> {
    let src = user_dir.join("saves").join(SAVE_FILE);
    if !src.is_file() {
        return Ok(None);
    }
    let dir = user_dir.join("backups");
    std::fs::create_dir_all(&dir).map_err(|e| UpdateError::io(dir.display(), &e))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let reason = clean_reason(reason);
    // the counter only goes up within a second (even when the older ones of that second were pruned already), so the order of
    // the names is the order the backups were made in
    let stamp_s = stamp.to_string();
    let n = std::fs::read_dir(&dir)
        .map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| e.file_name().to_str().map(order_key)).filter(|k| k.0 == stamp_s).map(|k| k.1 + 1).max().unwrap_or(1))
        .unwrap_or(1);
    let mut dest = if n <= 1 { dir.join(format!("{PREFIX}{stamp}-{reason}.json")) } else { dir.join(format!("{PREFIX}{stamp}-{reason}-{n}.json")) };
    let mut n = n;
    while dest.exists() {
        n += 1;
        dest = dir.join(format!("{PREFIX}{stamp}-{reason}-{n}.json"));
    }
    let tmp = dest.with_extension("json.part");
    std::fs::copy(&src, &tmp).map_err(|e| UpdateError::io(format!("copy {} to {}", src.display(), tmp.display()), &e))?;
    std::fs::rename(&tmp, &dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        UpdateError::io(dest.display(), &e)
    })?;
    prune(&dir, keep.max(1));
    log::info!("save backed up to {}", dest.display());
    Ok(Some(dest))
}

/// Sort key of a backup name: the time stamp, then the counter (`-2` = the second backup of that second; a plain name is the first).
/// A plain string sort would put `...-manual-2.json` before `...-manual.json` ('-' sorts before '.') and prune the newest one.
fn order_key(name: &str) -> (String, u32) {
    let rest = name.strip_prefix(PREFIX).and_then(|r| r.strip_suffix(".json")).unwrap_or(name);
    let stamp: String = rest.chars().take(15).collect();
    let n = rest.rsplit_once('-').and_then(|(_, n)| n.parse::<u32>().ok()).filter(|_| rest.len() > 16).unwrap_or(1);
    (stamp, n)
}

fn prune(dir: &Path, keep: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            n.starts_with(PREFIX) && n.ends_with(".json")
        })
        .collect();
    files.sort_by_key(|p| order_key(p.file_name().and_then(|n| n.to_str()).unwrap_or(""))); // oldest first
    while files.len() > keep {
        let _ = std::fs::remove_file(files.remove(0));
    }
}

/// One backup as the save manager lists it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackupEntry {
    /// File name inside `backups/` (the page sends it back to restore).
    pub file: String,
    /// Unix seconds, read from the time stamp in the name (local time).
    pub unix: i64,
    /// `before-update`, `manual`, `before-restore`, `before-import` (or whatever a later version wrote).
    pub reason: String,
    pub size: u64,
}

/// Splits `mcla_save-<yyyymmdd-hhmmss>-<reason>[-<n>].json` into (unix seconds, reason).
pub fn parse_name(name: &str) -> Option<(i64, String)> {
    let rest = name.strip_prefix(PREFIX)?.strip_suffix(".json")?;
    if rest.len() < 16 || !rest.is_char_boundary(15) {
        return None;
    }
    let (stamp, tail) = rest.split_at(15);
    let dt = chrono::NaiveDateTime::parse_from_str(stamp, "%Y%m%d-%H%M%S").ok()?;
    let reason = tail.strip_prefix('-')?;
    if reason.is_empty() || !reason.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    // "manual-2" is the second backup of that reason in the same second
    let reason = match reason.rsplit_once('-') {
        Some((head, n)) if !head.is_empty() && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => head,
        _ => reason,
    };
    use chrono::TimeZone;
    let unix = chrono::Local.from_local_datetime(&dt).earliest().map(|d| d.timestamp()).unwrap_or(0);
    Some((unix, reason.to_string()))
}

/// The backups, newest first. Files that are not named like a backup are ignored.
pub fn list(user_dir: &Path) -> Vec<BackupEntry> {
    let Ok(rd) = std::fs::read_dir(user_dir.join("backups")) else { return Vec::new() };
    let mut v: Vec<((String, u32), BackupEntry)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let file = e.file_name().to_str()?.to_string();
            let (unix, reason) = parse_name(&file)?;
            let size = e.metadata().ok().filter(|m| m.is_file())?.len();
            Some((order_key(&file), BackupEntry { file, unix, reason, size }))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0)); // newest first: the time stamp, then the counter of backups made in the same second
    v.into_iter().map(|x| x.1).collect()
}

/// The path of a backup the page named. Only a plain file name that parses as a backup is accepted (no separators, no `..`).
pub fn path_of(user_dir: &Path, file: &str) -> Result<PathBuf> {
    if file.contains(['/', '\\']) || parse_name(file).is_none() {
        return Err(UpdateError::new(Kind::Missing, format!("{file:?} is not a backup of the save manager")));
    }
    let p = user_dir.join("backups").join(file);
    if !p.is_file() {
        return Err(UpdateError::new(Kind::Missing, format!("the backup {file} no longer exists")));
    }
    Ok(p)
}

/// Replaces the current save with `data`: a backup of the current one comes first (reason `reason`), a backup that cannot be made
/// stops everything, then the new bytes go through a temporary file with a flush and a rename, so a crash never leaves half a save.
/// Returns the backup that was made (`None` when there was no save).
pub fn replace_save(user_dir: &Path, data: &[u8], reason: &str, keep: usize) -> Result<Option<PathBuf>> {
    let made = backup_save(user_dir, reason, keep).map_err(|e| UpdateError::new(Kind::Io, format!("could not back up the current save, nothing was replaced: {}", e.detail)))?;
    let dest = user_dir.join("saves").join(SAVE_FILE);
    let dir = user_dir.join("saves");
    std::fs::create_dir_all(&dir).map_err(|e| UpdateError::io(dir.display(), &e))?;
    let tmp = dir.join(format!("{SAVE_FILE}.part"));
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &dest)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        UpdateError::io(format!("write {}", dest.display()), &e)
    })?;
    log::info!("save replaced ({} bytes, reason {reason}); previous save: {}", data.len(), made.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "none".into()));
    Ok(made)
}

/// Used by callers that want a typed refusal when the backup was required.
pub fn require(user_dir: &Path, reason: &str, keep: usize) -> Result<Option<PathBuf>> {
    backup_save(user_dir, reason, keep).map_err(|e| UpdateError::new(Kind::Io, format!("could not back up the save, update not started: {}", e.detail)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_with_save(content: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("saves")).unwrap();
        std::fs::write(d.path().join("saves").join(SAVE_FILE), content).unwrap();
        d
    }

    #[test]
    fn copies_the_save_with_time_and_reason() {
        let d = user_with_save("{\"a\":1}");
        let p = backup_save(d.path(), "before update", 10).unwrap().unwrap();
        let name = p.file_name().unwrap().to_str().unwrap().to_string();
        assert!(name.starts_with("mcla_save-") && name.ends_with("-before-update.json"), "{name}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "{\"a\":1}");
        // the original is untouched
        assert_eq!(std::fs::read_to_string(d.path().join("saves").join(SAVE_FILE)).unwrap(), "{\"a\":1}");
    }

    #[test]
    fn no_save_means_nothing_to_do() {
        let d = tempfile::tempdir().unwrap();
        assert!(backup_save(d.path(), "manual", 10).unwrap().is_none());
        assert!(!d.path().join("backups").exists());
    }

    #[test]
    fn keeps_only_the_newest_n() {
        let d = user_with_save("x");
        let dir = d.path().join("backups");
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..5 {
            std::fs::write(dir.join(format!("mcla_save-2020010{i}-000000-manual.json")), "old").unwrap();
        }
        std::fs::write(dir.join("notes.txt"), "not a backup").unwrap();
        backup_save(d.path(), "manual", 3).unwrap();
        let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names.iter().filter(|n| n.starts_with("mcla_save-")).count(), 3, "{names:?}");
        assert!(names.contains(&"notes.txt".to_string()));
        assert!(!names.contains(&"mcla_save-20200100-000000-manual.json".to_string()));
    }

    #[test]
    fn two_backups_in_one_second_do_not_collide() {
        let d = user_with_save("x");
        let a = backup_save(d.path(), "manual", 10).unwrap().unwrap();
        let b = backup_save(d.path(), "manual", 10).unwrap().unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn reasons_are_made_filename_safe() {
        assert_eq!(clean_reason("../../evil name"), "evil-name");
        assert_eq!(clean_reason(""), "backup");
    }

    #[test]
    fn names_parse_and_list_newest_first() {
        let (u, r) = parse_name("mcla_save-20261015-131029-before-update.json").unwrap();
        assert_eq!(r, "before-update");
        assert!(u > 1_700_000_000);
        assert_eq!(parse_name("mcla_save-20261015-131029-manual-2.json").unwrap().1, "manual");
        assert!(parse_name("mcla_save-notadate-manual.json").is_none());
        assert!(parse_name("../mcla_save-20261015-131029-manual.json").is_none());
        let d = user_with_save("{}");
        let dir = d.path().join("backups");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mcla_save-20200101-000000-manual.json"), "aa").unwrap();
        std::fs::write(dir.join("mcla_save-20210101-000000-before-update.json"), "bbbb").unwrap();
        std::fs::write(dir.join("readme.txt"), "x").unwrap();
        let l = list(d.path());
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].reason, "before-update");
        assert_eq!(l[0].size, 4);
    }

    #[test]
    fn path_of_refuses_anything_but_a_backup_name() {
        let d = user_with_save("{}");
        let p = backup_save(d.path(), "manual", 10).unwrap().unwrap();
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(path_of(d.path(), name).is_ok());
        assert!(path_of(d.path(), r"..\saves\mcla_save.json").is_err());
        assert!(path_of(d.path(), "mcla_save-20200101-000000-manual.json").is_err()); // does not exist
    }

    #[test]
    fn replace_save_backs_up_first_and_leaves_no_part_file() {
        let d = user_with_save("{\"old\":1}");
        let made = replace_save(d.path(), b"{\"new\":2}", "before-import", 10).unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(&made).unwrap(), "{\"old\":1}");
        assert_eq!(std::fs::read_to_string(d.path().join("saves").join(SAVE_FILE)).unwrap(), "{\"new\":2}");
        assert!(!d.path().join("saves").join("mcla_save.json.part").exists());
        assert!(made.file_name().unwrap().to_str().unwrap().ends_with("-before-import.json"));
        // no save yet: nothing to back up, the file is simply created
        let e = tempfile::tempdir().unwrap();
        assert!(replace_save(e.path(), b"{}", "before-import", 10).unwrap().is_none());
        assert!(e.path().join("saves").join(SAVE_FILE).is_file());
    }

    #[test]
    fn backups_of_the_same_second_keep_their_order() {
        let d = user_with_save("0");
        let mut made = Vec::new();
        for i in 1..=5 {
            std::fs::write(d.path().join("saves").join(SAVE_FILE), format!("v{i}")).unwrap();
            made.push(backup_save(d.path(), "manual", 3).unwrap().unwrap());
        }
        // the newest three survive, whatever the second they fell in, and they list newest first
        let l = list(d.path());
        assert_eq!(l.len(), 3, "{l:?}");
        let texts: Vec<String> = l.iter().map(|b| std::fs::read_to_string(d.path().join("backups").join(&b.file)).unwrap()).collect();
        assert_eq!(texts, vec!["v5", "v4", "v3"], "{l:?}");
    }
}
