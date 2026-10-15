//! The map screen's city: the game shows Los Angeles itself from the air. Here the whole
//! city (every sector on the disc) is drawn once from straight above into a colour image
//! and a height field - each surface in the mean colour of its texture, with its night
//! vertex colours kept apart - and cached beside the extracted data. The map screen then
//! flies a camera over a mesh built from the two.
use crate::assets::Assets;
use crate::render::{MatMode, Vertex};
use crate::world::{decode_sector, SectorInfo, SharedTextures};
use glam::Vec3;
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// metres per texel of the colour image
pub const RES: f32 = 1.5;
/// colour texels per height sample (and per mesh vertex)
pub const STEP: usize = 4;
const MAGIC: &[u8; 8] = b"MCLAMAP3";
/// metres per step of the full-resolution heights
const FINE_Q: f32 = 0.02;

pub struct CityMap {
    pub w: usize,
    pub h: usize,
    pub min_x: f32,
    pub min_z: f32,
    /// RGB8, w x h
    pub color: Vec<u8>,
    /// per height sample (gw x gh): height (m) and the night light's colour
    pub gw: usize,
    pub gh: usize,
    pub height: Vec<f32>,
    pub night: Vec<[u8; 3]>,
    /// height of every texel (w x h), in steps of 2 cm above the lowest
    pub fine: Vec<u16>,
    pub fine_min: f32,
}

struct Tile {
    x0: usize,
    z0: usize,
    w: usize,
    h: usize,
    y: Vec<f32>,
    color: Vec<[u8; 3]>,
    night: Vec<[u8; 3]>,
}

fn cache_paths(assets: &Assets) -> (PathBuf, PathBuf, PathBuf) {
    (assets.path("citymap_color.png"), assets.path("citymap_relief.bin"), assets.path("citymap_height.png"))
}

