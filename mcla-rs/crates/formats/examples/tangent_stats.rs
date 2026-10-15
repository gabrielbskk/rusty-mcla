//! How the city meshes' stored tangents relate to the frame one gets from the triangles'
//! positions and texture coordinates (the lighting work: PSCityBumpSpec tilts the normal along
//! the STORED tangent and cross(normal, tangent) x w).
//!     cargo run --release -p mcla-formats --example tangent_stats -- <dir of sector .r1 files> [shader filter]
use glam::Vec3;
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

#[derive(Default)]
struct Stat {
    tris: u64,
    no_tangent: u64,
    zero_tangent: u64,
    /// stored tangent . geometric dP/du: histogram over [-1, 1] in 8 bins
    t_dot: [u64; 8],
    /// (cross(n, t) x w) . geometric dP/dv
    b_dot: [u64; 8],
    w_pos: u64,
    w_neg: u64,
    len_sum: f64,
    /// walls only (|n.y| < 0.3): is the stored tangent horizontal (|t.y| < 0.3) or vertical (> 0.7)
    wall_t_horizontal: u64,
    wall_t_vertical: u64,
    wall_gt_horizontal: u64,
    wall_gt_vertical: u64,
    /// stored normal . face normal (by the triangle's winding), -1 .. 1 in 8 bins; walls only: the same for the horizontal parts alone
    n_dot: [u64; 8],
    wall_n_h: [u64; 8],
    /// raw w (0 .. 3) of the first corner against the sign of cross(T, N) . dP/dv (-, about 0, +)
    wb: [[u64; 3]; 4],
    /// raw w per VERTEX of the batches
    wv: [u64; 4],
    /// triangles whose corners differ in w; of those, with corner normals more than 8 degrees apart
    mixed_w: u64,
    mixed_w_bent: u64,
}

fn bin(x: f32) -> usize {
    (((x.clamp(-1.0, 1.0) + 1.0) * 4.0) as usize).min(7)
}

/// tangent_stats <dir> near <x> <y> <z> <radius>: the triangles round a point - shader, face
/// normal, stored normal and tangent of the first corner, how many
fn near(a: &[String]) {
    let c = Vec3::new(a[3].parse().unwrap(), a[4].parse().unwrap(), a[5].parse().unwrap());
    let r: f32 = a[6].parse().unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&a[1]).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |e| e == "r1")).collect();
    files.sort();
    for p in &files {
        let Some(res) = load(p) else { continue };
        let Ok(sec) = Sector::parse(&res) else { continue };
        for (bi, b) in sec.batches.iter().enumerate() {
            let m = &b.mesh;
            let name = sec.shaders.get(b.shader).map(|s| s.name.clone()).unwrap_or_default();
            let mut seen: std::collections::BTreeMap<(i32, i32, i32, i32, i32, i32), (u32, f32, f32)> = Default::default();
            for t in m.indices.chunks_exact(3) {
                let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
                if i0 >= m.positions.len() || i1 >= m.positions.len() || i2 >= m.positions.len() {
                    continue;
                }
                let mid = (m.positions[i0] + m.positions[i1] + m.positions[i2]) / 3.0;
                if mid.distance(c) > r {
                    continue;
                }
                let f = (m.positions[i1] - m.positions[i0]).cross(m.positions[i2] - m.positions[i0]);
                let area = f.length() * 0.5;
                let f = f.normalize_or_zero();
                let n = m.normals.get(i0).copied().unwrap_or(Vec3::ZERO);
                let q = |v: f32| (v * 10.0).round() as i32;
                let e = seen.entry((q(f.x), q(f.y), q(f.z), q(n.x), q(n.y), q(n.z))).or_insert((0, 0.0, mid.y));
                e.0 += 1;
                e.1 += area;
                e.2 = e.2.max(mid.y);
                if a.get(7).is_some() {
                    for &i in &[i0, i1, i2] {
                        println!("    {name} batch {bi} v ({:.1} {:.1} {:.1}) colour0 {:?} colour1 {:?}", m.positions[i].x, m.positions[i].y, m.positions[i].z, m.colors.get(i), m.colors1.get(i));
                    }
                }
            }
            for (k, v) in seen {
                if v.1 > 1.0 {
                    println!("{} batch {bi} {name} flags {:x}: face ({} {} {}) / 10, stored normal ({} {} {}) / 10: {} triangles, {:.1} m2, up to y {:.1}", p.file_name().unwrap().to_string_lossy(), b.flags, k.0, k.1, k.2, k.3, k.4, k.5, v.0, v.1, v.2);
                }
            }
        }
    }
}

