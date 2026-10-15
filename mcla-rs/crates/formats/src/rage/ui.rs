//! UI scene resources (type 27, magic "DIU"): a scripted UI document. We do not run
//! its scripts; we pull out the embedded image objects so the original UI art
//! (HUD dials, fonts, menu frames) can be reused.
//!
//! Image object: `{LE vtable 0x00584114, ..., +0x18 name ptr ("t:/mc4/assets/ui/hud/textures/hud_0327.dds"),
//! +0x1c D3DBaseTexture ptr}`; the D3D object carries the 6 Xenos fetch dwords at +0x1c.

use super::texture::Texture;
use super::Rd;
use crate::rsc::Resource;

pub const UI_IMAGE_VT: u32 = 0x0058_4114;

/// All embedded textures of a UI scene, with their short names (file stem, lower case).
pub fn ui_textures(res: &Resource) -> Vec<Texture> {
    let rd = Rd::new(res);
    let vs = res.virtual_size;
    let mut out = Vec::new();
    let mut o = 0usize;
    while o + 0x24 <= vs {
        if rd.vtable(o) == UI_IMAGE_VT {
            if let (Some(np), Some(dp)) = (rd.ptr(o + 0x18), rd.ptr(o + 0x1c)) {
                let full = rd.cstr(np);
                if !full.is_empty() && rd.u32(dp + 0x14) == 0xFFFF_0000 {
                    let short = full.rsplit(['/', '\\']).next().unwrap_or(&full).to_ascii_lowercase();
                    let mut fetch = [0u32; 6];
                    for (i, f) in fetch.iter_mut().enumerate() {
                        *f = rd.u32(dp + 0x1c + i * 4);
                    }
                    if let Ok(t) = Texture::from_fetch(&rd, short, fetch) {
                        out.push(t);
                    }
                }
            }
            o += 0x20;
            continue;
        }
        o += 4;
    }
    out
}

/// Names of UI variables / labels found in a scene (debug aid).
pub fn ui_strings(res: &Resource) -> Vec<String> {
    let v = &res.data[..res.virtual_size];
    let mut out = Vec::new();
    let mut i = 0;
    while i < v.len() {
        if v[i].is_ascii_graphic() {
            let s = i;
            while i < v.len() && (v[i].is_ascii_graphic() || v[i] == b' ') {
                i += 1;
            }
            if i < v.len() && v[i] == 0 && i - s >= 4 {
                out.push(String::from_utf8_lossy(&v[s..i]).to_string());
            }
        }
        i += 1;
    }
    out
}
