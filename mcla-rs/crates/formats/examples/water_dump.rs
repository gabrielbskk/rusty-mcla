//! The water meshes' vertex streams (water work, 2026-10-14): for every batch whose shader name
//! contains the filter, the stride, the channels the mesh has and a few vertices.
//!     cargo run --release -p mcla-formats --example water_dump -- <dir of sector .r1 files> <filter> [vertices]
use mcla_formats::rage::sector::Sector;
use mcla_formats::rsc::Resource;

fn load(p: &std::path::Path) -> Option<Resource> {
    let d = std::fs::read(p).ok()?;
    if d.len() > 16 && &d[0..4] == b"MCLR" {
        let rtype = u32::from_le_bytes(d[4..8].try_into().unwrap());
        let flags = u32::from_le_bytes(d[8..12].try_into().unwrap());
        let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
        let data = d[16..].to_vec();
        let vs = vs.min(data.len());
        return Some(Resource { rtype, flags, virtual_size: vs, physical_size: data.len() - vs, data });
    }
    Resource::parse(&d).ok()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let want = a[2].to_ascii_lowercase();
    let nv: usize = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(4);
    let mut files: Vec<_> = std::fs::read_dir(&a[1]).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |e| e == "r1")).collect();
    files.sort();
    for p in &files {
        let Some(res) = load(p) else { continue };
        let Ok(sec) = Sector::parse(&res) else { continue };
        for (h, t) in sec.textures.iter() {
            let names = ["l_sha_ground_water.dds", "l_sha_water_foam.dds", "l_sha_water_ripple_n.dds"];
            if let Some(n) = names.iter().find(|n| mcla_formats::hash::joaat(n) == *h) {
                let _ = n;
                println!("{} own texture {:08x} {} {} x {} {:?} levels {}", p.file_name().unwrap().to_string_lossy(), h, n, t.width, t.height, t.format, t.levels.len());
            }
        }
        for (bi, b) in sec.batches.iter().enumerate() {
            let Some(sh) = sec.shaders.get(b.shader) else { continue };
            if !sh.name.to_ascii_lowercase().contains(&want) {
                continue;
            }
            let m = &b.mesh;
            let (mn, mx) = m.bbox();
            println!(
                "{} batch {bi} {} flags {:x} stride {} verts {} tris {} pos {} nrm {} tan {} uv0 {} uv1 {} uv2 {} col {} col1 {} box {:.0?}..{:.0?}",
                p.file_name().unwrap().to_string_lossy(), sh.name, b.flags, m.raw_stride, m.positions.len(), m.indices.len() / 3,
                m.positions.len(), m.normals.len(), m.tangents.len(), m.uv0.len(), m.uv1.len(), m.uv2.len(), m.colors.len(), m.colors1.len(), mn, mx
            );
            for i in 0..nv.min(m.positions.len()) {
                println!(
                    "    v{i} pos {:.2?} nrm {:?} uv0 {:?} uv1 {:?} col {:?} col1 {:?}",
                    m.positions[i], m.normals.get(i), m.uv0.get(i), m.uv1.get(i), m.colors.get(i), m.colors1.get(i)
                );
            }
        }
    }
}
