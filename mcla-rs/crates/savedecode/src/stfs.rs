//! Reader of an Xbox 360 STFS package (`CON `, `LIVE`, `PIRS`): takes one file out of it.
//!
//! A save game the console keeps is a signed (not encrypted) STFS container: a header with a volume descriptor, then 0x1000-byte blocks. Hash-table
//! blocks sit between the data blocks (one level-0 table before every 0xAA data blocks, one level-1 table after the first of those groups), and a file
//! is a run of data blocks that is either marked *consecutive* or chained through the `next block` field of the hash entries. No key is involved.
//!
//! The layout rules are the ones of the community documentation (Free60 "STFS") and of the Velocity reader (`StfsPackage::ComputeBackingDataBlockNumber`,
//! `ComputeLevel0BackingHashBlockNumber`, `GetHashAddressOfBlock`). **Not checked:** the signature and the SHA-1 hashes (a damaged block is not
//! noticed here; the save inside has its own structure checks afterwards). **Not supported:** more than 0x70E4 blocks (a game save is far smaller;
//! such a package is refused as damaged), SVOD packages (game discs).
//!
//! Every access is bounds-checked and every loop is bounded: a damaged or truncated package gives an [`Error`], never a panic (the launcher is
//! built with `panic = "abort"`).

use crate::{Error, Result};

const BLOCK: u64 = 0x1000;
/// Data blocks one level-0 hash table covers.
const PER_TABLE: u64 = 0xAA;
/// Data blocks the level-1 table covers (and the most this reader handles).
const PER_L1: u64 = 0x70E4;
const HASH_ENTRY: usize = 0x18;
const ENTRY: usize = 0x40;
const NO_PARENT: u16 = 0xFFFF;

fn bad(detail: impl Into<String>) -> Error {
    Error::new("stfs_damaged", detail)
}

fn be32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

fn le24(b: &[u8], o: usize) -> Option<u32> {
    let s = b.get(o..o.checked_add(3)?)?;
    Some(s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16)
}

fn be24(b: &[u8], o: usize) -> Option<u32> {
    let s = b.get(o..o.checked_add(3)?)?;
    Some((s[0] as u32) << 16 | (s[1] as u32) << 8 | s[2] as u32)
}

/// True when `bytes` starts like a package this reader looks at.
pub fn is_package(bytes: &[u8]) -> bool {
    matches!(bytes.get(..4), Some(b"CON " | b"LIVE" | b"PIRS"))
}

/// One entry of the file table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub directory: bool,
    pub consecutive: bool,
    pub blocks: u32,
    pub start: u32,
    /// Index of the parent folder's entry, `0xFFFF` = the root.
    pub parent: u16,
    pub size: u32,
}

struct Package<'a> {
    data: &'a [u8],
    /// 0 for the packages with one hash block per table ("female"), 1 for two ("male").
    shift: u32,
    first_hash: u64,
    allocated: u64,
    top_level: u8,
    block_sep: u8,
}

impl<'a> Package<'a> {
    fn open(data: &'a [u8]) -> Result<Package<'a>> {
        if !is_package(data) {
            return Err(Error::new("stfs_damaged", "no CON / LIVE / PIRS magic"));
        }
        if data.len() < 0x3AD {
            return Err(bad("the header is cut off"));
        }
        if be32(data, 0x3A9) != Some(0) {
            return Err(Error::new("stfs_unsupported", "the package is not an STFS file system (a game disc layout?)"));
        }
        if data[0x379] != 0x24 {
            return Err(bad("the volume descriptor has the wrong size"));
        }
        let header_size = be32(data, 0x340).ok_or_else(|| bad("no header size"))? as u64;
        let first_hash = (header_size + 0xFFF) & !0xFFF;
        let block_sep = data[0x37B];
        let allocated = be32(data, 0x395).ok_or_else(|| bad("no block count"))? as u64;
        if allocated == 0 || allocated > PER_L1 {
            return Err(bad(format!("{allocated} allocated blocks (this reader takes 1..={PER_L1})")));
        }
        let top_level = if allocated <= PER_TABLE { 0 } else { 1 };
        Ok(Package { data, shift: ((!block_sep) & 1) as u32, first_hash, allocated, top_level, block_sep })
    }

    /// Distance in blocks between level-0 tables (the data blocks plus the table).
    fn step0(&self) -> u64 {
        if self.shift == 0 { 0xAB } else { 0xAC }
    }

    fn slice(&self, at: u64, len: u64) -> Result<&'a [u8]> {
        let end = at.checked_add(len).ok_or_else(|| bad("an address overflows"))?;
        if end > self.data.len() as u64 {
            return Err(bad(format!("a block lies behind the end of the file (offset {end:#x}, file {:#x})", self.data.len())));
        }
        Ok(&self.data[at as usize..end as usize])
    }

