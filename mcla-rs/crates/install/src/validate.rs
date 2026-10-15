//! ISO validation (LAUNCHER.md "Install pipeline", step 2): fast checks that fail early with a friendly error, and the
//! background full SHA-256 that only warns.

use crate::error::{ErrorKind, InstallError, IoCtx, Result};
use crate::manifest::Manifest;
use crate::progress::{Cancel, CheckStatus, Ctx, Stage, Unit};
use mcla_formats::iso::{Xiso, SECTOR};
use mcla_formats::rpf3::Rpf3;
use mcla_formats::ReadAt;
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Known game-partition base offsets (XGD2, XGD3, XGD1, raw), as in `mcla_formats::iso`.
const BASES: [u64; 4] = [0x0FD9_0000, 0x0208_0000, 0x1830_0000, 0];
const MAGIC: &[u8; 20] = b"MICROSOFT*XBOX*MEDIA";

/// What the validation learned about the image. Serialised into `manifest.local.json` (as `iso`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IsoIdentity {
    pub file_name: String,
    pub size: u64,
    pub partition_base: u64,
    pub title_id: String,
    /// Game regions word of `default.xex` (informational).
    pub region_mask: String,
    pub rpf_entries: u32,
    /// Full SHA-256, once computed.
    pub sha256: Option<String>,
    /// "verified" | "unrecognised" | "not_checked"
    pub dump: String,
}

/// A validated, opened disc: everything later stages need.
pub struct Disc {
    pub path: PathBuf,
    pub iso: Xiso,
    pub key: [u8; 32],
    pub identity: IsoIdentity,
    pub warnings: Vec<String>,
}

fn kind_check(ctx: &Ctx, id: &str, r: Result<String>) -> Result<String> {
    match &r {
        Ok(t) => ctx.check_line(Stage::Validate, id, CheckStatus::Pass, t.clone()),
        Err(e) => ctx.check_line(Stage::Validate, id, CheckStatus::Fail, e.detail.clone()),
    }
    r
}

/// Reject what is plainly not a plain disc image, by name and by the first bytes.
fn sniff(path: &Path, head: &[u8]) -> Result<()> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let comp_ext = ["cso", "ciso", "zso", "zip", "7z", "rar", "gz", "xz", "bz2", "zst", "chd", "wbfs", "nkit"];
    if comp_ext.contains(&ext.as_str()) {
        return Err(InstallError::new(ErrorKind::CompressedImage, format!("file extension .{ext}")));
    }
    let h4 = &head[..head.len().min(4)];
    let compressed: [(&[u8], &str); 8] = [(b"PK\x03\x04", "zip"), (b"7z\xBC\xAF", "7z"), (b"Rar!", "rar"), (b"\x1f\x8b", "gzip"), (b"CISO", "cso"), (b"ZISO", "zso"), (b"BZh", "bzip2"), (b"MCom", "chd")];
    for (m, name) in compressed {
        if head.starts_with(m) {
            return Err(InstallError::new(ErrorKind::CompressedImage, format!("starts like a {name} file")));
        }
    }
    if ext == "xex" || h4 == b"XEX2" {
        return Err(InstallError::new(ErrorKind::NotAnImage, "a .xex executable, not a disc image"));
    }
    if matches!(h4, b"LIVE" | b"PIRS" | b"CON ") {
        return Err(InstallError::new(ErrorKind::NotAnImage, "a Games on Demand / STFS package, not a disc image"));
    }
    Ok(())
}

/// Find the game partition base by reading the volume descriptor directly (no mapping, so a bad file cannot fault).
fn find_base(f: &mut std::fs::File, len: u64) -> std::result::Result<u64, ErrorKind> {
    let mut truncated = false;
    for b in BASES {
        let off = b + 32 * SECTOR;
        if off + SECTOR > len {
            continue;
        }
        let mut hdr = [0u8; 28];
        if f.seek(SeekFrom::Start(off)).is_err() || f.read_exact(&mut hdr).is_err() {
            continue;
        }
        if &hdr[..20] == MAGIC {
            let root_sec = u32::from_le_bytes([hdr[20], hdr[21], hdr[22], hdr[23]]) as u64;
            let root_size = u32::from_le_bytes([hdr[24], hdr[25], hdr[26], hdr[27]]) as u64;
            // the root directory must lie inside the file, or the walk would read past the mapping
            if root_size > 0 && root_size < (64 << 20) {
                if b + root_sec * SECTOR + root_size <= len {
                    return Ok(b);
                }
                truncated = true;
            }
        }
    }
    Err(if truncated { ErrorKind::Truncated } else { ErrorKind::NotXbox360 })
}

