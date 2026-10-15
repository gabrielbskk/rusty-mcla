//! Preflight (LAUNCHER.md "Install pipeline", step 3): free space, write permission, long paths, a sensible folder.

use crate::error::{ErrorKind, InstallError, Result};
use crate::layout::Layout;
use crate::manifest::Manifest;
use crate::progress::{CheckStatus, Ctx, Stage};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreflightInfo {
    pub root: PathBuf,
    pub free_bytes: u64,
    /// What the build still needs (the full size minus what is already built).
    pub required_bytes: u64,
    pub already_built_bytes: u64,
}

/// The nearest existing ancestor of `p` (the drive's free space is asked there).
fn existing_ancestor(p: &Path) -> PathBuf {
    let mut cur = p.to_path_buf();
    while !cur.exists() {
        if !cur.pop() {
            return PathBuf::from(".");
        }
    }
    cur
}

pub fn free_space(path: &Path) -> Option<u64> {
    fs2::available_space(existing_ancestor(path)).ok()
}

fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Ok(ft) = e.file_type() {
                if ft.is_dir() {
                    total += dir_size(&e.path());
                } else if let Ok(m) = e.metadata() {
                    total += m.len();
                }
            }
        }
    }
    total
}

fn same_or_inside(path: &Path, dir: &Path) -> bool {
    let a = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let b = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    a.starts_with(b)
}

/// Check that `root` can take the install. Emits checklist lines (ids `space`, `writable`, `path_length`, `folder`).
pub fn preflight(root: &Path, iso: Option<&Path>, m: &Manifest, ctx: &Ctx) -> Result<PreflightInfo> {
    let l = Layout::new(root);
    ctx.message(Stage::Preflight, format!("Checking {}", root.display()));
    let fail = |id: &str, e: InstallError| -> InstallError {
        ctx.check_line(Stage::Preflight, id, CheckStatus::Fail, e.detail.clone());
        e
    };
    // folder
    if root.is_file() {
        return Err(fail("folder", InstallError::new(ErrorKind::BadFolder, "this is a file, not a folder")));
    }
    if let Some(iso) = iso {
        for sub in [l.data(), l.root.join("tools")] {
            if sub.exists() && same_or_inside(iso, &sub) {
                return Err(fail("folder", InstallError::new(ErrorKind::BadFolder, format!("the disc image is inside {}", sub.display()))));
            }
        }
    }
    ctx.check_line(Stage::Preflight, "folder", CheckStatus::Pass, root.display().to_string());
    // writable
    std::fs::create_dir_all(root).map_err(|e| fail("writable", InstallError::io(root.display(), &e)))?;
    let probe = root.join(format!(".write_test_{}", std::process::id()));
    std::fs::write(&probe, b"x").map_err(|e| {
        let mut er = InstallError::io(root.display(), &e);
        if er.kind != ErrorKind::DiskSpace {
            er.kind = ErrorKind::NotWritable;
        }
        fail("writable", er)
    })?;
    let _ = std::fs::remove_file(&probe);
    ctx.check_line(Stage::Preflight, "writable", CheckStatus::Pass, "the folder can be written to");
    // path length (Windows tools such as ffmpeg need paths under MAX_PATH)
    let abs = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let shown = abs.to_string_lossy().trim_start_matches(r"\\?\").to_string();
    let longest = shown.len() + "/data/".len() + m.space.max_rel_path as usize;
    if cfg!(windows) && longest > 255 {
        return Err(fail("path_length", InstallError::new(ErrorKind::PathTooLong, format!("{longest} characters needed, limit 255"))));
    }
    ctx.check_line(Stage::Preflight, "path_length", CheckStatus::Pass, format!("longest file path about {longest} characters"));
    // space
    let built = dir_size(&l.data());
    let required = m.required_free_bytes().saturating_sub(built);
    let free = free_space(root).unwrap_or(u64::MAX);
    if free < required {
        return Err(fail("space", InstallError::new(ErrorKind::DiskSpace, format!("{} MB free, {} MB needed", free / 1_000_000, required / 1_000_000))));
    }
    ctx.check_line(Stage::Preflight, "space", CheckStatus::Pass, format!("{:.1} GB free, {:.1} GB needed", free as f64 / 1e9, required as f64 / 1e9));
    Ok(PreflightInfo { root: root.to_path_buf(), free_bytes: free, required_bytes: required, already_built_bytes: built })
}

/// Only the numbers, for the UI to show as the user picks a folder.
pub fn space_for(root: &Path, m: &Manifest) -> (u64, u64) {
    let built = dir_size(&Layout::new(root).data());
    (free_space(root).unwrap_or(0), m.required_free_bytes().saturating_sub(built))
}