    /// The data block's number counted with the hash tables in.
    fn backing(&self, n: u64) -> u64 {
        let base = (((n + PER_TABLE) / PER_TABLE) << self.shift) + n;
        if n < PER_TABLE {
            base
        } else {
            base + (((n + PER_L1) / PER_L1) << self.shift)
        }
    }

    fn block(&self, n: u64) -> Result<&'a [u8]> {
        if n >= self.allocated {
            return Err(bad(format!("block {n:#x} is outside the {:#x} allocated", self.allocated)));
        }
        self.slice(self.first_hash + (self.backing(n) << 12), BLOCK)
    }

    /// Address of the level-0 hash table (the active one of the pair) that holds block `n`'s entry.
    fn l0_entry_address(&self, n: u64) -> Result<u64> {
        let table = if n < PER_TABLE { 0 } else { (n / PER_TABLE) * self.step0() + (1 << self.shift) };
        let mut at = self.first_hash + (table << 12);
        if self.top_level == 0 {
            at += ((self.block_sep & 2) as u64) << 11;
        } else {
            // the level-1 table (right after the first group) says which of the two blocks of each level-0 table is the live one
            let l1 = self.first_hash + (self.step0() << 12) + (((self.block_sep & 2) as u64) << 11);
            let status = *self.slice(l1 + (n / PER_TABLE) * HASH_ENTRY as u64 + 0x14, 1)?.first().ok_or_else(|| bad("empty slice"))?;
            at += ((status & 0x40) as u64) << 6;
        }
        Ok(at + (n % PER_TABLE) * HASH_ENTRY as u64)
    }

    /// The block that follows `n` in its file (hash entry: 20 bytes SHA-1, a status byte, 24 bits next block).
    fn next(&self, n: u64) -> Result<u64> {
        if n >= self.allocated {
            return Err(bad(format!("block {n:#x} is outside the {:#x} allocated", self.allocated)));
        }
        let e = self.slice(self.l0_entry_address(n)?, HASH_ENTRY as u64)?;
        be24(e, 0x15).map(u64::from).ok_or_else(|| bad("short hash entry"))
    }

    /// `count` blocks of a file, starting at `start`: a plain run, or the chain of hash entries.
    fn blocks_of(&self, start: u64, count: u64, consecutive: bool) -> Result<Vec<u64>> {
        if count > self.allocated {
            return Err(bad("a file claims more blocks than the package has"));
        }
        let mut v = Vec::with_capacity(count as usize);
        let mut b = start;
        for i in 0..count {
            if b >= self.allocated {
                return Err(bad(format!("block {b:#x} is outside the {:#x} allocated", self.allocated)));
            }
            v.push(b);
            if i + 1 < count {
                b = if consecutive { b + 1 } else { self.next(b)? };
            }
        }
        Ok(v)
    }

    fn read_blocks(&self, blocks: &[u64], len: u64) -> Result<Vec<u8>> {
        let mut out = Vec::with_capacity(len as usize);
        for &b in blocks {
            let d = self.block(b)?;
            let take = (len - out.len() as u64).min(BLOCK) as usize;
            out.extend_from_slice(&d[..take]);
        }
        Ok(out)
    }

    fn entries(&self) -> Result<Vec<FileEntry>> {
        let count = u16::from_le_bytes([self.data[0x37C], self.data[0x37D]]) as u64;
        let start = le24(self.data, 0x37E).ok_or_else(|| bad("no file table"))? as u64;
        if count == 0 {
            return Err(bad("the file table has no blocks"));
        }
        // the file table is a file itself, chained like any other (its blocks are not marked consecutive)
        let blocks = self.blocks_of(start, count, false)?;
        let mut out = Vec::new();
        for &b in &blocks {
            let d = self.block(b)?;
            for i in 0..BLOCK as usize / ENTRY {
                let e = &d[i * ENTRY..(i + 1) * ENTRY];
                let flags = e[0x28];
                let len = (flags & 0x3F) as usize;
                if len == 0 {
                    continue;
                }
                let name: String = e[..len.min(0x28)].iter().map(|&c| if c.is_ascii() { c as char } else { '?' }).collect();
                let (Some(blocks), Some(start)) = (le24(e, 0x29), le24(e, 0x2F)) else { continue };
                out.push(FileEntry {
                    name,
                    directory: flags & 0x80 != 0,
                    consecutive: flags & 0x40 != 0,
                    blocks,
                    start,
                    parent: u16::from_be_bytes([e[0x32], e[0x33]]),
                    size: be32(e, 0x34).unwrap_or(0),
                });
            }
        }
        Ok(out)
    }
}

