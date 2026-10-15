//! The save container (big endian): u32 version (the Xbox 360 writes 14, the PS3 15), u32 entry count (9), then per entry u32 name length,
//! name, u8 data version, u32 data size, data. 0xCD bytes are uninitialised padding.

use crate::util::B;
use crate::{Error, Result};

pub struct Entry {
    pub name: String,
    pub version: u8,
    pub size: usize,
    /// Offset of the data in the file.
    pub off: usize,
}

pub struct Container<'a> {
    pub data: &'a [u8],
    pub version: u32,
    pub entries: Vec<Entry>,
}

/// The entries every save has, with their sizes (identical in every real save).
pub const REQUIRED: &[(&str, usize)] = &[
    ("Stats", 16908),
    ("LocalOptions", 254032),
    ("EditorData", 42256),
    ("ColorData", 160),
    ("UserVinylData", 17292),
    ("RubberbandMgr", 200),
    ("Career", 200000),
    ("RaceFavorites", 6400),
    ("enableGps", 1),
];

/// True when `head` (the first bytes of a file) starts like a plain save of the original: version 14 (Xbox 360) or 15 (PS3), the entry count 9,
/// the first entry's name length 5 and its name `Stat..`.
pub fn looks_like_original(head: &[u8]) -> bool {
    let b = B(head);
    head.len() >= 16 && matches!(b.u32(0), 14 | 15) && b.u32(4) == 9 && b.u32(8) == 5 && &head[12..16] == b"Stat"
}

impl<'a> Container<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Container<'a>> {
        let b = B(data);
        if data.len() < 8 {
            return Err(Error::new("short", "the file is too short for a save"));
        }
        let version = b.u32(0);
        if !matches!(version, 14 | 15) {
            return Err(Error::new("version", format!("save version {version}; this reader knows 14 (Xbox 360) and 15 (PS3)")));
        }
        let count = b.u32(4) as usize;
        if count != REQUIRED.len() {
            return Err(Error::new("layout", format!("{count} entries, expected {}", REQUIRED.len())));
        }
        let mut off = 8usize;
        let mut entries = Vec::new();
        for _ in 0..count {
            let nl = b.u32(off) as usize;
            if nl > 64 || off + 4 + nl + 5 > data.len() {
                return Err(Error::new("layout", "damaged entry table"));
            }
            let name = crate::util::latin1(b.slice(off + 4, nl));
            let version = b.u8(off + 4 + nl);
            let size = b.u32(off + 4 + nl + 1) as usize;
            let start = off + 4 + nl + 5;
            if start.checked_add(size).map(|e| e > data.len()).unwrap_or(true) {
                return Err(Error::new("short", format!("entry {name} runs past the end of the file")));
            }
            entries.push(Entry { name, version, size, off: start });
            off = start + size;
        }
        for (name, size) in REQUIRED {
            match entries.iter().find(|e| e.name == *name) {
                None => return Err(Error::new("layout", format!("entry {name} is missing"))),
                Some(e) if e.size != *size => return Err(Error::new("layout", format!("entry {name} is {} bytes, expected {size}", e.size))),
                _ => {}
            }
        }
        Ok(Container { data, version, entries })
    }

    pub fn entry(&self, name: &str) -> &Entry {
        // (`parse` checked that every required entry exists)
        self.entries.iter().find(|e| e.name == name).unwrap_or(&self.entries[0])
    }

    /// The data of a required entry.
    pub fn data(&self, name: &str) -> &'a [u8] {
        let e = self.entry(name);
        &self.data[e.off..e.off + e.size]
    }
}
