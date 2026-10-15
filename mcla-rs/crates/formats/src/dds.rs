//! Plain DDS files (PC layout, little-endian, untiled) as found in
//! `textures/global/cars/globaltex`, `sky/`, `textures/`.

use crate::bin::u32le;
use crate::rage::texture::Texture;
use crate::rage::xenos::TexFormat;
use crate::{bail, Result};

pub fn parse_dds(name: &str, d: &[u8]) -> Result<Texture> {
    if d.len() < 128 || &d[0..4] != b"DDS " {
        bail!("not a DDS file");
    }
    let height = u32le(d, 12);
    let width = u32le(d, 16);
    let mips = u32le(d, 28).max(1);
    let pf_flags = u32le(d, 80);
    let fourcc = &d[84..88];
    let bpp = u32le(d, 88);
    let rmask = u32le(d, 92);
    let amask = u32le(d, 104);
    let mut off = 128usize;
    let format = if pf_flags & 0x4 != 0 {
        match fourcc {
            b"DXT1" => TexFormat::Dxt1,
            b"DXT3" | b"DXT2" => TexFormat::Dxt3,
            b"DXT5" | b"DXT4" => TexFormat::Dxt5,
            b"ATI2" | b"BC5U" => TexFormat::Dxn,
            b"ATI1" | b"BC4U" => TexFormat::Dxt5a,
            b"DX10" => {
                off += 20;
                let dxgi = u32le(d, 128);
                match dxgi {
                    70..=72 => TexFormat::Dxt1,
                    73..=75 => TexFormat::Dxt3,
                    76..=78 => TexFormat::Dxt5,
                    82..=84 => TexFormat::Dxn,
                    79..=81 => TexFormat::Dxt5a,
                    27..=29 => TexFormat::Rgba8,
                    o => bail!("unsupported DXGI format {}", o),
                }
            }
            o => bail!("unsupported fourcc {:?}", String::from_utf8_lossy(o)),
        }
    } else if bpp == 32 {
        TexFormat::Rgba8
    } else if bpp == 8 {
        TexFormat::R8
    } else if bpp == 16 {
        if amask == 0x8000 {
            TexFormat::Rgb5a1
        } else if amask == 0xF000 {
            TexFormat::Rgba4
        } else {
            TexFormat::Rgb565
        }
    } else {
        bail!("unsupported DDS pixel format bpp={}", bpp);
    };
    let (bw, bpb) = format.block();
    let mut levels = Vec::new();
    let (mut w, mut h) = (width as usize, height as usize);
    for _ in 0..mips {
        let wb = (w + bw - 1) / bw;
        let hb = (h + bw - 1) / bw;
        let size = wb * hb * bpb;
        if off + size > d.len() {
            break;
        }
        let mut lvl = d[off..off + size].to_vec();
        if format == TexFormat::Rgba8 {
            // DDS 32-bit is B,G,R,A in memory (for the usual 0x00ff0000 red mask);
            // convert to the A,R,G,B order our decoder expects.
            let bgra = rmask == 0x00ff_0000 || rmask == 0;
            for px in lvl.chunks_exact_mut(4) {
                let (b, g, r, a) = if bgra { (px[0], px[1], px[2], px[3]) } else { (px[2], px[1], px[0], px[3]) };
                px[0] = a;
                px[1] = r;
                px[2] = g;
                px[3] = b;
            }
        }
        levels.push(lvl);
        off += size;
        if w == 1 && h == 1 {
            break;
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    if levels.is_empty() {
        bail!("DDS without data");
    }
    Ok(Texture { name: name.to_string(), width, height, format, levels, fetch: [0; 6] })
}