/// Title id and region word of a `default.xex` header (big-endian XEX2 layout). `None` if the header is not understood.
fn xex_ids(head: &[u8]) -> Option<(u32, u32)> {
    let be32 = |o: usize| -> Option<u32> { head.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])) };
    if head.get(0..4)? != b"XEX2" {
        return None;
    }
    let sec = be32(16)? as usize;
    let n = be32(20)? as usize;
    let mut title = None;
    for i in 0..n.min(256) {
        if be32(24 + i * 8)? == 0x0004_0006 {
            let off = be32(28 + i * 8)? as usize;
            title = be32(off + 12);
        }
    }
    let region = be32(sec + 0x178).unwrap_or(0);
    Some((title?, region))
}

fn guarded<T>(what: &str, f: impl FnOnce() -> T) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|_| InstallError::new(ErrorKind::NotXbox360, format!("{what}: the image structure is damaged")))
}

impl Disc {
    /// Run the fast checks, emitting a checklist line per check. Errors are friendly kinds; warnings do not stop.
    pub fn open(path: &Path, m: &Manifest, ctx: &Ctx) -> Result<Disc> {
        let spec = &m.iso;
        let mut warnings = Vec::new();
        ctx.message(Stage::Validate, format!("Checking {}", path.display()));
        // 1. readable, and not obviously something else
        kind_check(
            ctx,
            "readable",
            (|| {
                let md = std::fs::metadata(path).map_err(|e| InstallError::io(path.display(), &e))?;
                if md.is_dir() {
                    return Err(InstallError::new(ErrorKind::NotAnImage, "this is a folder"));
                }
                let mut f = std::fs::File::open(path).ctx(path.display())?;
                let mut head = [0u8; 16];
                let n = f.read(&mut head).ctx("read the start of the image")?;
                sniff(path, &head[..n])?;
                Ok(format!("{} bytes", md.len()))
            })(),
        )?;
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        // 2. layout
        let mut f = std::fs::File::open(path).ctx(path.display())?;
        let base = find_base(&mut f, len);
        kind_check(
            ctx,
            "layout",
            base.map(|b| format!("Xbox 360 disc layout, game partition at {b:#x}")).map_err(|k| {
                let k = if k == ErrorKind::NotXbox360 && len < 0x0208_0000 + 33 * SECTOR { ErrorKind::Truncated } else { k };
                InstallError::new(k, if k == ErrorKind::Truncated { "the image ends before the disc's directory" } else { "no Xbox disc volume descriptor at any known offset" })
            }),
        )?;
        let base = base.unwrap_or(0);
        drop(f);
        let iso = guarded("read the disc directory", || Xiso::open(path)).and_then(|r| r.map_err(|e| InstallError::new(ErrorKind::NotXbox360, format!("{e:#}"))));
        let iso = match iso {
            Ok(i) => i,
            Err(e) => {
                ctx.check_line(Stage::Validate, "directory", CheckStatus::Fail, e.detail.clone());
                return Err(e);
            }
        };
        // 3. required files, inside the image
        let missing_game = iso.find("default.xex").is_none();
        let r: Result<String> = (|| {
            if missing_game {
                return Err(InstallError::new(ErrorKind::NotXbox360, "default.xex is not on the disc"));
            }
            for name in &spec.required_files {
                let Some(e) = iso.find(name) else {
                    let kind = if name == &spec.game_archive { ErrorKind::WrongGame } else { ErrorKind::MissingFile };
                    return Err(InstallError::new(kind, format!("{name} is not on the disc")));
                };
                if e.offset + e.size > len {
                    return Err(InstallError::new(ErrorKind::Truncated, format!("{name} ends past the end of the image")));
                }
            }
            Ok(format!("{} required files present", spec.required_files.len()))
        })();
        kind_check(ctx, "required_files", r)?;
        // 4. size (a different size with all files present only warns: padding differs between rips)
        if len == spec.size {
            ctx.check_line(Stage::Validate, "size", CheckStatus::Pass, format!("{len} bytes, as expected"));
        } else {
            let w = format!("{len} bytes (expected {}): another rip of the disc?", spec.size);
            ctx.check_line(Stage::Validate, "size", CheckStatus::Warn, w.clone());
            warnings.push(w);
        }
        // 5. the executable: is it this game?
        let xe = iso.find("default.xex").ok_or_else(|| InstallError::new(ErrorKind::NotXbox360, "default.xex"))?;
        let head_len = xe.size.min(0x10000) as usize;
        let head = iso.file("default.xex").and_then(|s| s.read_vec(0, head_len)).map_err(|e| InstallError::new(ErrorKind::Unreadable, format!("{e:#}")));
        let (title_id, region) = match head.as_ref().ok().and_then(|h| xex_ids(h)) {
            Some(x) => x,
            None => {
                let e = InstallError::new(ErrorKind::NotXbox360, "default.xex is not a readable XEX2 executable");
                ctx.check_line(Stage::Validate, "executable", CheckStatus::Fail, e.detail.clone());
                return Err(e);
            }
        };
        let title_hex = format!("{title_id:08X}");
        if !spec.title_id.is_empty() && !spec.title_id.eq_ignore_ascii_case(&title_hex) {
            let e = InstallError::new(ErrorKind::WrongGame, format!("title id {title_hex}, expected {}", spec.title_id));
            ctx.check_line(Stage::Validate, "executable", CheckStatus::Fail, e.detail.clone());
            return Err(e);
        }
        ctx.check_line(Stage::Validate, "executable", CheckStatus::Pass, format!("default.xex, title id {title_hex}"));
        ctx.check()?;
        // 6. the archive key: the table must decrypt and parse
        let key = guarded("derive the archive key", || mcla_formats::keys::derive(&iso)).and_then(|r| r.map_err(|e| InstallError::new(ErrorKind::UnsupportedEdition, format!("{e:#}"))));
        let key = match key {
            Ok(k) => {
                ctx.check_line(Stage::Validate, "archive_key", CheckStatus::Pass, "archive key found in default.xex, the table decrypts");
                k
            }
            Err(e) => {
                ctx.check_line(Stage::Validate, "archive_key", CheckStatus::Fail, e.detail.clone());
                return Err(e);
            }
        };
        // 7. entry count
        let rpf = iso.file(&spec.game_archive).map_err(|e| InstallError::new(ErrorKind::MissingFile, format!("{e:#}")))?;
        let entries = match guarded("read the archive table", || Rpf3::open(Arc::new(rpf), &key)).and_then(|r| r.map_err(|e| InstallError::new(ErrorKind::UnsupportedEdition, format!("{e:#}")))) {
            Ok(r) => r.entries.len() as u32,
            Err(e) => {
                ctx.check_line(Stage::Validate, "entries", CheckStatus::Fail, e.detail.clone());
                return Err(e);
            }
        };
        if entries != spec.rpf_entries {
            let e = InstallError::new(ErrorKind::UnsupportedEdition, format!("{} has {entries} entries, expected {}", spec.game_archive, spec.rpf_entries));
            ctx.check_line(Stage::Validate, "entries", CheckStatus::Fail, e.detail.clone());
            return Err(e);
        }
        ctx.check_line(Stage::Validate, "entries", CheckStatus::Pass, format!("{entries} archive entries"));
        let identity = IsoIdentity {
            file_name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            size: len,
            partition_base: base,
            title_id: title_hex,
            region_mask: format!("{region:08X}"),
            rpf_entries: entries,
            sha256: None,
            dump: "not_checked".into(),
        };
        Ok(Disc { path: path.to_path_buf(), iso, key, identity, warnings })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DumpState {
    Verified,
    Unrecognised,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HashResult {
    pub sha256: String,
    pub state: DumpState,
}

/// Full SHA-256 of the image with progress by bytes (stage `Checksum`). A mismatch is `Unrecognised`, never an error.
pub fn hash_iso(path: &Path, m: &Manifest, ctx: &Ctx) -> Result<HashResult> {
    hash_iso_with_stop(path, m, ctx, None)
}

/// As [`hash_iso`], and also stops (with a cancelled error) when `stop` is set: the pipeline uses it to end the background hash.
pub fn hash_iso_with_stop(path: &Path, m: &Manifest, ctx: &Ctx, stop: Option<&Cancel>) -> Result<HashResult> {
    let mut f = std::fs::File::open(path).ctx(path.display())?;
    let total = f.metadata().ctx(path.display())?.len();
    let counter = ctx.counter(Stage::Checksum, Unit::Bytes, total);
    counter.set_message("Checking the disc image checksum");
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 4 << 20];
    loop {
        let n = f.read(&mut buf).ctx("read the image")?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        counter.add(n as u64)?;
        if let Some(s) = stop {
            s.check()?;
        }
    }
    counter.finish();
    let hex = hex(&h.finalize());
    let state = if hex.eq_ignore_ascii_case(&m.iso.sha256) { DumpState::Verified } else { DumpState::Unrecognised };
    Ok(HashResult { sha256: hex, state })
}

pub(crate) fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silent() -> Ctx {
        Ctx::silent()
    }

    #[test]
    fn folder_is_not_an_image() {
        let m = Manifest::embedded();
        let e = Disc::open(&std::env::temp_dir(), &m, &silent()).err().unwrap();
        assert_eq!(e.kind, ErrorKind::NotAnImage);
    }

    #[test]
    fn missing_file_is_unreadable() {
        let m = Manifest::embedded();
        let e = Disc::open(Path::new("Z:/definitely/not/here.iso"), &m, &silent()).err().unwrap();
        assert_eq!(e.kind, ErrorKind::Unreadable);
    }

    #[test]
    fn compressed_by_extension_and_magic() {
        let dir = std::env::temp_dir().join(format!("mcla_inst_t1_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let m = Manifest::embedded();
        let a = dir.join("x.cso");
        std::fs::write(&a, b"CISO....").unwrap();
        assert_eq!(Disc::open(&a, &m, &silent()).err().unwrap().kind, ErrorKind::CompressedImage);
        let b = dir.join("y.iso");
        std::fs::write(&b, b"PK\x03\x04rest").unwrap();
        assert_eq!(Disc::open(&b, &m, &silent()).err().unwrap().kind, ErrorKind::CompressedImage);
        let c = dir.join("z.iso");
        std::fs::write(&c, b"XEX2\0\0\0\0").unwrap();
        assert_eq!(Disc::open(&c, &m, &silent()).err().unwrap().kind, ErrorKind::NotAnImage);
        let d = dir.join("small.iso");
        std::fs::write(&d, vec![0u8; 5000]).unwrap();
        assert_eq!(Disc::open(&d, &m, &silent()).err().unwrap().kind, ErrorKind::Truncated);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn xex_ids_parse() {
        let mut h = vec![0u8; 0x400];
        h[0..4].copy_from_slice(b"XEX2");
        h[16..20].copy_from_slice(&0x200u32.to_be_bytes());
        h[20..24].copy_from_slice(&1u32.to_be_bytes());
        h[24..28].copy_from_slice(&0x0004_0006u32.to_be_bytes());
        h[28..32].copy_from_slice(&0x100u32.to_be_bytes());
        h[0x100 + 12..0x100 + 16].copy_from_slice(&0x5454_07F2u32.to_be_bytes());
        h[0x200 + 0x178..0x200 + 0x17C].copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
        assert_eq!(xex_ids(&h), Some((0x5454_07F2, 0xFFFF_FFFF)));
        assert_eq!(xex_ids(&h[..10]), None);
    }
}
