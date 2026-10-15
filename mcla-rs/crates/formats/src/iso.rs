//! Xbox 360 disc images: locate the XDVDFS game partition and walk its
//! binary-tree directories.

use crate::bin::{u16le, u32le};
use crate::{bail, Context, ReadAt, Result};
use std::path::Path;
use std::sync::Arc;

pub const SECTOR: u64 = 2048;
const MAGIC: &[u8; 20] = b"MICROSOFT*XBOX*MEDIA";
/// Known game-partition base offsets (XGD2, XGD3, XGD1, raw).
const BASES: [u64; 4] = [0x0FD9_0000, 0x0208_0000, 0x1830_0000, 0];

#[derive(Debug, Clone)]
pub struct XEntry {
    pub path: String,
    /// Absolute byte offset inside the image.
    pub offset: u64,
    pub size: u64,
    pub is_dir: bool,
}

pub struct Xiso {
    pub map: Arc<memmap2::Mmap>,
    pub base: u64,
    pub entries: Vec<XEntry>,
}

impl Xiso {
    pub fn open(path: &Path) -> Result<Self> {
        let f = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
        // SAFETY: the image is treated as read-only; concurrent external modification is a user error.
        let map = unsafe { memmap2::Mmap::map(&f)? };
        let map = Arc::new(map);
        let mut base = None;
        for b in BASES {
            let off = b + 32 * SECTOR;
            if off + 20 <= map.len() as u64 && &map[off as usize..off as usize + 20] == MAGIC {
                base = Some(b);
                break;
            }
        }
        let base = base.ok_or_else(|| crate::anyhow!("no XDVDFS magic found"))?;
        let hdr = &map[(base + 32 * SECTOR) as usize..(base + 33 * SECTOR) as usize];
        let root_sec = u32le(hdr, 0x14) as u64;
        let root_size = u32le(hdr, 0x18) as u64;
        let mut entries = Vec::new();
        let mut me = Xiso { map, base, entries: Vec::new() };
        me.walk(root_sec, root_size, "", 0, &mut entries)?;
        me.entries = entries;
        Ok(me)
    }

    fn walk(&self, sec: u64, size: u64, path: &str, depth: u32, out: &mut Vec<XEntry>) -> Result<()> {
        if size == 0 {
            return Ok(());
        }
        let start = (self.base + sec * SECTOR) as usize;
        let data = &self.map[start..start + size as usize];
        self.rec(data, 0, path, depth, out)
    }

    fn rec(&self, data: &[u8], off: usize, path: &str, depth: u32, out: &mut Vec<XEntry>) -> Result<()> {
        if off + 14 > data.len() {
            return Ok(());
        }
        let l = u16le(data, off) as usize;
        let r = u16le(data, off + 2) as usize;
        if l == 0xFFFF && r == 0xFFFF {
            return Ok(());
        }
        let sec = u32le(data, off + 4) as u64;
        let sz = u32le(data, off + 8) as u64;
        let attr = data[off + 12];
        let nl = data[off + 13] as usize;
        let name = String::from_utf8_lossy(&data[off + 14..off + 14 + nl]).to_string();
        if l != 0 {
            self.rec(data, l * 4, path, depth, out)?;
        }
        let p = format!("{}/{}", path, name);
        if attr & 0x10 != 0 {
            out.push(XEntry { path: p.clone(), offset: self.base + sec * SECTOR, size: sz, is_dir: true });
            if depth < 16 {
                self.walk(sec, sz, &p, depth + 1, out)?;
            }
        } else {
            out.push(XEntry { path: p, offset: self.base + sec * SECTOR, size: sz, is_dir: false });
        }
        if r != 0 {
            self.rec(data, r * 4, path, depth, out)?;
        }
        Ok(())
    }

    pub fn find(&self, name: &str) -> Option<&XEntry> {
        let n = name.to_ascii_lowercase();
        self.entries
            .iter()
            .find(|e| !e.is_dir && (e.path.to_ascii_lowercase() == n || e.path.to_ascii_lowercase() == format!("/{}", n)))
    }

    /// A [`ReadAt`] window over a file inside the image.
    pub fn file(&self, name: &str) -> Result<crate::SubRange<memmap2::Mmap>> {
        let e = self.find(name).ok_or_else(|| crate::anyhow!("{} not in image", name))?;
        if e.offset + e.size > self.map.len() as u64 {
            bail!("entry {} outside image", name);
        }
        Ok(crate::SubRange { inner: self.map.clone(), base: e.offset, len: e.size })
    }
}