/// tangent_stats <dir> frame <x> <y> <z> <radius>: per batch with triangles round a point, the triangles (area) by
/// raw w of the tangent word, the sign of T . dP/du and the sign of (cross(T, N) x w) . dP/dv
fn frame(a: &[String]) {
    let c = Vec3::new(a[3].parse().unwrap(), a[4].parse().unwrap(), a[5].parse().unwrap());
    let r: f32 = a[6].parse().unwrap();
    let mut files: Vec<_> = std::fs::read_dir(&a[1]).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |e| e == "r1")).collect();
    files.sort();
    let sg = |d: f32| if d < -0.3 { '-' } else if d > 0.3 { '+' } else { '0' };
    for p in &files {
        let Some(res) = load(p) else { continue };
        let Ok(sec) = Sector::parse(&res) else { continue };
        for (bi, b) in sec.batches.iter().enumerate() {
            let m = &b.mesh;
            if m.tangents.len() != m.positions.len() || m.uv0.len() != m.positions.len() {
                continue;
            }
            let name = sec.shaders.get(b.shader).map(|s| s.name.clone()).unwrap_or_default();
            let mut tab: std::collections::BTreeMap<(u32, char, char), (u32, f32)> = Default::default();
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            for t in m.indices.chunks_exact(3) {
                let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
                if i0 >= m.positions.len() || i1 >= m.positions.len() || i2 >= m.positions.len() {
                    continue;
                }
                let mid = (m.positions[i0] + m.positions[i1] + m.positions[i2]) / 3.0;
                if mid.distance(c) > r {
                    continue;
                }
                let (e1, e2) = (m.positions[i1] - m.positions[i0], m.positions[i2] - m.positions[i0]);
                let (d1, d2) = (m.uv0[i1] - m.uv0[i0], m.uv0[i2] - m.uv0[i0]);
                let det = d1.x * d2.y - d2.x * d1.y;
                let area = e1.cross(e2).length() * 0.5;
                let tv = m.tangents[i0];
                let t3 = Vec3::new(tv.x, tv.y, tv.z).normalize_or_zero();
                let n = m.normals.get(i0).copied().unwrap_or(Vec3::Y);
                let w = if tv.w >= 1.5 { -1.0 } else if tv.w < 0.5 { 0.0 } else { 1.0 };
                let (st, sb) = if det.abs() < 1e-9 { ('?', '?') } else {
                    let gt = ((e1 * d2.y - e2 * d1.y) / det).normalize_or_zero();
                    let gb = ((e2 * d1.x - e1 * d2.x) / det).normalize_or_zero();
                    (sg(t3.dot(gt)), sg((t3.cross(n) * w).dot(gb)))
                };
                let e = tab.entry((tv.w as u32, st, sb)).or_insert((0, 0.0));
                e.0 += 1;
                e.1 += area;
                lo = lo.min(mid);
                hi = hi.max(mid);
            }
            if !tab.is_empty() {
                println!("{} batch {bi} {name} flags {:x} ({:.1} {:.1} {:.1}) .. ({:.1} {:.1} {:.1}): {}", p.file_name().unwrap().to_string_lossy(), b.flags, lo.x, lo.y, lo.z, hi.x, hi.y, hi.z,
                    tab.iter().map(|(k, v)| format!("w{} T{} B{}: {} / {:.1} m2", k.0, k.1, k.2, v.0, v.1)).collect::<Vec<_>>().join("; "));
            }
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.get(2).map(|s| s.as_str()) == Some("frame") {
        frame(&a);
        return;
    }
    // tangent_stats <dir> shaders <part of a shader name>: the textures such shaders name, by
    // sampler hash, and how many shaders have each
    if a.get(2).map(|s| s.as_str()) == Some("shaders") {
        let want = a[3].to_ascii_lowercase();
        let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
        let mut files: Vec<_> = std::fs::read_dir(&a[1]).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |e| e == "r1")).collect();
        files.sort();
        for p in files.iter().take(a.get(4).and_then(|v| v.parse().ok()).unwrap_or(40)) {
            let Some(res) = load(p) else { continue };
            let Ok(sec) = Sector::parse(&res) else { continue };
            for sh in &sec.shaders {
                if !sh.name.to_ascii_lowercase().contains(&want) { continue }
                for (h, n) in sh.textures() {
                    *seen.entry(format!("{} 0x{:08x} {}", sh.name, h, n)).or_default() += 1;
                }
            }
        }
        for (k, n) in seen { println!("{n:5} {k}"); }
        return;
    }
    if a.get(2).map(|s| s.as_str()) == Some("near") {
        near(&a);
        return;
    }
    let filter = a.get(2).map(|s| s.to_ascii_lowercase());
    let mut stats: std::collections::BTreeMap<String, Stat> = Default::default();
    let mut files: Vec<_> = std::fs::read_dir(&a[1]).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().map_or(false, |e| e == "r1")).collect();
    files.sort();
    let take = if std::env::var("MCLA_TS_ALL").is_ok() { usize::MAX } else { 80 };
    for p in files.iter().take(take) {
        let Some(res) = load(p) else { continue };
        let Ok(sec) = Sector::parse(&res) else { continue };
        for b in &sec.batches {
            let name = sec.shaders.get(b.shader).map(|s| s.name.clone()).unwrap_or_default();
            if filter.as_ref().map_or(false, |f| !name.to_ascii_lowercase().contains(f.as_str())) {
                continue;
            }
            let m = &b.mesh;
            let st = stats.entry(name).or_default();
            for tv in &m.tangents {
                st.wv[(tv.w as usize).min(3)] += 1;
            }
            for t in m.indices.chunks_exact(3) {
                let (i0, i1, i2) = (t[0] as usize, t[1] as usize, t[2] as usize);
                if i0 >= m.positions.len() || i1 >= m.positions.len() || i2 >= m.positions.len() || m.uv0.len() != m.positions.len() {
                    continue;
                }
                st.tris += 1;
                {
                    let fnrm = (m.positions[i1] - m.positions[i0]).cross(m.positions[i2] - m.positions[i0]).normalize_or_zero();
                    let n = m.normals.get(i0).copied().unwrap_or(Vec3::Y);
                    if fnrm.length() > 0.5 {
                        st.n_dot[bin(n.dot(fnrm).abs() * 2.0 - 1.0)] += 1;
                        if fnrm.y.abs() < 0.3 {
                            let (a, b) = (Vec3::new(n.x, 0.0, n.z).normalize_or_zero(), Vec3::new(fnrm.x, 0.0, fnrm.z).normalize_or_zero());
                            st.wall_n_h[bin(a.dot(b).abs() * 2.0 - 1.0)] += 1;
                        }
                    }
                }
                if m.tangents.len() != m.positions.len() {
                    st.no_tangent += 1;
                    continue;
                }
                let (e1, e2) = (m.positions[i1] - m.positions[i0], m.positions[i2] - m.positions[i0]);
                let (d1, d2) = (m.uv0[i1] - m.uv0[i0], m.uv0[i2] - m.uv0[i0]);
                let det = d1.x * d2.y - d2.x * d1.y;
                if det.abs() < 1e-9 {
                    continue;
                }
                let gt = ((e1 * d2.y - e2 * d1.y) / det).normalize_or_zero();
                let gb = ((e2 * d1.x - e1 * d2.x) / det).normalize_or_zero();
                let tv = m.tangents[i0];
                let t3 = Vec3::new(tv.x, tv.y, tv.z);
                if t3.length() < 0.01 {
                    st.zero_tangent += 1;
                    continue;
                }
                st.len_sum += t3.length() as f64;
                let n = m.normals.get(i0).copied().unwrap_or(Vec3::Y);
                let bt = n.cross(t3) * tv.w;
                st.t_dot[bin(t3.normalize().dot(gt))] += 1;
                st.b_dot[bin(bt.normalize_or_zero().dot(gb))] += 1;
                if tv.w >= 0.0 { st.w_pos += 1 } else { st.w_neg += 1 }
                {
                    let d = t3.normalize().cross(n).dot(gb);
                    st.wb[(tv.w as usize).min(3)][if d < -0.3 { 0 } else if d > 0.3 { 2 } else { 1 }] += 1;
                    let (w1, w2) = (m.tangents[i1].w, m.tangents[i2].w);
                    if w1 != tv.w || w2 != tv.w {
                        st.mixed_w += 1;
                        let (n1, n2) = (m.normals.get(i1).copied().unwrap_or(Vec3::Y), m.normals.get(i2).copied().unwrap_or(Vec3::Y));
                        if n.dot(n1) < 0.99 || n.dot(n2) < 0.99 || n1.dot(n2) < 0.99 { st.mixed_w_bent += 1 }
                    }
                }
                if n.y.abs() < 0.3 {
                    let ty = t3.normalize().y.abs();
                    if ty < 0.3 { st.wall_t_horizontal += 1 } else if ty > 0.7 { st.wall_t_vertical += 1 }
                    let gy = gt.y.abs();
                    if gy < 0.3 { st.wall_gt_horizontal += 1 } else if gy > 0.7 { st.wall_gt_vertical += 1 }
                }
            }
        }
    }
    for (name, s) in &stats {
        if s.tris == 0 {
            continue;
        }
        let with = s.tris - s.no_tangent - s.zero_tangent;
        println!("{name}: {} triangles, {} without the channel, {} with a zero tangent, mean length {:.3}, w + {} / - {}", s.tris, s.no_tangent, s.zero_tangent, s.len_sum / with.max(1) as f64, s.w_pos, s.w_neg);
        println!("    |normal . face normal| (0 .. 1 in 8 bins): {:?}; walls, horizontal parts: {:?}", s.n_dot, s.wall_n_h);
        if with > 0 {
            println!("    tangent . dP/du  (-1 .. 1 in 8 bins): {:?}", s.t_dot);
            println!("    binormal . dP/dv (-1 .. 1 in 8 bins): {:?}", s.b_dot);
            println!("    raw w per vertex 0/1/2/3: {:?}; per triangle, raw w x sign of cross(T, N) . dP/dv [-, 0, +]: {:?}; triangles with mixed w {} (bent {})", s.wv, s.wb, s.mixed_w, s.mixed_w_bent);
            println!("    walls: stored tangent horizontal {} vertical {}; dP/du horizontal {} vertical {}", s.wall_t_horizontal, s.wall_t_vertical, s.wall_gt_horizontal, s.wall_gt_vertical);
        }
    }
}
