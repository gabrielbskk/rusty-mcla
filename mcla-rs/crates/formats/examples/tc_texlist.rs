//! Texture-dictionary audit (lighting work, 2026-10-15): lists every texture the port can reach, one TSV line each.
//!     cargo build --release -p mcla-formats --example tc_texlist --target-dir D:\MCLA\work\lighting\target
//!     tc_texlist <extracted cache dir>  >  out\tc_textures.tsv
//! Columns: kind, file (relative to the cache dir), order, name, key (the dictionary's hash), name_hash
//! (joaat of the lower-cased stored name), width, height, format, levels, level0_bytes, fnv64 of level 0,
//! mean R G B A of the decoded level 0 (plain, not alpha weighted).
use mcla_formats::rage::character::Character;
use mcla_formats::rage::sector::Sector;
use mcla_formats::rage::texture::Texture;
use mcla_formats::rage::xenos;
use mcla_formats::rage::{scan::Drawable, vt, Dictionary, Rd};
use mcla_formats::rsc::Resource;
use rayon::prelude::*;
use std::path::{Path, PathBuf};

fn load(p: &Path) -> Option<Resource> {
    let d = std::fs::read(p).ok()?;
    if d.len() > 16 && &d[0..4] == b"MCLR" {
        let rtype = u32::from_le_bytes(d[4..8].try_into().unwrap());
        let flags = u32::from_le_bytes(d[8..12].try_into().unwrap());
        let vs = u32::from_le_bytes(d[12..16].try_into().unwrap()) as usize;
        let data = d[16..].to_vec();
        let vs = vs.min(data.len());
        return Some(Resource { rtype, flags, virtual_size: vs, physical_size: data.len() - vs, data });
    }
    None
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    v.sort();
    for p in v {
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn fnv(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn line(kind: &str, file: &str, order: usize, key: u32, t: &Texture) -> String {
    let nh = mcla_formats::hash::joaat(&t.name.to_ascii_lowercase());
    let l0 = t.levels.first().map(|v| v.as_slice()).unwrap_or(&[]);
    let mut m = [0u64; 4];
    let n;
    if l0.is_empty() {
        n = 1;
    } else {
        let px = xenos::decode_to_rgba8(t.format, l0, t.width as usize, t.height as usize);
        n = (px.len() / 4).max(1) as u64;
        for c in px.chunks_exact(4) {
            for k in 0..4 {
                m[k] += c[k] as u64;
            }
        }
    }
    format!(
        "{}\t{}\t{}\t{}\t{:08x}\t{:08x}\t{}\t{}\t{:?}\t{}\t{}\t{:016x}\t{:.2}\t{:.2}\t{:.2}\t{:.2}",
        kind, file, order, t.name, key, nh, t.width, t.height, t.format, t.levels.len(), l0.len(), fnv(l0), m[0] as f64 / n as f64, m[1] as f64 / n as f64, m[2] as f64 / n as f64, m[3] as f64 / n as f64
    )
}

fn dict_lines(kind: &str, file: &str, rd: &Rd, doff: usize, out: &mut Vec<String>) {
    if let Ok(d) = Dictionary::parse(rd, doff) {
        for (i, (h, v)) in d.hashes.iter().zip(d.values.iter()).enumerate() {
            if let Some(o) = v {
                match Texture::parse(rd, *o) {
                    Ok(t) => out.push(line(kind, file, i, *h, &t)),
                    Err(e) => eprintln!("ERR {} {} {:08x}: {:#}", kind, file, h, e),
                }
            }
        }
    }
}

/// `tc_texlist <cache dir> refs`: the texture NAMES the consumers' shaders ask for (consumer, file, name, hash).
fn refs(root: &Path) {
    use mcla_formats::rage::drawable::Shader;
    let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_string_lossy().replace('\\', "/");
    let emit = |consumer: &str, file: &str, shaders: &[&Shader]| {
        let mut seen = std::collections::BTreeSet::new();
        for sh in shaders {
            for (_ph, n) in sh.textures() {
                if seen.insert(n.to_string()) {
                    println!("{}	{}	{}	{:08x}", consumer, file, n, mcla_formats::hash::joaat(n));
                }
            }
        }
    };
    let mut files = Vec::new();
    walk(&root.join("resources"), &mut files);
    for p in &files {
        let r = rel(p);
        let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if r.starts_with("resources/city/sc/") && ext == "r1" {
            let Some(res) = load(p) else { continue };
            let rd = Rd::new(&res);
            if rd.vtable(0) == vt::CITY_SECTOR {
                if let Ok(sec) = Sector::parse(&res) {
                    let used: std::collections::BTreeSet<usize> = sec.batches.iter().map(|b| b.shader).collect();
                    let v: Vec<&Shader> = used.iter().filter_map(|i| sec.shaders.get(*i)).collect();
                    emit("sector", &r, &v);
                }
            } else if r.ends_with("0xa6e77868.r1") {
                // far city (decode_far): root +08 -> shader group
                if let Some(g) = rd.ptr(0x08) {
                    if let Some(arr) = rd.ptr(g + 0x08) {
                        let shaders: Vec<Shader> = (0..rd.u16(g + 0x0c) as usize).filter_map(|i| rd.ptr(arr + i * 4).and_then(|so| Shader::parse(&rd, so).ok())).collect();
                        let v: Vec<&Shader> = shaders.iter().collect();
                        emit("far", &r, &v);
                    }
                }
            }
        } else if r.starts_with("resources/cutsceneitems/") && ext == "r102" {
            let Some(res) = load(p) else { continue };
            if let Ok(d) = Drawable::parse(&res) {
                let used: std::collections::BTreeSet<usize> = d.models.iter().flat_map(|m| m.parts.iter().map(|x| x.1 as usize)).collect();
                let v: Vec<&Shader> = used.iter().filter_map(|i| d.shader_group.shaders.get(*i)).collect();
                emit("set", &r, &v);
            }
        } else if r.starts_with("resources/prop/") && ext == "r131" {
            let Some(res) = load(p) else { continue };
            let props = mcla_formats::rage::props::parse_props(&res);
            let v: Vec<&Shader> = props.iter().flat_map(|p| p.shaders.iter()).collect();
            emit("prop", &r, &v);
        } else if ext == "r83" && (r.starts_with("resources/vehicle/") || r.starts_with("resources/rims/") || r.starts_with("resources/brakes/") || r.starts_with("resources/tires/models/")) {
            let Some(res) = load(p) else { continue };
            if let Ok(d) = Drawable::parse(&res) {
                let v: Vec<&Shader> = d.shader_group.shaders.iter().collect();
                let c = if r.starts_with("resources/vehicle/") { "car" } else if r.starts_with("resources/rims/") { "rim" } else if r.starts_with("resources/brakes/") { "brake" } else { "tiremodel" };
                emit(c, &r, &v);
            }
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::args().nth(1).expect("cache dir"));
    if std::env::args().nth(2).as_deref() == Some("refs") {
        refs(&root);
        return;
    }
    let rel = |p: &Path| p.strip_prefix(&root).unwrap_or(p).to_string_lossy().replace('\\', "/");
    let mut files = Vec::new();
    walk(&root.join("resources"), &mut files);
    walk(&root.join("textures"), &mut files);
    let lines: Vec<Vec<String>> = files
        .par_iter()
        .map(|p| {
            let r = rel(p);
            let mut out = Vec::new();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            // plain DDS pictures of textures/ (the car/global ones are named by hash)
            if r.starts_with("textures/") && !r.ends_with(".list") && !r.ends_with(".obj") {
                if let Ok(d) = std::fs::read(p) {
                    let fname = p.file_name().unwrap().to_string_lossy().to_string();
                    let key = if let Some(hex) = fname.strip_prefix("0x") { u32::from_str_radix(hex, 16).unwrap_or(0) } else { mcla_formats::hash::joaat(&fname) };
                    match mcla_formats::dds::parse_dds(&fname, &d) {
                        Ok(t) => {
                            let kind = if r.starts_with("textures/global/cars/") { "globaltex_cars" } else if r.starts_with("textures/global/city/") { "globaltex_city" } else { "textures_other" };
                            out.push(line(kind, &r, 0, key, &t));
                        }
                        Err(e) => eprintln!("ERR dds {}: {:#}", r, e),
                    }
                }
                return out;
            }
            if ext == "obj" {
                return out;
            }
            let Some(res) = load(p) else { return out };
            let rd = Rd::new(&res);
            let vtab = rd.vtable(0);
            // ---- by place
            if r.starts_with("resources/city/sc/") && ext == "r1" {
                if vtab == vt::TEXTURE_DICT {
                    dict_lines("city_dict", &r, &rd, 0, &mut out);
                    return out;
                }
                if vtab == vt::CITY_SECTOR {
                    if let Ok(sec) = Sector::parse(&res) {
                        let mut v: Vec<_> = sec.textures.iter().collect();
                        v.sort_by_key(|x| *x.0);
                        for (i, (h, t)) in v.into_iter().enumerate() {
                            out.push(line("city_sector", &r, i, *h, t));
                        }
                    }
                    return out;
                }
                eprintln!("SKIP city/sc {} vtable {:08x}", r, vtab);
                return out;
            }
            if r.starts_with("resources/prop/") && ext == "r1" {
                dict_lines("prop_dict", &r, &rd, 0, &mut out);
                return out;
            }
            if (r.starts_with("resources/tires/textures") || r.starts_with("resources/brakes") || r.starts_with("resources/license_plates")) && ext == "r1" {
                let top = if r.starts_with("resources/tires/textures_small") { "tires_small" } else if r.starts_with("resources/tires") { "tires" } else if r.starts_with("resources/brakes") { "brakes" } else { "plates" };
                if vtab == vt::TEXTURE_DICT {
                    dict_lines(&format!("{top}_dict"), &r, &rd, 0, &mut out);
                } else {
                    // tire set: textures behind pointers at +8, +12; plate: the root pointer
                    let mut k = 0;
                    for off in [0usize, 8, 12] {
                        let at = if off == 0 { rd.ptr(0) } else { rd.ptr(off) };
                        if let Some(Ok(t)) = at.map(|o| Texture::parse(&rd, o)) {
                            if t.width > 0 && t.width <= 4096 && !t.levels.is_empty() {
                                out.push(line(&format!("{top}_obj"), &r, k, mcla_formats::hash::joaat(&t.name.to_ascii_lowercase()), &t));
                                k += 1;
                            }
                        }
                    }
                    if k == 0 {
                        eprintln!("SKIP {} vtable {:08x}", r, vtab);
                    }
                }
                return out;
            }
            if r.starts_with("resources/cutsceneitems/") && ext == "r102" {
                if let Ok(d) = Drawable::parse(&res) {
                    let mut v: Vec<_> = d.texture_offsets.iter().collect();
                    v.sort_by_key(|x| *x.0);
                    for (i, (h, o)) in v.into_iter().enumerate() {
                        if let Ok(t) = Texture::parse(&rd, *o) {
                            out.push(line("set", &r, i, *h, &t));
                        }
                    }
                }
                return out;
            }
            if ext == "r83" && (r.starts_with("resources/vehicle/") || r.starts_with("resources/rims/") || r.starts_with("resources/tires/models/") || r.starts_with("resources/brakes/")) {
                let kind = if r.starts_with("resources/vehicle/") { "car" } else if r.starts_with("resources/rims/") { "rim" } else if r.starts_with("resources/tires/") { "tiremodel" } else { "brake" };
                if let Ok(d) = Drawable::parse(&res) {
                    let mut v: Vec<_> = d.texture_offsets.iter().collect();
                    v.sort_by_key(|x| *x.0);
                    for (i, (h, o)) in v.into_iter().enumerate() {
                        if let Ok(t) = Texture::parse(&rd, *o) {
                            out.push(line(kind, &r, i, *h, &t));
                        }
                    }
                }
                return out;
            }
            if ext == "r6" && r.starts_with("resources/character/") {
                if let Ok(c) = Character::parse(&res) {
                    for (i, t) in c.textures.iter().enumerate() {
                        out.push(line("char", &r, i, mcla_formats::hash::joaat(&t.name.to_ascii_lowercase()), t));
                    }
                }
                return out;
            }
            if r.starts_with("resources/rmptfx/") && ext == "r107" {
                if let Some(doff) = rd.ptr(0x08) {
                    dict_lines("particles_dict", &r, &rd, doff, &mut out);
                }
                return out;
            }
            if r.starts_with("resources/ui/") && (ext == "r9") && r.ends_with("meshtextures.r9") {
                dict_lines("ui_meshtextures", &r, &rd, 0, &mut out);
                return out;
            }
            out
        })
        .collect();
    let mut n = 0;
    println!("kind\tfile\torder\tname\tkey\tname_hash\tw\th\tformat\tlevels\tl0bytes\tfnv64\tmeanR\tmeanG\tmeanB\tmeanA");
    for l in lines.iter().flatten() {
        println!("{}", l);
        n += 1;
    }
    eprintln!("{} textures", n);
}