/// The files of the package (name, size, ...). Folders are listed too (`directory`).
pub fn list(package: &[u8]) -> Result<Vec<FileEntry>> {
    Package::open(package)?.entries()
}

/// Takes the file `name` (any case) out of the package; a file in the root is preferred over one inside a folder. `max_len` is the biggest file
/// accepted (a package that claims more is refused before anything is allocated). Errors: `stfs_damaged`, `stfs_unsupported`, `stfs_no_file`,
/// `stfs_too_big`.
pub fn extract(package: &[u8], name: &str, max_len: u64) -> Result<Vec<u8>> {
    let p = Package::open(package)?;
    let entries = p.entries()?;
    let mut hits = entries.iter().filter(|e| !e.directory && e.name.eq_ignore_ascii_case(name));
    let first = hits.next();
    let e = match first {
        Some(f) if f.parent == NO_PARENT => f,
        Some(f) => hits.find(|e| e.parent == NO_PARENT).unwrap_or(f),
        None => return Err(Error::new("stfs_no_file", format!("{name} is not in the package"))),
    };
    let size = e.size as u64;
    if size > max_len {
        return Err(Error::new("stfs_too_big", format!("{name} is {size} bytes")));
    }
    if size == 0 {
        return Ok(Vec::new());
    }
    let need = size.div_ceil(BLOCK);
    if (e.blocks as u64) < need {
        return Err(bad(format!("{name} claims {size} bytes in {} blocks", e.blocks)));
    }
    let blocks = p.blocks_of(e.start as u64, need, e.consecutive)?;
    p.read_blocks(&blocks, size)
}

/// Builder of small packages for the tests of this crate and of the launcher (not part of the API).
#[doc(hidden)]
pub mod testing {
    use super::*;

    /// One file (or folder) to put in a test package.
    #[derive(Clone)]
    pub struct File {
        pub name: String,
        pub data: Vec<u8>,
        /// Marked consecutive (a plain run of blocks); otherwise its blocks are interleaved with the other chained files and linked by hash entries.
        pub consecutive: bool,
        pub directory: bool,
        /// Slot index of the folder it lies in; `None` = the root.
        pub parent: Option<u16>,
    }

    impl File {
        pub fn new(name: &str, data: &[u8], consecutive: bool) -> File {
            File { name: name.to_string(), data: data.to_vec(), consecutive, directory: false, parent: None }
        }
    }

    pub struct Options {
        pub magic: [u8; 4],
        /// Two hash blocks per table (the console's own save packages), else one.
        pub male: bool,
        /// Male packages only: the second block of each pair is the live one (the first holds stale junk).
        pub second_active: bool,
        /// Data blocks added behind the files (not allocated to any file), to pass the 0xAA boundary.
        pub extra_blocks: u32,
    }

    impl Default for Options {
        fn default() -> Options {
            Options { magic: *b"CON ", male: true, second_active: false, extra_blocks: 0 }
        }
    }

    fn put24le(v: &mut [u8], o: usize, x: u32) {
        v[o] = x as u8;
        v[o + 1] = (x >> 8) as u8;
        v[o + 2] = (x >> 16) as u8;
    }

