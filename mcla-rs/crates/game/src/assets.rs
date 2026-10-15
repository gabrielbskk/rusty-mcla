//! Asset access: reads resources from the extracted data directory
//! (`mcla-tool extract` output) or, later, straight from the ISO.

use anyhow::{Context, Result};
use mcla_formats::rsc::Resource;
use std::path::{Path, PathBuf};

pub struct Assets {
    pub root: PathBuf,
}

impl Assets {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Assets { root: root.into() }
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Load a resource saved by `mcla-tool extract` ("MCLR" header) or a raw RSC blob.
    pub fn load_resource(&self, rel: impl AsRef<Path>) -> Result<Resource> {
        let p = if rel.as_ref().is_absolute() { rel.as_ref().to_path_buf() } else { self.root.join(rel.as_ref()) };
        load_resource_file(&p)
    }

    pub fn read(&self, rel: &str) -> Result<Vec<u8>> {
        std::fs::read(self.root.join(rel)).with_context(|| format!("read {}", rel))
    }

    /// All city sector / dictionary files.
    pub fn city_files(&self) -> Result<Vec<PathBuf>> {
        let dir = self.root.join("resources/city/sc");
        let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
            .with_context(|| format!("list {}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().map_or(false, |x| x == "r1"))
            .collect();
        v.sort();
        Ok(v)
    }
}

pub fn load_resource_file(p: &Path) -> Result<Resource> {
    let d = std::fs::read(p).with_context(|| format!("read {}", p.display()))?;
    if d.len() > 16 && &d[0..4] == b"MCLR" {
        let rtype = u32::from_le_bytes(d[4..8].try_into().unwrap());
        let flags = u32::from_le_bytes(d[8..12].try_into().unwrap());
        let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
        let data = d[16..].to_vec();
        let vs = vs.min(data.len());
        return Ok(Resource { rtype, flags, virtual_size: vs, physical_size: data.len() - vs, data });
    }
    Resource::parse(&d)
}
