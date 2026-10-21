//! Precompute the game runtime's city index for installations on slower storage.

use anyhow::{Context, Result};
use glam::Vec3;
use mcla_formats::rage::sector::Sector;
use mcla_formats::rage::{vt, Rd};
use rayon::prelude::*;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
struct SectorInfo {
    file: String,
    min: [f32; 3],
    max: [f32; 3],
    tris: u32,
}

#[derive(Serialize)]
struct WorldIndex {
    sectors: Vec<SectorInfo>,
    dicts: Vec<String>,
}

pub fn build(args: &[String]) -> Result<()> {
    let root = Path::new(args.first().context("usage: mcla-tool world-index <data/cache>")?);
    let city = root.join("resources/city/sc");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&city)
        .with_context(|| format!("list {}", city.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "r1"))
        .collect();
    files.sort();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build()?;
    let results = pool.install(|| files.par_iter().map(|path| {
        let Ok(resource) = super::load_res(&path.to_string_lossy()) else {
            return (path.clone(), 0, None);
        };
        let reader = Rd::new(&resource);
        let class = reader.vtable(0);
        if class != vt::CITY_SECTOR {
            return (path.clone(), class, None);
        }
        let Ok(sector) = Sector::parse(&resource) else {
            return (path.clone(), class, None);
        };
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut tris = 0u32;
        for batch in &sector.batches {
            let (a, b) = batch.mesh.bbox();
            min = min.min(a);
            max = max.max(b);
            tris += (batch.mesh.indices.len() / 3) as u32;
        }
        let rel = path.strip_prefix(root).unwrap_or(path).to_string_lossy().replace('\\', "/");
        (path.clone(), class, Some(SectorInfo { file: rel, min: min.to_array(), max: max.to_array(), tris }))
    }).collect::<Vec<_>>());
    let mut index = WorldIndex { sectors: Vec::new(), dicts: Vec::new() };
    for (path, class, sector) in results {
        if let Some(sector) = sector {
            index.sectors.push(sector);
        } else if class == vt::TEXTURE_DICT {
            index.dicts.push(path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/"));
        }
    }
    if index.sectors.is_empty() {
        anyhow::bail!("no city sectors found in {}", city.display());
    }
    let output = root.join("world_index.json");
    std::fs::write(&output, serde_json::to_vec_pretty(&index)?)?;
    println!("{}: {} sectors, {} dictionaries", output.display(), index.sectors.len(), index.dicts.len());
    Ok(())
}
