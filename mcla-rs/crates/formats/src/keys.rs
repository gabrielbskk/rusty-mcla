//! The archive / script key is not stored anywhere in this project. It is found in the user's own
//! disc: the key is a 32-byte constant inside the game's executable (`default.xex`), and the right one is
//! the 32-byte window of that executable that decrypts the table of contents of `xarchive_cache.rpf`
//! into a plausible table.

use crate::crypto::decrypt_toc;
use crate::iso::Xiso;
use crate::{bail, Context, ReadAt, Result};
use rayon::prelude::*;

fn le32(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// Is a decrypted 16-byte table entry a plausible entry of an archive of `count` entries and `alen` bytes?
fn plausible(e: &[u8], count: u32, alen: u64) -> bool {
    let (b, c, d) = (le32(e, 4), le32(e, 8), le32(e, 12));
    if c & 0x8000_0000 != 0 && d & 0xC000_0000 == 0 {
        // directory: first child and child count inside the table
        return (c & 0x7FFF_FFFF) <= count && d <= count;
    }
    if d & 0x8000_0000 != 0 {
        return (c & !0xFFF) as u64 + b as u64 <= alen;
    }
    if d & 0x4000_0000 != 0 {
        return c as u64 + (d & 0x3FFF_FFFF) as u64 <= alen;
    }
    c as u64 + b as u64 <= alen
}

/// Find the key in a decrypted executable `image` by testing every window against the archive's encrypted table.
pub fn find_in_image(image: &[u8], rpf: &impl ReadAt) -> Result<[u8; 32]> {
    let hdr = rpf.read_vec(0, 0x20).context("archive header")?;
    if &hdr[0..4] != b"RPF3" {
        bail!("the archive is not an RPF3 file");
    }
    let toc_size = le32(&hdr, 4) as usize;
    let count = le32(&hdr, 8);
    let alen = rpf.len();
    if (count as usize) * 16 > toc_size || count < 8 || image.len() < 32 {
        bail!("the archive table is not valid");
    }
    let enc_toc = rpf.read_vec(0x800, toc_size).context("archive table")?;
    let first: [u8; 128] = enc_toc[..128].try_into().unwrap();
    let candidates: Vec<usize> = (0..image.len() - 32)
        .into_par_iter()
        .filter(|&o| {
            let w = &image[o..o + 32];
            let mut seen = [false; 256];
            let mut distinct = 0;
            for &b in w {
                if !seen[b as usize] {
                    seen[b as usize] = true;
                    distinct += 1;
                }
            }
            if distinct < 20 {
                return false;
            }
            let key: [u8; 32] = w.try_into().unwrap();
            let mut buf = first;
            decrypt_toc(&mut buf, &key);
            le32(&buf, 8) & 0x8000_0000 != 0 && buf.chunks_exact(16).all(|e| plausible(e, count, alen))
        })
        .collect();
    let mut found = None;
    for o in candidates {
        let key: [u8; 32] = image[o..o + 32].try_into().unwrap();
        let mut toc = enc_toc.clone();
        decrypt_toc(&mut toc, &key);
        if (0..count as usize).all(|i| plausible(&toc[i * 16..i * 16 + 16], count, alen)) {
            if found.is_some() {
                bail!("more than one possible key in the executable");
            }
            found = Some(key);
        }
    }
    found.ok_or_else(|| crate::anyhow!("no key in the executable fits the archive"))
}

/// Derive the key from the user's disc image.
pub fn derive(iso: &Xiso) -> Result<[u8; 32]> {
    let xe = iso.find("default.xex").context("the disc image has no default.xex: is this the right game?")?;
    let raw = iso.file("default.xex")?.read_vec(0, xe.size as usize)?;
    let image = crate::xex::image(&raw).context("cannot read default.xex")?;
    let rpf = iso.file("xarchive_cache.rpf").context("the disc image has no xarchive_cache.rpf: is this the right game?")?;
    find_in_image(&image, &rpf).context("this does not look like the supported disc (Midnight Club: Los Angeles, Complete Edition, Xbox 360)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::{BlockEncrypt, KeyInit};

    struct Mem(Vec<u8>);
    impl ReadAt for Mem {
        fn len(&self) -> u64 {
            self.0.len() as u64
        }
        fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<()> {
            buf.copy_from_slice(&self.0[off as usize..off as usize + buf.len()]);
            Ok(())
        }
    }

    fn test_key() -> [u8; 32] {
        std::array::from_fn(|i| ((i * 37 + 11) % 251) as u8)
    }

    /// A small archive: header, then a table of 16 entries (a root directory and 15 files) encrypted with `key`.
    fn archive(key: &[u8; 32]) -> Mem {
        let mut toc = Vec::new();
        for i in 0..16u32 {
            let e: [u32; 4] = if i == 0 { [0x1234, 1, 0x8000_0001, 15] } else { [0x100 + i, 64, 0x1000 + i * 64, 0] };
            for w in e {
                toc.extend_from_slice(&w.to_le_bytes());
            }
        }
        let c = aes::Aes256::new(key.into());
        for _ in 0..16 {
            for chunk in toc.chunks_exact_mut(16) {
                c.encrypt_block(chunk.into());
            }
        }
        let mut a = vec![0u8; 0x800];
        a[0..4].copy_from_slice(b"RPF3");
        a[4..8].copy_from_slice(&(toc.len() as u32).to_le_bytes());
        a[8..12].copy_from_slice(&16u32.to_le_bytes());
        a[0x10..0x14].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        a.extend_from_slice(&toc);
        a.resize(0x4000, 0);
        Mem(a)
    }

    #[test]
    fn finds_the_key_inside_an_image() {
        let key = test_key();
        let mut image = vec![0u8; 4096];
        image[1000..1032].copy_from_slice(&key);
        assert_eq!(find_in_image(&image, &archive(&key)).unwrap(), key);
    }

    #[test]
    fn no_key_in_the_image_is_an_error() {
        let image = vec![0u8; 4096];
        assert!(find_in_image(&image, &archive(&test_key())).is_err());
    }
}
