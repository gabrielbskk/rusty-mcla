//! XEX2 executables (`default.xex`): decrypt and expand the PE image.
//!
//! Only used to read the game's own executable on the user's disc (see [`crate::keys`]). The XEX file key is wrapped
//! with the Xbox 360 retail key, a published console constant that cannot be derived.

use crate::{bail, Result};
use aes::cipher::{BlockDecrypt, KeyInit};

/// Xbox 360 retail XEX key: a public console constant (it wraps the file key of every retail XEX and is published
/// with the open-source Xbox 360 emulators). It is not a game key and cannot be derived. Kept on purpose (project
/// decision 2026-10-15, docs/LAUNCHER.md); the game's own archive/script key is never stored, see [`crate::keys`].
const RETAIL_XEX_KEY: [u8; 16] = [0x20, 0xB1, 0x85, 0xA5, 0x9D, 0x28, 0xFD, 0xC3, 0x40, 0x58, 0x3F, 0xBB, 0x08, 0x96, 0xBF, 0x91];

fn be32(d: &[u8], o: usize) -> Result<u32> {
    match d.get(o..o + 4) {
        Some(b) => Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]])),
        None => bail!("xex truncated"),
    }
}

fn be16(d: &[u8], o: usize) -> Result<u16> {
    match d.get(o..o + 2) {
        Some(b) => Ok(u16::from_be_bytes([b[0], b[1]])),
        None => bail!("xex truncated"),
    }
}

/// Decrypted and expanded PE image of an XEX2 file (encryption none/normal, compression none/basic/LZX).
pub fn image(d: &[u8]) -> Result<Vec<u8>> {
    if d.get(0..4) != Some(b"XEX2") {
        bail!("not an XEX2 file");
    }
    let pe_off = be32(d, 8)? as usize;
    let sec = be32(d, 16)? as usize;
    let nhdr = be32(d, 20)? as usize;
    let mut fmt = 0usize;
    for i in 0..nhdr {
        if be32(d, 24 + i * 8)? == 0x0000_03ff {
            fmt = be32(d, 28 + i * 8)? as usize;
        }
    }
    if fmt == 0 || pe_off > d.len() {
        bail!("xex has no file format info");
    }
    let image_size = be32(d, sec + 4)? as usize;
    let enc = be16(d, fmt + 4)?;
    let comp = be16(d, fmt + 6)?;
    let mut data = d[pe_off..].to_vec();
    if enc != 0 {
        let Some(wrapped) = d.get(sec + 0x150..sec + 0x160) else { bail!("xex truncated") };
        let mut blk = aes::Block::from(<[u8; 16]>::try_from(wrapped).unwrap());
        aes::Aes128::new(&RETAIL_XEX_KEY.into()).decrypt_block(&mut blk);
        let session = aes::Aes128::new(&blk);
        // CBC, zero IV
        let mut prev = [0u8; 16];
        for chunk in data.chunks_exact_mut(16) {
            let cur: [u8; 16] = (&*chunk).try_into().unwrap();
            let mut b = aes::Block::from(cur);
            session.decrypt_block(&mut b);
            for k in 0..16 {
                chunk[k] = b[k] ^ prev[k];
            }
            prev = cur;
        }
    }
    match comp {
        0 => Ok(data),
        1 => {
            // basic: (data size, zero size) pairs
            let n = (be32(d, fmt)? as usize).saturating_sub(8) / 8;
            let mut img = Vec::with_capacity(image_size);
            let mut p = 0usize;
            for i in 0..n {
                let (ds, zs) = (be32(d, fmt + 8 + i * 8)? as usize, be32(d, fmt + 12 + i * 8)? as usize);
                let Some(src) = data.get(p..p + ds) else { bail!("xex basic block outside the data") };
                img.extend_from_slice(src);
                img.resize(img.len() + zs, 0);
                p += ds;
            }
            Ok(img)
        }
        2 => {
            let window = be32(d, fmt + 8)?;
            let mut block_size = be32(d, fmt + 12)? as usize;
            let ws = match window {
                0x8000 => lzxd::WindowSize::KB32,
                0x10000 => lzxd::WindowSize::KB64,
                0x20000 => lzxd::WindowSize::KB128,
                0x40000 => lzxd::WindowSize::KB256,
                0x80000 => lzxd::WindowSize::KB512,
                0x100000 => lzxd::WindowSize::MB1,
                0x200000 => lzxd::WindowSize::MB2,
                _ => bail!("xex lzx window {:#x}", window),
            };
            let mut lzx = lzxd::Lzxd::new(ws);
            let mut img: Vec<u8> = Vec::with_capacity(image_size);
            let mut p = 0usize;
            while block_size != 0 && p + block_size <= data.len() {
                let blk = &data[p..p + block_size];
                let next = be32(blk, 0)? as usize;
                let mut q = 24;
                while q + 2 <= blk.len() {
                    let cs = be16(blk, q)? as usize;
                    q += 2;
                    if cs == 0 {
                        break;
                    }
                    let want = (image_size - img.len()).min(0x8000);
                    if want == 0 || q + cs > blk.len() {
                        break;
                    }
                    let dec = lzx.decompress_next(&blk[q..q + cs], want).map_err(|e| crate::anyhow!("xex lzx at {:#x}: {:?}", img.len(), e))?;
                    img.extend_from_slice(dec);
                    q += cs;
                }
                p += block_size;
                block_size = next;
            }
            Ok(img)
        }
        c => bail!("xex compression {} not supported", c),
    }
}
