//! Drivers and riders: the `drv_*_lod02_set` characters posed with each car's own driver
//! animations (`resources/animation/<hash>/opponent/male/*.r2`: one pack per car with
//! `<CAR>_Drv_steer_sweep`, `_Drv_Idle_stp`, `_Drv_Idle_Low`, `_Drv_rev`, ...). The
//! animations are authored in car space, so the root track seats the driver.

use crate::assets::load_resource_file;
use crate::render::{Frustum, Gpu};
use crate::skin::{blend_pose, Clip, SkinBuffer, SkinModel};
use glam::{Mat4, Quat, Vec3};
use hashbrown::HashMap;
use mcla_formats::rage::anim::{animation_named, pack_entries};
use mcla_formats::rage::Rd;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// What the driver of one car needs to know this frame.
pub struct Seat<'a> {
    pub id: usize,
    pub car: &'a str,
    pub mat: Mat4,
    pub pos: Vec3,
    /// -1 left .. 1 right
    pub steer: f32,
    pub speed: f32,
    pub reversing: bool,
    pub player: bool,
    pub cop: bool,
    pub bike: bool,
    /// step 19d: the character set this driver is ("drv_mb_009_set"), else empty
    pub set: &'a str,
}

struct Rig {
    model: Arc<SkinModel>,
    steer: Clip,
    idle_stop: Option<Clip>,
    reverse: Option<Clip>,
}

#[derive(Default, Clone, Copy)]
struct Pose {
    steer: f32,
    stopped: f32,
    reverse: f32,
    seen: u32,
}

#[derive(Default)]
pub struct Drivers {
    char_dir: PathBuf,
    /// car key ("frd_mustang_06", "vpd_chv_impala_64") -> animation pack
    packs: HashMap<String, PathBuf>,
    /// driver sets: (name, bike rider, police)
    sets: Vec<(String, bool, bool)>,
    models: HashMap<usize, Option<Arc<SkinModel>>>,
    rigs: HashMap<(String, usize), Option<Rig>>,
    poses: HashMap<usize, Pose>,
    buf: SkinBuffer,
    /// the player's head in car space (for the cockpit camera)
    pub player_head: Option<Vec3>,
    /// the camera sits in the player's head
    pub headless: bool,
    time: f32,
    frame: u32,
}

/// "vp_frd_mustang_06_dub" -> "frd_mustang_06"; DLC cars keep their "vpd_" prefix.
fn car_key(name: &str) -> String {
    let n = name.to_ascii_lowercase();
    let n = n.trim_end_matches("_dub");
    n.strip_prefix("vp_").unwrap_or(n).to_string()
}

fn hash(a: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E37_79B9);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^ (h >> 13)
}

