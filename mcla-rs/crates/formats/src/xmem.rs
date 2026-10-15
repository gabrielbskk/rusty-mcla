//! Xbox 360 `XMemCompress` LZX streams.
//!
//! Layout used inside MC:LA RSC resources:
//! ```text
//! u32be 0x0FF512EF           magic
//! u32be total                bytes of chunk data that follow
//! chunks...                  each: [u16be csize]            -> 0x8000 output bytes
//!                                  [0xFF u16be usize u16be csize] -> `usize` output bytes
//! ```
//! Each chunk is one LZX frame of a single continuous LZX stream (64 KiB window),
//! exactly like XNB containers, so the `lzxd` crate can be driven chunk by chunk.

use crate::bin::{u16be, u32be};
use crate::{bail, Result};
use lzxd::{Lzxd, WindowSize};

pub const XMEM_MAGIC: u32 = 0x0FF5_12EF;

/// Walk the chunk headers and return the total decompressed size without decoding.
pub fn decompressed_len(stream: &[u8]) -> Result<usize> {
    if stream.len() < 8 || u32be(stream, 0) != XMEM_MAGIC {
        bail!("not an XMemCompress stream");
    }
    let total = u32be(stream, 4) as usize;
    let end = 8 + total;
    if end > stream.len() {
        bail!("XMem stream truncated: need {} have {}", end, stream.len());
    }
    let mut p = 8;
    let mut out = 0usize;
    while p < end {
        let (u, c, hl) = chunk_header(&stream[p..end])?;
        out += u;
        p += hl + c;
    }
    Ok(out)
}

fn chunk_header(b: &[u8]) -> Result<(usize, usize, usize)> {
    if b.is_empty() {
        bail!("empty chunk header");
    }
    if b[0] == 0xFF {
        if b.len() < 5 {
            bail!("short extended chunk header");
        }
        Ok((u16be(b, 1) as usize, u16be(b, 3) as usize, 5))
    } else {
        if b.len() < 2 {
            bail!("short chunk header");
        }
        Ok((0x8000, u16be(b, 0) as usize, 2))
    }
}

/// Decompress a whole XMemCompress stream.
pub fn decompress(stream: &[u8]) -> Result<Vec<u8>> {
    // MC:LA resources were compressed with a 128 KiB LZX window; the LZX state
    // (window + trees) continues across chunks.
    decompress_with_window(stream, WindowSize::KB128)
}

pub fn decompress_with_window(stream: &[u8], window: WindowSize) -> Result<Vec<u8>> {
    if stream.len() < 8 || u32be(stream, 0) != XMEM_MAGIC {
        bail!("not an XMemCompress stream");
    }
    let total = u32be(stream, 4) as usize;
    let end = (8 + total).min(stream.len());
    let mut out = Vec::with_capacity(decompressed_len(stream).unwrap_or(total * 4));
    let mut lzx = Lzxd::new(window);
    let mut p = 8;
    while p < end {
        let (usize_, csize, hl) = chunk_header(&stream[p..end])?;
        p += hl;
        if p + csize > end {
            bail!("chunk overruns stream");
        }
        let chunk = &stream[p..p + csize];
        p += csize;
        if usize_ == 0 {
            continue;
        }
        let dec = lzx
            .decompress_next(chunk, usize_)
            .map_err(|e| crate::anyhow!("LZX error at {:#x}: {:?}", p, e))?;
        out.extend_from_slice(dec);
    }
    Ok(out)
}