    /// Builds a package. The physical layout is written the long way round (walking the groups and emitting tables and blocks in order), not
    /// with the reader's address formula, so a mistake in the formula shows. At most 0x70E4 blocks.
    pub fn build(files: &[File], o: &Options) -> Vec<u8> {
        let tbl = if o.male { 2 } else { 1 };
        let header_size: u32 = if &o.magic == b"CON " { 0xAD0E } else { 0x971A };
        let first_hash = ((header_size as u64 + 0xFFF) & !0xFFF) as usize;
        // block assignment: the file table first, then consecutive files, then the chained ones interleaved
        let table_blocks = (files.len() * ENTRY).div_ceil(0x1000).max(1);
        let mut next_block = table_blocks as u32;
        let mut start = vec![0u32; files.len()];
        let mut blocks_of: Vec<Vec<u32>> = vec![Vec::new(); files.len()];
        for (i, f) in files.iter().enumerate() {
            let n = f.data.len().div_ceil(0x1000) as u32;
            if f.consecutive && !f.directory {
                start[i] = next_block;
                blocks_of[i] = (next_block..next_block + n).collect();
                next_block += n;
            }
        }
        let chained: Vec<usize> = (0..files.len()).filter(|&i| !files[i].consecutive && !files[i].directory).collect();
        let most = chained.iter().map(|&i| files[i].data.len().div_ceil(0x1000)).max().unwrap_or(0);
        for k in 0..most {
            for &i in &chained {
                if k < files[i].data.len().div_ceil(0x1000) {
                    if k == 0 {
                        start[i] = next_block;
                    }
                    blocks_of[i].push(next_block);
                    next_block += 1;
                }
            }
        }
        let allocated = next_block;
        let total = allocated + o.extra_blocks;
        assert!(total as u64 <= PER_L1, "the test builder does not do level 2");
        // physical sequence: per group of 0xAA data blocks a level-0 table (1 or 2 blocks), the level-1 table after the first group
        #[derive(Clone, Copy)]
        enum P {
            L0(u32, u8),
            L1(u8),
            Data(u32),
        }
        let groups = total.div_ceil(PER_TABLE as u32);
        let mut seq: Vec<P> = Vec::new();
        for g in 0..groups {
            for t in 0..tbl as u8 {
                seq.push(P::L0(g, t));
            }
            for n in g * 0xAA..((g + 1) * 0xAA).min(total) {
                seq.push(P::Data(n));
            }
            if g == 0 && groups > 1 {
                for t in 0..tbl as u8 {
                    seq.push(P::L1(t));
                }
            }
        }
        let mut img = vec![0u8; first_hash + seq.len() * 0x1000];
        let at = |idx: usize| first_hash + idx * 0x1000;
        let active = if o.male && o.second_active { 1u8 } else { 0 };
        // data blocks
        let mut payload: Vec<Option<Vec<u8>>> = vec![None; total as usize];
        let mut table = vec![0u8; table_blocks * 0x1000];
        let mut slot = 0usize;
        for (i, f) in files.iter().enumerate() {
            let e = &mut table[slot * ENTRY..(slot + 1) * ENTRY];
            let n = f.name.as_bytes();
            e[..n.len()].copy_from_slice(n);
            e[0x28] = n.len() as u8 | if f.consecutive { 0x40 } else { 0 } | if f.directory { 0x80 } else { 0 };
            let cnt = blocks_of[i].len() as u32;
            put24le(e, 0x29, cnt);
            put24le(e, 0x2C, cnt);
            put24le(e, 0x2F, start[i]);
            e[0x32..0x34].copy_from_slice(&f.parent.unwrap_or(0xFFFF).to_be_bytes());
            e[0x34..0x38].copy_from_slice(&(f.data.len() as u32).to_be_bytes());
            for (k, &b) in blocks_of[i].iter().enumerate() {
                let from = k * 0x1000;
                let to = (from + 0x1000).min(f.data.len());
                payload[b as usize] = Some(f.data[from..to].to_vec());
            }
            slot += 1;
        }
        for b in 0..table_blocks {
            payload[b] = Some(table[b * 0x1000..(b + 1) * 0x1000].to_vec());
        }
        // next-block links: the file table is a chain, so are the chained files
        let mut next = vec![0xFFFFFFu32; total as usize];
        for b in 0..table_blocks.saturating_sub(1) {
            next[b] = b as u32 + 1;
        }
        for (i, f) in files.iter().enumerate() {
            if f.consecutive {
                continue;
            }
            for w in blocks_of[i].windows(2) {
                next[w[0] as usize] = w[1];
            }
        }
        for (idx, p) in seq.iter().enumerate() {
            match *p {
                P::Data(n) => {
                    if let Some(d) = &payload[n as usize] {
                        img[at(idx)..at(idx) + d.len()].copy_from_slice(d);
                    }
                }
                P::L0(g, t) => {
                    let block = &mut img[at(idx)..at(idx) + 0x1000];
                    if o.male && t != active {
                        block.fill(0xFF); // the stale half of the pair
                        continue;
                    }
                    for n in g * 0xAA..((g + 1) * 0xAA).min(total) {
                        let e = &mut block[(n % 0xAA) as usize * HASH_ENTRY..][..HASH_ENTRY];
                        e[0x14] = if n < allocated { 0x80 } else { 0 };
                        let nx = next[n as usize];
                        e[0x15] = (nx >> 16) as u8;
                        e[0x16] = (nx >> 8) as u8;
                        e[0x17] = nx as u8;
                    }
                }
                P::L1(t) => {
                    let block = &mut img[at(idx)..at(idx) + 0x1000];
                    if o.male && t != active {
                        block.fill(0xFF);
                        continue;
                    }
                    for g in 0..groups {
                        block[g as usize * HASH_ENTRY + 0x14] = 0x80 | if active == 1 { 0x40 } else { 0 };
                    }
                }
            }
        }
        // header
        img[..4].copy_from_slice(&o.magic);
        img[0x340..0x344].copy_from_slice(&header_size.to_be_bytes());
        img[0x379] = 0x24;
        img[0x37B] = (if o.male { 0 } else { 1 }) | if o.male && o.second_active { 2 } else { 0 };
        img[0x37C..0x37E].copy_from_slice(&(table_blocks as u16).to_le_bytes());
        put24le(&mut img, 0x37E, 0);
        img[0x395..0x399].copy_from_slice(&allocated.to_be_bytes());
        img[0x399..0x39D].copy_from_slice(&o.extra_blocks.to_be_bytes());
        img[0x3A9..0x3AD].copy_from_slice(&0u32.to_be_bytes());
        img
    }

