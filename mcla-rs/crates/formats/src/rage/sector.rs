//! City sector resources (`resources/city/sc/*.r1`, class 0x005cb710):
//! a texture dictionary, a shader group and a list of (shader, geometry) draw batches
//! whose vertices are already in world space.

use super::drawable::Shader;
use super::geometry::{parse_geometry, Mesh};
use super::texture::Texture;
use super::{vt, Dictionary, Rd};
use crate::rsc::Resource;
use crate::{bail, Result};
use hashbrown::HashMap;

#[derive(Debug, Clone)]
pub struct Batch {
    pub shader: usize,
    pub mesh: Mesh,
    /// low-nibble flags from the shader pointer (meaning TBD: 1/2/4 seen)
    pub flags: u32,
}

#[derive(Debug, Default)]
pub struct Sector {
    /// textures keyed by name hash (joaat of lower-case name including ".dds")
    pub textures: HashMap<u32, Texture>,
    pub shaders: Vec<Shader>,
    pub batches: Vec<Batch>,
    pub errors: Vec<String>,
}

impl Sector {
    pub fn parse(res: &Resource) -> Result<Sector> {
        let rd = Rd::new(res);
        if rd.vtable(0) != vt::CITY_SECTOR {
            bail!("not a city sector (vtable {:#x})", rd.vtable(0));
        }
        let mut sec = Sector::default();
        // textures
        if let Some(td) = rd.ptr(0x08) {
            match Dictionary::parse(&rd, td) {
                Ok(d) => {
                    for (h, v) in d.hashes.iter().zip(d.values.iter()) {
                        if let Some(o) = v {
                            match Texture::parse(&rd, *o) {
                                Ok(t) => {
                                    sec.textures.insert(*h, t);
                                }
                                Err(e) => sec.errors.push(format!("texture {:08x}: {:#}", h, e)),
                            }
                        }
                    }
                }
                Err(e) => sec.errors.push(format!("texture dict: {:#}", e)),
            }
        }
        // batches
        let count = rd.u16(0x24) as usize;
        let Some(arr) = rd.ptr(0x18) else {
            return Ok(sec);
        };
        let mut shader_index: HashMap<usize, usize> = HashMap::new();
        for i in 0..count {
            // low nibble of the shader pointer carries per-batch flags
            let flags = rd.u32(arr + i * 8) & 0xF;
            let so = rd.res.ptr(rd.u32(arr + i * 8) & !0xF);
            let go = rd.res.ptr(rd.u32(arr + i * 8 + 4) & !0xF);
            if i == 0 {
                log::debug!("batch0 so={:?} go={:?} vs={:#x} ib_word={:#x}", so, go, res.virtual_size, go.map(|g| rd.u32(g + 0x18)).unwrap_or(0));
            }
            let (Some(so), Some(go)) = (so, go) else {
                continue;
            };
            if rd.vtable(so) != vt::SHADER || rd.vtable(go) != vt::GEOMETRY {
                sec.errors.push(format!("batch {} unexpected classes {:#x}/{:#x}", i, rd.vtable(so), rd.vtable(go)));
                continue;
            }
            let si = match shader_index.get(&so) {
                Some(&i) => i,
                None => {
                    let s = Shader::parse(&rd, so)?;
                    sec.shaders.push(s);
                    let i = sec.shaders.len() - 1;
                    shader_index.insert(so, i);
                    i
                }
            };
            match parse_geometry(&rd, go) {
                Ok(mesh) => sec.batches.push(Batch { shader: si, mesh, flags }),
                Err(e) => sec.errors.push(format!("batch {} geometry: {:#}", i, e)),
            }
        }
        Ok(sec)
    }

    /// Look up a texture by shader parameter name.
    pub fn texture_by_name(&self, name: &str) -> Option<&Texture> {
        let h = crate::hash::joaat(name);
        if let Some(t) = self.textures.get(&h) {
            return Some(t);
        }
        // try without extension
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
        self.textures.get(&crate::hash::joaat(stem))
    }
}