impl Drivers {
    pub fn load(&mut self, data_dir: &str) {
        let root = Path::new(data_dir);
        self.char_dir = root.join("resources/character");
        let mut names: Vec<String> = std::fs::read_dir(&self.char_dir).map(|d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).collect()).unwrap_or_default();
        names.sort();
        for n in names {
            let l = n.to_ascii_lowercase();
            // the in-car versions; male ones (the packs used here are the male set)
            if !(l.ends_with("lod02_set") || l.ends_with("lod2_set")) || !(l.starts_with("drv_m") || l.starts_with("pol_")) {
                continue;
            }
            self.sets.push((n, l.contains("_mot_"), l.starts_with("pol_")));
        }
        // which pack belongs to which car: read once, then kept in a small index file
        let index = root.join("driver_packs.tsv");
        if let Ok(t) = std::fs::read_to_string(&index) {
            for line in t.lines() {
                if let Some((k, f)) = line.split_once('\t') {
                    self.packs.insert(k.to_string(), PathBuf::from(f));
                }
            }
        }
        if self.packs.is_empty() {
            let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("resources/animation")).map(|d| d.flatten().map(|e| e.path().join("opponent/male")).filter(|p| p.is_dir()).collect()).unwrap_or_default();
            dirs.sort();
            let mut out = String::new();
            for dir in dirs {
                let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
                files.sort();
                for f in files {
                    let Ok(res) = load_resource_file(&f) else { continue };
                    let mut count: HashMap<String, u32> = HashMap::new();
                    for (_, name) in pack_entries(&res) {
                        let l = name.to_ascii_lowercase();
                        let Some(i) = l.find("_fdrv_").or_else(|| l.find("_drv_")) else { continue };
                        *count.entry(l[..i].to_string()).or_default() += 1;
                    }
                    // every pack also carries the shared Challenger gestures
                    let mut v: Vec<(String, u32)> = count.into_iter().collect();
                    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                    let key = v.iter().find(|(k, n)| k != "dge_challenger_70" && *n >= 10).or(v.first()).map(|(k, _)| k.clone());
                    if let Some(k) = key {
                        if !self.packs.contains_key(&k) {
                            out.push_str(&format!("{}\t{}\n", k, f.display()));
                            self.packs.insert(k, f);
                        }
                    }
                }
            }
            let _ = std::fs::write(&index, out);
        }
        log::info!("drivers: {} character sets, {} animation packs", self.sets.len(), self.packs.len());
    }

    fn model(&mut self, gpu: &mut Gpu, si: usize) -> Option<Arc<SkinModel>> {
        if !self.models.contains_key(&si) {
            let name = &self.sets[si].0;
            let dir = self.char_dir.join(name);
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).map(|d| d.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == "r6").unwrap_or(false)).collect()).unwrap_or_default();
            files.sort();
            let m = files.first().and_then(|f| SkinModel::load(gpu, name, f)).map(Arc::new);
            if m.is_none() {
                log::warn!("drivers: could not load {}", name);
            }
            self.models.insert(si, m);
        }
        self.models[&si].clone()
    }

    fn pick_set(&self, s: &Seat) -> Option<usize> {
        // (step 19d: a driver who was named - Racer_SetCharacterName - is that character)
        if !s.set.is_empty() {
            if let Some(i) = self.sets.iter().position(|n| n.0.eq_ignore_ascii_case(s.set)) {
                return Some(i);
            }
        }
        let want = |n: &(String, bool, bool)| n.1 == s.bike && n.2 == (s.cop && !s.bike);
        if s.player {
            // "mp" is the player character
            if let Some(i) = self.sets.iter().position(|n| n.0.to_ascii_lowercase().starts_with("drv_mp_") && n.1 == s.bike) {
                return Some(i);
            }
        }
        let c: Vec<usize> = (0..self.sets.len()).filter(|&i| want(&self.sets[i]) && !self.sets[i].0.to_ascii_lowercase().starts_with("drv_mp_")).collect();
        if c.is_empty() {
            return self.sets.iter().position(|n| n.1 == s.bike);
        }
        Some(c[hash(s.id as u32 + 17) as usize % c.len()])
    }

    fn rig(&mut self, gpu: &mut Gpu, car: &str, si: usize) -> bool {
        let key = (car_key(car), si);
        if !self.rigs.contains_key(&key) {
            let rig = (|| -> Option<Rig> {
                let model = self.model(gpu, si)?;
                let pack = self.packs.get(&key.0).or_else(|| self.packs.get("dge_challenger_70"))?.clone();
                let res = load_resource_file(&pack).ok()?;
                let rd = Rd::new(&res);
                let entries = pack_entries(&res);
                let clip = |suffixes: &[&str]| -> Option<Clip> {
                    for s in suffixes {
                        // the car's own clip first, then the shared one
                        let mut hit = entries.iter().filter(|(_, n)| n.to_ascii_lowercase().ends_with(s)).collect::<Vec<_>>();
                        hit.sort_by_key(|(_, n)| !n.to_ascii_lowercase().starts_with(&key.0));
                        if let Some((o, _)) = hit.first() {
                            return animation_named(&rd, *o).map(|a| Clip::bind(&Arc::new(a), &model.ids, false, false));
                        }
                    }
                    None
                };
                let steer = clip(&["drv_steer_sweep", "drv_low_lean_sweep", "drv_idle_stp_sweep"])?;
                Some(Rig { steer, idle_stop: clip(&["drv_idle_stp"]), reverse: clip(&["drv_rev"]), model })
            })();
            if rig.is_none() {
                log::warn!("drivers: no animations for {}", car);
            }
            self.rigs.insert(key.clone(), rig);
        }
        self.rigs[&key].is_some()
    }

    /// Pose and skin the drivers of the cars near the camera.
    pub fn prepare(&mut self, gpu: &mut Gpu, seats: &[Seat], cam: Vec3, frustum: &Frustum, dt: f32) {
        self.buf.clear();
        self.time += dt;
        self.frame += 1;
        let mut local: Vec<(Quat, Vec3)> = Vec::new();
        let mut other: Vec<(Quat, Vec3)> = Vec::new();
        for s in seats {
            let dist = s.pos.distance(cam);
            // ambient drivers only matter close up
            let reach = if s.car.starts_with("va_") { 40.0 } else { 70.0 };
            if dist > reach || !frustum.intersects_aabb(s.pos - Vec3::splat(2.5), s.pos + Vec3::splat(2.5)) {
                continue;
            }
            let Some(si) = self.pick_set(s) else { continue };
            if !self.rig(gpu, s.car, si) {
                continue;
            }
            let frame = self.frame;
            let p = self.poses.entry(s.id).or_default();
            if p.seen + 1 != frame {
                *p = Pose { steer: s.steer, ..Default::default() };
            }
            p.seen = frame;
            let k = (dt * 8.0).min(1.0);
            p.steer += (s.steer - p.steer) * k;
            p.stopped += ((if s.speed.abs() < 0.6 { 1.0 } else { 0.0 }) - p.stopped) * (dt * 3.0).min(1.0);
            p.reverse += ((if s.reversing && s.speed.abs() > 0.3 { 1.0 } else { 0.0 }) - p.reverse) * (dt * 4.0).min(1.0);
            let p = *p;
            let Some(Some(rig)) = self.rigs.get(&(car_key(s.car), si)) else { continue };
            // the sweep runs from full left lock to full right lock
            let last = rig.steer.last_frame();
            rig.steer.local(&rig.model.bones, (0.5 + 0.5 * p.steer.clamp(-1.0, 1.0)) * last, &mut local);
            if let Some(idle) = rig.idle_stop.as_ref() {
                if p.stopped > 0.01 {
                    idle.local(&rig.model.bones, idle.frame(self.time + s.id as f32 * 0.37, true), &mut other);
                    blend_pose(&mut local, &other, p.stopped);
                }
            }
            if let Some(rev) = rig.reverse.as_ref() {
                if p.reverse > 0.01 {
                    rev.local(&rig.model.bones, rev.last_frame(), &mut other);
                    blend_pose(&mut local, &other, p.reverse);
                }
            }
            // (the cockpit view shows the arms and the wheel, not the driver's own head -
            // `ref/cockpit_view.png`)
            if s.player && self.headless {
                self.buf.hide_from = rig.model.head;
            }
            self.buf.emit(&rig.model, 0, &local, s.mat);
            if s.player {
                self.player_head = rig.model.head.and_then(|h| self.buf.bone_pos(h)).map(|w| s.mat.inverse().transform_point3(w));
            }
        }
        if self.frame == 50 {
            log::info!("drivers: {} seats, {} rigs, {} poses", seats.len(), self.rigs.len(), self.poses.len());
            for s in seats {
                log::info!("  seat {} {} dist {:.1}", s.id, s.car, s.pos.distance(cam));
            }
        }
        self.buf.upload(gpu);
    }

    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, draws: &mut usize) {
        self.buf.draw(pass, gpu, false, draws);
    }
}