    /// The package with the single file `name`.
    pub fn single(name: &str, data: &[u8]) -> Vec<u8> {
        build(&[File::new(name, data, true)], &Options::default())
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn pattern(n: usize, seed: u32) -> Vec<u8> {
        (0..n as u32).map(|i| (i.wrapping_mul(2654435761).wrapping_add(seed) >> 13) as u8).collect()
    }

    const MAX: u64 = 8 << 20;

    #[test]
    fn a_file_comes_out_of_every_kind_of_package() {
        let save = pattern(537_428, 1); // the size of a real mc4.sav: 132 blocks
        for magic in [*b"CON ", *b"LIVE", *b"PIRS"] {
            for male in [true, false] {
                for second in [false, true] {
                    for consecutive in [true, false] {
                        let o = Options { magic, male, second_active: second, extra_blocks: 0 };
                        let pkg = build(&[File::new("mc4.sav", &save, consecutive), File::new("other.bin", &pattern(5000, 9), !consecutive)], &o);
                        assert_eq!(extract(&pkg, "mc4.sav", MAX).unwrap(), save, "{:?} male={male} second={second} consecutive={consecutive}", magic);
                        assert_eq!(extract(&pkg, "MC4.SAV", MAX).unwrap(), save, "the name is matched without regard to case");
                    }
                }
            }
        }
    }

    #[test]
    fn a_file_that_crosses_the_hash_table_groups_is_read_whole() {
        // filler of 300 blocks, then the save: it starts in group 1 and (with the chained one) is spread over the level-1 table's range
        let save = pattern(600_000, 3);
        for male in [true, false] {
            for consecutive in [true, false] {
                for second in [false, true] {
                    let files = [File::new("filler", &pattern(300 * 4096 - 17, 5), true), File::new("mc4.sav", &save, consecutive), File::new("tail", &pattern(40_000, 6), false)];
                    let pkg = build(&files, &Options { male, second_active: second, extra_blocks: 10, ..Options::default() });
                    assert_eq!(extract(&pkg, "mc4.sav", MAX).unwrap(), save, "male={male} consecutive={consecutive} second={second}");
                    assert_eq!(extract(&pkg, "tail", MAX).unwrap(), pattern(40_000, 6));
                    assert_eq!(extract(&pkg, "filler", MAX).unwrap(), pattern(300 * 4096 - 17, 5));
                }
            }
        }
    }

    #[test]
    fn a_file_that_straddles_the_first_group_boundary_is_read_whole() {
        // 0xAA blocks hold the table block(s) at 0 and the 169 first data blocks; a save starting at block 100 crosses 0xAA
        let save = pattern(120 * 4096, 7);
        for consecutive in [true, false] {
            let files = [File::new("a", &pattern(100 * 4096, 8), true), File::new("mc4.sav", &save, consecutive)];
            let pkg = build(&files, &Options { male: false, ..Options::default() });
            assert_eq!(extract(&pkg, "mc4.sav", MAX).unwrap(), save);
        }
    }

    #[test]
    fn the_file_table_may_span_several_blocks_and_folders_are_handled() {
        let mut files = vec![File { name: "folder".into(), data: Vec::new(), consecutive: false, directory: true, parent: None }];
        for i in 0..70 {
            files.push(File::new(&format!("f{i}"), &pattern(10 + i, i as u32), true));
        }
        // the same name in a folder and in the root: the root one wins
        files.push(File { name: "mc4.sav".into(), data: pattern(5000, 11), consecutive: true, directory: false, parent: Some(0) });
        files.push(File::new("mc4.sav", &pattern(4000, 12), true));
        let pkg = build(&files, &Options::default());
        assert_eq!(extract(&pkg, "mc4.sav", MAX).unwrap(), pattern(4000, 12));
        assert_eq!(extract(&pkg, "f69", MAX).unwrap(), pattern(79, 69));
        let l = list(&pkg).unwrap();
        assert_eq!(l.len(), 73);
        assert!(l[0].directory && l[0].name == "folder");
        // only in a folder: found there
        let only = build(&[File { name: "mc4.sav".into(), data: pattern(300, 1), consecutive: true, directory: false, parent: Some(0) }], &Options::default());
        assert_eq!(extract(&only, "mc4.sav", MAX).unwrap(), pattern(300, 1));
    }

    #[test]
    fn refusals_have_their_own_codes() {
        let pkg = single("mc4.sav", &pattern(9000, 2));
        assert_eq!(extract(&pkg, "other.sav", MAX).unwrap_err().code, "stfs_no_file");
        assert_eq!(extract(&pkg, "mc4.sav", 100).unwrap_err().code, "stfs_too_big");
        let mut svod = pkg.clone();
        svod[0x3A9..0x3AD].copy_from_slice(&1u32.to_be_bytes());
        assert_eq!(extract(&svod, "mc4.sav", MAX).unwrap_err().code, "stfs_unsupported");
        assert_eq!(extract(b"hello", "mc4.sav", MAX).unwrap_err().code, "stfs_damaged");
        assert_eq!(extract(&single("mc4.sav", &[]), "mc4.sav", MAX).unwrap(), Vec::<u8>::new());
        let mut big = pkg.clone();
        big[0x395..0x399].copy_from_slice(&0x4AF768u32.to_be_bytes());
        assert_eq!(extract(&big, "mc4.sav", MAX).unwrap_err().code, "stfs_damaged");
    }

    #[test]
    fn damaged_and_truncated_packages_never_panic() {
        let base = build(&[File::new("mc4.sav", &pattern(200_000, 4), false), File::new("x", &pattern(9000, 5), true)], &Options { extra_blocks: 3, ..Options::default() });
        // every truncation (a coarse step through the whole file, a fine one near the header and the first tables)
        let mut cuts: Vec<usize> = (0..base.len()).step_by(997).collect();
        cuts.extend((0..0x1000).chain(0xAC00..0xB200));
        for cut in cuts {
            let _ = extract(&base[..cut.min(base.len())], "mc4.sav", MAX);
            let _ = list(&base[..cut.min(base.len())]);
        }
        // damaged header, volume descriptor and file table fields: random-ish bytes written over ranges, and every byte of the descriptor flipped
        let mut state = 0x1234_5678u32;
        let mut rnd = move || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for _ in 0..3000 {
            let mut p = base.clone();
            for _ in 0..(rnd() % 6 + 1) {
                let region = rnd() % 4;
                let pos = match region {
                    0 => 0x340 + (rnd() % 0x70) as usize,
                    1 => 0x370 + (rnd() % 0x40) as usize,
                    2 => 0xB000 + (rnd() % 0x3000) as usize, // first tables and the file table
                    _ => (rnd() as usize) % p.len(),
                };
                if pos < p.len() {
                    p[pos] = rnd() as u8;
                }
            }
            let _ = extract(&p, "mc4.sav", MAX);
            let _ = extract(&p, "x", MAX);
            let _ = list(&p);
        }
        for i in 0x300..0x3B0 {
            for v in [0u8, 0xFF, 0x80, 0x7F, 1] {
                let mut p = base.clone();
                p[i] = v;
                let _ = extract(&p, "mc4.sav", MAX);
            }
        }
        // a hash chain that loops on itself is bounded by the block count: every next-block field of the first level-0 table points at block 1
        let mut looped = build(&[File::new("mc4.sav", &pattern(20_000, 4), false)], &Options { male: false, ..Options::default() });
        for off in (0xB000..0xC000).step_by(0x18) {
            if off + 0x18 <= 0xC000 {
                looped[off + 0x15..off + 0x18].copy_from_slice(&[0, 0, 1]);
            }
        }
        let _ = extract(&looped, "mc4.sav", MAX);
    }
}