impl CityMap {
    /// The cached map, if it has been drawn before.
    pub fn load(assets: &Assets) -> Option<CityMap> {
        let (png_path, bin_path, fine_path) = cache_paths(assets);
        let mut f = std::fs::File::open(&bin_path).ok()?;
        let mut head = [0u8; 8 + 4 * 7];
        f.read_exact(&mut head).ok()?;
        if &head[..8] != MAGIC {
            return None;
        }
        let u = |i: usize| u32::from_le_bytes(head[8 + i * 4..12 + i * 4].try_into().unwrap());
        let fl = |i: usize| f32::from_le_bytes(head[8 + i * 4..12 + i * 4].try_into().unwrap());
        let (w, h, gw, gh, min_x, min_z, fine_min) = (u(0) as usize, u(1) as usize, u(2) as usize, u(3) as usize, fl(4), fl(5), fl(6));
        let n = gw * gh;
        let mut raw = vec![0u8; n * 7];
        f.read_exact(&mut raw).ok()?;
        let height: Vec<f32> = raw[..n * 4].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let night: Vec<[u8; 3]> = raw[n * 4..].chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
        let dec = png::Decoder::new(std::fs::File::open(&png_path).ok()?);
        let mut reader = dec.read_info().ok()?;
        let mut color = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut color).ok()?;
        if info.width as usize != w || info.height as usize != h || info.color_type != png::ColorType::Rgb {
            return None;
        }
        color.truncate(w * h * 3);
        // the full-resolution heights: a 16-bit grey image
        let mut dec = png::Decoder::new(std::fs::File::open(&fine_path).ok()?);
        dec.set_transformations(png::Transformations::IDENTITY);
        let mut reader = dec.read_info().ok()?;
        let mut raw = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut raw).ok()?;
        if info.width as usize != w || info.height as usize != h || info.bit_depth != png::BitDepth::Sixteen {
            return None;
        }
        let fine: Vec<u16> = raw[..w * h * 2].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        Some(CityMap { w, h, min_x, min_z, color, gw, gh, height, night, fine, fine_min })
    }

    fn save(&self, assets: &Assets) -> anyhow::Result<()> {
        let (png_path, bin_path, fine_path) = cache_paths(assets);
        {
            let f = std::io::BufWriter::new(std::fs::File::create(&fine_path)?);
            let mut enc = png::Encoder::new(f, self.w as u32, self.h as u32);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Sixteen);
            enc.set_compression(png::Compression::Fast);
            let mut wr = enc.write_header()?;
            let mut raw = Vec::with_capacity(self.fine.len() * 2);
            for v in &self.fine {
                raw.extend_from_slice(&v.to_be_bytes());
            }
            wr.write_image_data(&raw)?;
        }
        {
            let f = std::io::BufWriter::new(std::fs::File::create(&png_path)?);
            let mut enc = png::Encoder::new(f, self.w as u32, self.h as u32);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut wr = enc.write_header()?;
            wr.write_image_data(&self.color)?;
        }
        let mut f = std::io::BufWriter::new(std::fs::File::create(&bin_path)?);
        f.write_all(MAGIC)?;
        for v in [self.w as u32, self.h as u32, self.gw as u32, self.gh as u32] {
            f.write_all(&v.to_le_bytes())?;
        }
        f.write_all(&self.min_x.to_le_bytes())?;
        f.write_all(&self.min_z.to_le_bytes())?;
        f.write_all(&self.fine_min.to_le_bytes())?;
        for v in &self.height {
            f.write_all(&v.to_le_bytes())?;
        }
        for c in &self.night {
            f.write_all(c)?;
        }
        Ok(())
    }

    /// Draw the whole city from above (takes a while: every sector is read).
    pub fn bake(assets: &Assets, sectors: &[SectorInfo], shared: &SharedTextures) -> CityMap {
        let t0 = std::time::Instant::now();
        let mut mn = Vec3::splat(f32::INFINITY);
        let mut mx = Vec3::splat(f32::NEG_INFINITY);
        for s in sectors {
            mn = mn.min(Vec3::from(s.min));
            mx = mx.max(Vec3::from(s.max));
        }
        let (min_x, min_z) = (mn.x.floor(), mn.z.floor());
        let w = (((mx.x - min_x) / RES).ceil() as usize / STEP + 1) * STEP;
        let h = (((mx.z - min_z) / RES).ceil() as usize / STEP + 1) * STEP;
        // mean colours of the shared textures, found once
        let shared_means: Mutex<HashMap<u32, Option<[u8; 3]>>> = Mutex::new(HashMap::new());
        let tiles: Vec<Tile> = sectors
            .par_iter()
            .enumerate()
            .filter_map(|(id, info)| {
                let cs = decode_sector(id, &assets.path(&info.file)).ok()?;
                let mut means: HashMap<u32, Option<[u8; 3]>> = HashMap::new();
                for (hsh, t) in &cs.textures {
                    means.insert(*hsh, Some(t.mean_color().0));
                }
                let mean_of = |hsh: u32, means: &mut HashMap<u32, Option<[u8; 3]>>| -> Option<[u8; 3]> {
                    if let Some(m) = means.get(&hsh) {
                        return *m;
                    }
                    if let Some(m) = shared_means.lock().unwrap().get(&hsh) {
                        return *m;
                    }
                    let m = shared.get(hsh).map(|t| t.mean_color().0);
                    shared_means.lock().unwrap().insert(hsh, m);
                    m
                };
                // the sector's footprint
                let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
                for b in &cs.batches {
                    lo = lo.min(b.min);
                    hi = hi.max(b.max);
                }
                if !lo.is_finite() || !hi.is_finite() {
                    return None;
                }
                let px = |x: f32| ((x - min_x) / RES).floor().max(0.0) as usize;
                let pz = |z: f32| ((z - min_z) / RES).floor().max(0.0) as usize;
                let (x0, z0) = (px(lo.x).min(w - 1), pz(lo.z).min(h - 1));
                let (x1, z1) = (px(hi.x).min(w - 1), pz(hi.z).min(h - 1));
                let (tw, th) = (x1 - x0 + 1, z1 - z0 + 1);
                let mut tile = Tile { x0, z0, w: tw, h: th, y: vec![f32::NEG_INFINITY; tw * th], color: vec![[0; 3]; tw * th], night: vec![[0; 3]; tw * th] };
                for b in &cs.batches {
                    // glass and light cones would hide what is under them
                    if matches!(b.mode, MatMode::Blend | MatMode::Neon | MatMode::Marker | MatMode::StoreGlass | MatMode::OceanWater | MatMode::OceanWaterLod) {
                        continue;
                    }
                    let mut albedo = mean_of(b.diffuse, &mut means).unwrap_or([120, 120, 120]);
                    if matches!(b.mode, MatMode::Road | MatMode::Ground) {
                        // ground maps store their brightness in the red channel
                        let g = (albedo[0] as f32 * 0.82) as u8;
                        albedo = [g, g, g];
                    }
                    for t in b.indices.chunks_exact(3) {
                        let (Some(a), Some(bv), Some(c)) = (b.vertices.get(t[0] as usize), b.vertices.get(t[1] as usize), b.vertices.get(t[2] as usize)) else { continue };
                        let p = [a.pos, bv.pos, c.pos];
                        // to tile texels
                        let q: [[f32; 2]; 3] = [0, 1, 2].map(|i| [(p[i][0] - min_x) / RES - x0 as f32, (p[i][2] - min_z) / RES - z0 as f32]);
                        let nightc = |k: usize| (a.color1[k] as u32 + bv.color1[k] as u32 + c.color1[k] as u32) / 3;
                        let night = [0, 1, 2].map(|k| nightc(k).min(255) as u8);
                        let mut put = |ix: i32, iz: i32, y: f32, tile: &mut Tile| {
                            if ix < 0 || iz < 0 || ix as usize >= tile.w || iz as usize >= tile.h {
                                return;
                            }
                            let i = iz as usize * tile.w + ix as usize;
                            if y > tile.y[i] {
                                tile.y[i] = y;
                                tile.color[i] = albedo;
                                tile.night[i] = night;
                            }
                        };
                        let (bx0, bx1) = (q[0][0].min(q[1][0]).min(q[2][0]), q[0][0].max(q[1][0]).max(q[2][0]));
                        let (bz0, bz1) = (q[0][1].min(q[1][1]).min(q[2][1]), q[0][1].max(q[1][1]).max(q[2][1]));
                        let area = (q[1][0] - q[0][0]) * (q[2][1] - q[0][1]) - (q[2][0] - q[0][0]) * (q[1][1] - q[0][1]);
                        if area.abs() < 1e-6 {
                            continue;
                        }
                        // (small ones: their middle)
                        let (cx, cz) = ((q[0][0] + q[1][0] + q[2][0]) / 3.0, (q[0][1] + q[1][1] + q[2][1]) / 3.0);
                        put(cx.floor() as i32, cz.floor() as i32, (p[0][1] + p[1][1] + p[2][1]) / 3.0, &mut tile);
                        if bx1 - bx0 < 1.0 && bz1 - bz0 < 1.0 {
                            continue;
                        }
                        let inv = 1.0 / area;
                        for iz in (bz0.floor() as i32).max(0)..=(bz1.floor() as i32).min(tile.h as i32 - 1) {
                            for ix in (bx0.floor() as i32).max(0)..=(bx1.floor() as i32).min(tile.w as i32 - 1) {
                                let (sx, sz) = (ix as f32 + 0.5, iz as f32 + 0.5);
                                let w0 = ((q[1][0] - sx) * (q[2][1] - sz) - (q[2][0] - sx) * (q[1][1] - sz)) * inv;
                                let w1 = ((q[2][0] - sx) * (q[0][1] - sz) - (q[0][0] - sx) * (q[2][1] - sz)) * inv;
                                let w2 = 1.0 - w0 - w1;
                                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                                    continue;
                                }
                                put(ix, iz, w0 * p[0][1] + w1 * p[1][1] + w2 * p[2][1], &mut tile);
                            }
                        }
                    }
                }
                Some(tile)
            })
            .collect();
        // put the tiles together (sectors overlap at their edges: the higher surface shows)
        let mut y = vec![f32::NEG_INFINITY; w * h];
        let mut color = vec![0u8; w * h * 3];
        let mut night_full = vec![[0u8; 3]; w * h];
        for t in &tiles {
            for iz in 0..t.h {
                for ix in 0..t.w {
                    let (i, j) = (iz * t.w + ix, (t.z0 + iz) * w + t.x0 + ix);
                    if t.y[i] > y[j] {
                        y[j] = t.y[i];
                        color[j * 3..j * 3 + 3].copy_from_slice(&t.color[i]);
                        night_full[j] = t.night[i];
                    }
                }
            }
        }
        drop(tiles);
        // nothing there: the sea floor / the land beyond, dark
        let floor = mn.y.max(-20.0);
        for j in 0..w * h {
            if y[j] == f32::NEG_INFINITY {
                y[j] = floor;
                color[j * 3..j * 3 + 3].copy_from_slice(&[26, 30, 34]);
            }
        }
        // one height and one night colour per STEP x STEP block
        let (gw, gh) = (w / STEP + 1, h / STEP + 1);
        let mut height = vec![0.0f32; gw * gh];
        let mut night = vec![[0u8; 3]; gw * gh];
        for gz in 0..gh {
            for gx in 0..gw {
                let (mut sum, mut sn, mut n) = (0.0f32, [0u32; 3], 0u32);
                for dz in 0..STEP {
                    for dx in 0..STEP {
                        // (the block around the vertex)
                        let (ix, iz) = ((gx * STEP + dx).saturating_sub(STEP / 2).min(w - 1), (gz * STEP + dz).saturating_sub(STEP / 2).min(h - 1));
                        let j = iz * w + ix;
                        sum += y[j];
                        for k in 0..3 {
                            sn[k] += night_full[j][k] as u32;
                        }
                        n += 1;
                    }
                }
                height[gz * gw + gx] = sum / n as f32;
                night[gz * gw + gx] = [(sn[0] / n) as u8, (sn[1] / n) as u8, (sn[2] / n) as u8];
            }
        }
        let fine_min = y.iter().cloned().fold(f32::INFINITY, f32::min);
        let fine: Vec<u16> = y.iter().map(|v| ((v - fine_min) / FINE_Q).round().clamp(0.0, 65535.0) as u16).collect();
        let map = CityMap { w, h, min_x, min_z, color, gw, gh, height, night, fine, fine_min };
        log::info!("city map: {} sectors drawn into {} x {} texels in {:.1} s", sectors.len(), w, h, t0.elapsed().as_secs_f32());
        if let Err(e) = map.save(assets) {
            log::warn!("city map: could not be cached: {e:#}");
        }
        map
    }

    /// Height of the map's surface at a place.
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let gx = (((x - self.min_x) / (RES * STEP as f32)).round().max(0.0) as usize).min(self.gw - 1);
        let gz = (((z - self.min_z) / (RES * STEP as f32)).round().max(0.0) as usize).min(self.gh - 1);
        self.height[gz * self.gw + gx]
    }

    pub fn center(&self) -> Vec3 {
        Vec3::new(self.min_x + self.w as f32 * RES * 0.5, 0.0, self.min_z + self.h as f32 * RES * 0.5)
    }

    pub fn size(&self) -> (f32, f32) {
        (self.w as f32 * RES, self.h as f32 * RES)
    }

    /// The relief as a mesh with the colour image laid over it.
    pub fn mesh(&self) -> (Vec<Vertex>, Vec<u32>) {
        let cell = RES * STEP as f32;
        let mut verts = Vec::with_capacity(self.gw * self.gh);
        let hgt = |gx: i64, gz: i64| self.height[(gz.clamp(0, self.gh as i64 - 1) as usize) * self.gw + gx.clamp(0, self.gw as i64 - 1) as usize];
        for gz in 0..self.gh {
            for gx in 0..self.gw {
                let (x, z) = (self.min_x + gx as f32 * cell, self.min_z + gz as f32 * cell);
                let y = self.height[gz * self.gw + gx];
                // (walls are softened so that a block's sides shade without going black)
                let (dx, dz) = (hgt(gx as i64 + 1, gz as i64) - hgt(gx as i64 - 1, gz as i64), hgt(gx as i64, gz as i64 + 1) - hgt(gx as i64, gz as i64 - 1));
                let n = Vec3::new(-dx * 0.35, 2.0 * cell, -dz * 0.35).normalize();
                let nc = self.night[gz * self.gw + gx];
                verts.push(Vertex {
                    pos: [x, y, z],
                    normal: n.to_array(),
                    uv0: [(gx * STEP) as f32 / self.w as f32, (gz * STEP) as f32 / self.h as f32],
                    uv1: [0.0, 0.0],
                    color: [255, 255, 255, 0],
                    color1: [nc[0], nc[1], nc[2], 255],
                    uv2: [0.0, 0.0],
                });
            }
        }
        let mut idx = Vec::with_capacity((self.gw - 1) * (self.gh - 1) * 6);
        for gz in 0..self.gh - 1 {
            for gx in 0..self.gw - 1 {
                let a = (gz * self.gw + gx) as u32;
                let (b, c, d) = (a + 1, a + self.gw as u32, a + self.gw as u32 + 1);
                idx.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        (verts, idx)
    }

    fn fine_at(&self, ix: i64, iz: i64) -> f32 {
        let (ix, iz) = (ix.clamp(0, self.w as i64 - 1) as usize, iz.clamp(0, self.h as i64 - 1) as usize);
        self.fine_min + self.fine[iz * self.w + ix] as f32 * FINE_Q
    }

    /// A patch of the relief with a vertex every `step` texels around a place (`half` metres
    /// each way), for the close views.
    pub fn patch(&self, cx: f32, cz: f32, half: f32, step: usize) -> (Vec<Vertex>, Vec<u32>) {
        let step = step.max(1);
        let to_ix = |x: f32| ((x - self.min_x) / RES) as i64;
        let (x0, x1) = ((to_ix(cx - half).max(0) as usize / step) * step, (to_ix(cx + half).clamp(0, self.w as i64 - 1) as usize / step) * step);
        let to_iz = |z: f32| ((z - self.min_z) / RES) as i64;
        let (z0, z1) = ((to_iz(cz - half).max(0) as usize / step) * step, (to_iz(cz + half).clamp(0, self.h as i64 - 1) as usize / step) * step);
        if x1 <= x0 + step || z1 <= z0 + step {
            return (Vec::new(), Vec::new());
        }
        let (nx, nz) = ((x1 - x0) / step + 1, (z1 - z0) / step + 1);
        let mut verts = Vec::with_capacity(nx * nz);
        let s = step as i64;
        for kz in 0..nz {
            for kx in 0..nx {
                let (ix, iz) = ((x0 + kx * step) as i64, (z0 + kz * step) as i64);
                let y = self.fine_at(ix, iz);
                let (dx, dz) = (self.fine_at(ix + s, iz) - self.fine_at(ix - s, iz), self.fine_at(ix, iz + s) - self.fine_at(ix, iz - s));
                let n = Vec3::new(-dx * 0.3, 2.0 * RES * step as f32, -dz * 0.3).normalize();
                let g = ((iz as usize / STEP).min(self.gh - 1)) * self.gw + (ix as usize / STEP).min(self.gw - 1);
                let nc = self.night[g];
                verts.push(Vertex {
                    pos: [self.min_x + (ix as f32 + 0.5) * RES, y, self.min_z + (iz as f32 + 0.5) * RES],
                    normal: n.to_array(),
                    uv0: [(ix as f32 + 0.5) / self.w as f32, (iz as f32 + 0.5) / self.h as f32],
                    uv1: [0.0, 0.0],
                    color: [255, 255, 255, 0],
                    color1: [nc[0], nc[1], nc[2], 255],
                    uv2: [0.0, 0.0],
                });
            }
        }
        let mut idx = Vec::with_capacity((nx - 1) * (nz - 1) * 6);
        for kz in 0..nz - 1 {
            for kx in 0..nx - 1 {
                let a = (kz * nx + kx) as u32;
                let (b, c, d) = (a + 1, a + nx as u32, a + nx as u32 + 1);
                idx.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        (verts, idx)
    }

    /// RGBA8 copy of the colour image for the GPU.
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.w * self.h * 4);
        for c in self.color.chunks_exact(3) {
            out.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
        out
    }
}

/// Load the cached map or draw it, off the main thread.
pub fn spawn(assets: Arc<Assets>, sectors: Vec<SectorInfo>, shared: Arc<SharedTextures>) -> std::sync::mpsc::Receiver<CityMap> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let map = CityMap::load(&assets).unwrap_or_else(|| CityMap::bake(&assets, &sectors, &shared));
        let _ = tx.send(map);
    });
    rx
}
