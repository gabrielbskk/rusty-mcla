//! Ambient pedestrians: the game's own character sets (`resources/character/ped_*_set`)
//! skinned on the CPU and driven by the original animations
//! (`resources/ambients/<set>/*.r189`: walk, idle, run; `resources/ambients/*.r198`:
//! the shared evade / angry clips listed in `tune/ambients/peds/mcAvoidAnimMgrData.xml`).
//!
//! They walk the sidewalk polygons of the original's navigation mesh (`city.xsm`, material
//! flag bit 0 - what `mcAmbientPed` / `mcSidewalkFilter` accept), from polygon to polygon
//! through the mesh's own links; the route choice is the port's (the original's path queries
//! are not decoded). Without the mesh file, or with `MCLA_PED_KERB=1`, they stroll along the
//! kerb of the outermost traffic lane as before. They jump out of the way of approaching
//! cars (they are never hit: `bCollideWithPlayer` is false in the tuning) and shout their
//! evade lines from the `ped_*` speech banks.

use crate::ambient::{xml_value, Distribution};
use crate::assets::load_resource_file;
use crate::physics::collision::WorldCollider;
use crate::render::{Frustum, Gpu};
use crate::skin::{blend_pose, Clip, SkinBuffer, SkinModel};
use glam::{Mat4, Quat, Vec3};
use hashbrown::{HashMap, HashSet};
use mcla_formats::rage::anim::{find_animations, Animation};
use mcla_formats::rage::navmesh::{NavLink, NavMesh, AI_ROAD, AI_SIDEWALK};
use mcla_formats::rage::navsys::NavSys;
use std::path::{Path, PathBuf};
use std::sync::Arc;

struct PedType {
    female: bool,
    /// speech bank key, e.g. "fh_006"
    voice: String,
    model: SkinModel,
    walk: Clip,
    idle: Clip,
    evade: Vec<Clip>,
    angry: Vec<Clip>,
    lod_switch: [f32; 2],
}

impl PedType {
    fn clip(&self, s: State) -> (&Clip, bool) {
        match s {
            State::Walk => (&self.walk, true),
            State::Idle => (&self.idle, true),
            State::Evade(k) => (&self.evade[k], false),
            State::Angry(k) => (&self.angry[k], false),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Walk,
    Idle,
    Evade(usize),
    Angry(usize),
}

struct Ped {
    ty: usize,
    key: (usize, u32),
    pos: Vec3,
    rot: Quat,
    link: usize,
    fwd: bool,
    lat: f32,
    state: State,
    time: f32,
    /// local pose of the previous state, cross-faded out over `blend` seconds
    from: Vec<(Quat, Vec3)>,
    blend: f32,
    cur: Vec<(Quat, Vec3)>,
    /// stands around instead of walking
    loiter: bool,
    calm: f32,
    seed: u32,
    /// what the walker wants to do at the corner it has reached
    plan: Option<Plan>,
    /// crossing the street: kerb point over there, then the link and offset to walk on
    cross: Option<(Vec3, usize, f32)>,
    /// seconds spent waiting at the kerb
    wait: f32,
    /// on the navigation mesh: the sidewalk polygon walked on (`NO_POLY`: kerb walker)
    poly: u32,
    /// the polygon come from and the one behind the edge walked towards
    prev: u32,
    next: u32,
    /// the stretch being walked: from here to the point on the edge into `next`
    entry: Vec3,
    goal: Vec3,
    /// direction of the stretch before
    travel: Vec3,
    /// where across the pavement this one likes to walk (0 left .. 1 right)
    side: f32,
}

const NO_POLY: u32 = u32::MAX;
/// Width of pavement one "metre of street" of the density tuning stands for (the tuning
/// counts pedestrians per metre; the mesh gives areas). Invented.
const PAVEMENT_WIDTH: f32 = 4.0;

fn flat(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

/// Point on a shared edge to walk through, `side` of the way from its left end to its right
/// end as seen from `from`, at least 0.6 m from the ends where the edge is wide enough.
fn portal_point(l: &NavLink, from: Vec3, side: f32) -> Vec3 {
    let mid = (l.a + l.b) * 0.5;
    let d = mid - from;
    let right = Vec3::new(-d.z, 0.0, d.x);
    let (lft, rgt) = if (l.a - mid).dot(right) > 0.0 { (l.b, l.a) } else { (l.a, l.b) };
    let len = l.a.distance(l.b);
    let m = if len > 1.6 { 0.6 / len } else { 0.5 };
    lft.lerp(rgt, m + (1.0 - 2.0 * m) * side)
}

/// Where to go from polygon `cur`, entered from `from`: one of its other sidewalk
/// neighbours, the ones straight on preferred; `(polygon, point on the edge, false)`.
/// At a dead end `(cur, centre of cur, true)`: walk up to the middle, then choose again
/// with nothing excluded - which leads back.
fn mesh_next(mesh: &NavMesh, cur: u32, from: u32, pos: Vec3, heading: Vec3, side: f32, r: f32) -> (u32, Vec3, bool) {
    let mut cands: Vec<(f32, u32, Vec3)> = Vec::new();
    let mut total = 0.0;
    for l in mesh.links_of(cur) {
        if l.poly == from || l.poly == cur || !mesh.polys[l.poly as usize].sidewalk() {
            continue;
        }
        let g = portal_point(l, pos, side);
        let w = 0.15 + flat(g - pos).normalize_or_zero().dot(heading).max(0.0);
        total += w;
        cands.push((w, l.poly, g));
    }
    let mut pick = r * total;
    for (k, c) in cands.iter().enumerate() {
        pick -= c.0;
        if pick <= 0.0 || k + 1 == cands.len() {
            return (c.1, c.2, false);
        }
    }
    (cur, mesh.polys[cur as usize].center, true)
}

/// A way over the street from `pos` on polygon `cur`: the nearest pavement of another
/// island (7 .. 34 m away, as the kerb walkers cross) that lies across the direction of
/// travel or straight on, with nothing but road and pavement on the line to it.
/// `(point over there, its polygon, straight on)`.
fn mesh_crossing(mesh: &NavMesh, island: &[u32], cur: u32, pos: Vec3, travel: Vec3) -> Option<(Vec3, u32, bool)> {
    let my = island[cur as usize];
    let mut cands: Vec<(f32, Vec3, u32, bool)> = Vec::new();
    mesh.near(pos, 34.0, |i, q| {
        if !q.sidewalk() || island[i as usize] == my {
            return;
        }
        // nearest point of its outline, then a metre in
        let pts = q.points();
        let mut c = (f32::MAX, q.center);
        for k in 0..pts.len() {
            let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
            let ab = b - a;
            let t = (flat(pos - a).dot(flat(ab)) / flat(ab).length_squared().max(1e-6)).clamp(0.0, 1.0);
            let e = a + ab * t;
            let d = flat(e - pos).length_squared();
            if d < c.0 {
                c = (d, e);
            }
        }
        let c = c.1 + (q.center - c.1).clamp_length_max(1.0);
        let rel = flat(c - pos);
        let d = rel.length();
        if d < 7.0 || d > 34.0 || (c.y - pos.y).abs() > 2.0 {
            return;
        }
        let along = rel.dot(travel);
        let straight = along > d * 0.8;
        if straight || along.abs() < d * 0.45 {
            cands.push((d, c, i, straight));
        }
    });
    cands.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (d, c, i, straight) in cands.into_iter().take(4) {
        let n = d.ceil().max(2.0) as usize;
        if (1..n).all(|k| mesh.find(pos.lerp(c, k as f32 / n as f32), AI_SIDEWALK | AI_ROAD, 2.5).is_some()) {
            return Some((c, i, straight));
        }
    }
    None
}

#[derive(Clone, Copy)]
enum Plan {
    /// carry on along the next block (over the side street)
    Straight(usize, f32),
    /// over to the opposite pavement of the own street
    Cross(Vec3, usize, f32),
    /// on the navigation mesh: over the street to a point on that polygon (true: straight
    /// on, over the side street)
    Mesh(Vec3, u32, bool),
}

/// A moving car the pedestrians have to watch out for.
#[derive(Clone, Copy)]
pub struct Threat {
    pub pos: Vec3,
    pub vel: Vec3,
}

pub struct Speech {
    pub pos: Vec3,
    pub voice: String,
    pub female: bool,
    pub fast: bool,
}

struct Tuning {
    spawn_inner: f32,
    spawn_outer: f32,
    unspawn: f32,
    density: f32,
    type_spacing: f32,
}

pub struct Peds {
    districts: Distribution,
    hour: f32,
    dirs: Vec<(String, PathBuf, PathBuf)>,
    tune_dir: PathBuf,
    types: HashMap<usize, Option<PedType>>,
    avoid: Vec<(Arc<Animation>, bool, bool, u8)>,
    list: Vec<Ped>,
    active: HashSet<(usize, u32)>,
    curb: HashMap<usize, Option<f32>>,
    tune: Tuning,
    timer: f32,
    filled: bool,
    rng: u32,
    pub speech: Vec<Speech>,
    /// streets crossed so far (diagnostics)
    pub crossings: u32,
    /// evades started so far (diagnostics)
    pub evades: u32,
    evade_radius: f32,
    voices: Option<HashMap<String, String>>,
    buf: SkinBuffer,
    pub enabled: bool,
    /// the original's navigation mesh; without it the pedestrians follow the kerbs
    mesh: Option<NavMesh>,
    /// connected piece of pavement of every mesh polygon
    island: Vec<u32>,
}

fn hash(a: u32, b: u32) -> u32 {
    let mut h = a.wrapping_mul(0x9E37_79B9) ^ b.wrapping_mul(0x85EB_CA6B).rotate_left(13);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^ (h >> 16)
}

fn wrap_angle(mut d: f32) -> f32 {
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    d
}

impl Peds {
    pub fn new() -> Peds {
        Peds {
            districts: Distribution::default(),
            hour: 12.0,
            dirs: Vec::new(),
            tune_dir: PathBuf::new(),
            types: HashMap::new(),
            avoid: Vec::new(),
            list: Vec::new(),
            active: HashSet::new(),
            curb: HashMap::new(),
            tune: Tuning { spawn_inner: 95.0, spawn_outer: 140.0, unspawn: 150.0, density: 0.011, type_spacing: 20.0 },
            timer: 0.0,
            filled: false,
            rng: 0x1234_5678,
            speech: Vec::new(),
            evades: 0,
            crossings: 0,
            evade_radius: std::env::var("MCLA_PED_EVADE").ok().and_then(|v| v.parse().ok()).unwrap_or(2.3),
            voices: None,
            buf: SkinBuffer::default(),
            enabled: true,
            mesh: None,
            island: Vec::new(),
        }
    }

    fn rand(&mut self) -> f32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.rng >> 8) as f32 / 16777216.0
    }

    /// Index the pedestrian sets and load the shared evade animations.
    pub fn load(&mut self, data_dir: &str) {
        let root = Path::new(data_dir);
        self.tune_dir = root.join("tune/ambients/peds");
        if let Ok(t) = std::fs::read_to_string(root.join("tune/ambients/density_tuning.xml")) {
            let g = |tag: &str, d: f32| xml_value(&t, tag).unwrap_or(d);
            self.tune = Tuning {
                spawn_inner: g("fPedSpawnInnerRadius", 95.0),
                spawn_outer: g("fPedSpawnOuterRadius", 140.0),
                unspawn: g("fPedUnspawnOuterRadius", 150.0),
                density: g("fPedDensity", 0.011),
                type_spacing: g("fMinSpawnPedTypeSpacing", 20.0),
            };
        }
        let first = |dir: &Path, ext: &str| -> Option<PathBuf> {
            let mut v: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().map(|e| e == ext).unwrap_or(false)).collect();
            v.sort();
            v.into_iter().next()
        };
        let mut names: Vec<String> = std::fs::read_dir(root.join("resources/ambients")).map(|d| d.flatten().filter_map(|e| e.file_name().into_string().ok()).collect()).unwrap_or_default();
        names.sort();
        for n in names {
            if !n.starts_with("ped_") {
                continue;
            }
            let (Some(model), Some(anims)) = (first(&root.join("resources/character").join(&n), "r6"), first(&root.join("resources/ambients").join(&n), "r189")) else { continue };
            self.dirs.push((n, model, anims));
        }
        // shared evade / angry clips, classified by the game's own list
        let mut class: HashMap<String, (bool, bool, u8)> = HashMap::new();
        if let Ok(t) = std::fs::read_to_string(self.tune_dir.join("mcAvoidAnimMgrData.xml")) {
            for item in t.split("<Item>").skip(1) {
                let field = |tag: &str| -> String {
                    let open = format!("<{tag}>");
                    let Some(i) = item.find(&open) else { return String::new() };
                    let rest = &item[i + open.len()..];
                    rest.find('<').map(|e| rest[..e].trim().to_string()).unwrap_or_default()
                };
                let name = field("Name").rsplit('/').next().unwrap_or("").trim_end_matches(".anim").to_ascii_lowercase();
                let kind = match field("AvoidType").as_str() {
                    "AvoidMale" | "AvoidFemale" => 0,
                    "AngryMale" | "AngryFemale" => 1,
                    _ => 2,
                };
                class.insert(name, (field("AudioType") == "Fast", field("ApproachDir") == "Front", kind));
            }
        }
        if let Ok(res) = load_resource_file(&root.join("resources/ambients/0x364e4eea.r198")) {
            for a in find_animations(&res) {
                let (fast, front, kind) = class.get(&a.short()).copied().unwrap_or((false, false, 2));
                self.avoid.push((Arc::new(a), fast, front, kind));
            }
        }
        self.districts = Distribution::load(root, "ped_");
        // the navigation mesh (city.xsm); MCLA_PED_KERB=1 keeps the kerb walkers
        if std::env::var("MCLA_PED_KERB").is_err() {
            self.mesh = load_resource_file(&root.join("resources/city/sc/0x69d17d57.r26")).ok().and_then(|r| NavMesh::parse(&r));
            match &self.mesh {
                Some(m) => {
                    let (island, n) = m.sidewalk_islands();
                    log::info!("peds: nav mesh {} polygons, {} sidewalk in {} islands", m.polys.len(), island.iter().filter(|&&i| i != u32::MAX).count(), n);
                    self.island = island;
                }
                None => log::warn!("peds: no navigation mesh, walking the kerbs"),
            }
        }
        log::info!("peds: {} types, {} shared clips, {} districts", self.dirs.len(), self.avoid.len(), self.districts.types.len());
    }

    fn build_type(&self, gpu: &mut Gpu, ti: usize) -> Option<PedType> {
        let (name, model, anims) = &self.dirs[ti];
        let model = SkinModel::load(gpu, name, model)?;
        let own: Vec<Arc<Animation>> = find_animations(&load_resource_file(anims).ok()?).into_iter().map(Arc::new).collect();
        let pick = |what: &str| own.iter().find(|a| a.short().contains(what));
        let walk = Clip::bind(pick("walk")?, &model.ids, false, false);
        let idle = Clip::bind(pick("idle").or(pick("walk"))?, &model.ids, false, false);
        // "ped_fh_006_set" -> female, voice "fh_006"
        let core = name.trim_start_matches("ped_").trim_end_matches("_set").to_string();
        let female = core.trim_start_matches("sc_").starts_with('f');
        let gender = if female { "fped" } else { "mped" };
        let mut evade = Vec::new();
        let mut angry = Vec::new();
        for (a, fast, front, kind) in &self.avoid {
            if !a.short().starts_with(gender) {
                continue;
            }
            match kind {
                0 => evade.push(Clip::bind(a, &model.ids, *fast, *front)),
                1 => angry.push(Clip::bind(a, &model.ids, *fast, *front)),
                _ => {}
            }
        }
        let mut lod_switch = [40.0, 80.0];
        if let Ok(t) = std::fs::read_to_string(self.tune_dir.join(name).join("mcAmbientRenderType.xml")) {
            let s = xml_value(&t, "fLodSwitchScale").unwrap_or(1.0);
            lod_switch = [xml_value(&t, "fHighToMedSwitch").unwrap_or(40.0) * s, xml_value(&t, "fMedToLowSwitch").unwrap_or(80.0) * s];
        }
        Some(PedType { female, voice: core, model, walk, idle, evade, angry, lod_switch })
    }

    fn load_type(&mut self, gpu: &mut Gpu, ti: usize) -> bool {
        if !self.types.contains_key(&ti) {
            let t = self.build_type(gpu, ti);
            if t.is_none() {
                log::warn!("peds: could not load {}", self.dirs[ti].0);
            }
            self.types.insert(ti, t);
        }
        self.types.get(&ti).map(|t| t.is_some()).unwrap_or(false)
    }

    /// Distance from the lane centre to the middle of the pavement on its right, if there
    /// is a kerb within reach (only the outermost lane has one).
    fn curb_offset(&mut self, nav: &NavSys, col: &WorldCollider, li: usize) -> Option<f32> {
        if let Some(c) = self.curb.get(&li) {
            return *c;
        }
        let l = &nav.links[li];
        if l.len < 6.0 {
            return None;
        }
        let right = l.dir().cross(Vec3::Y).normalize_or_zero();
        // only the outermost lane borders the pavement
        let mid = (l.a + l.b) * 0.5;
        if l.side.iter().any(|&n| ((nav.links[n].a + nav.links[n].b) * 0.5 - mid).dot(right) > 1.0) {
            self.curb.insert(li, None);
            return None;
        }
        let mut found: Option<f32> = None;
        let mut votes = 0;
        // the kerb has to be there at several points along the link
        for t in [0.25f32, 0.5, 0.75] {
            let p = l.a.lerp(l.b, t);
            // collision not streamed in yet: decide later
            let h0 = col.ground(p, 1.0, 2.5)?;
            let mut prev = h0.point.y;
            let mut off = 1.0;
            while off <= 11.0 {
                let q = p + right * off;
                let Some(h) = col.ground(Vec3::new(q.x, prev, q.z), 0.8, 1.2) else { break };
                let step = h.point.y - prev;
                if (0.06..0.4).contains(&step) {
                    // room to walk behind the kerb?
                    let q2 = p + right * (off + 1.4);
                    if let Some(h2) = col.ground(Vec3::new(q2.x, h.point.y, q2.z), 0.5, 0.5) {
                        if (h2.point.y - h.point.y).abs() < 0.15 && h2.normal.y > 0.95 {
                            votes += 1;
                            found = Some(found.map(|f: f32| f.max(off + 1.2)).unwrap_or(off + 1.2));
                        }
                    }
                    break;
                }
                if step.abs() > 0.4 {
                    break;
                }
                prev = h.point.y;
                off += 0.4;
            }
        }
        if std::env::var("MCLA_PED_KERB").is_ok() && self.curb.len() < 40 {
            let p = l.a.lerp(l.b, 0.5);
            let prof: Vec<String> = (0..24).map(|k| col.ground(p + right * (k as f32 * 0.5) + Vec3::Y * 0.5, 1.0, 3.0).map(|h| format!("{:.2}", h.point.y - p.y)).unwrap_or("-".into())).collect();
            log::info!("kerb link {} len {:.0} sides {} votes {} found {:?}: {}", li, l.len, l.side.len(), votes, found, prof.join(" "));
        }
        let r = if votes >= 2 { found } else { None };
        self.curb.insert(li, r);
        r
    }

    /// A pedestrian set for this place and hour, from the district's distribution.
    fn pick_type(&self, pos: Vec3, h: u32) -> Option<usize> {
        if !self.districts.loaded() {
            // no district data at all: anything goes
            return Some(h as usize % self.dirs.len());
        }
        let r = (h & 0xffffff) as f32 / 16777216.0;
        let name = self.districts.pick(pos, self.hour, r, &|n| n.starts_with("ped_"))?;
        self.dirs.iter().position(|d| d.0.eq_ignore_ascii_case(name))
    }

    pub fn clear(&mut self) {
        self.list.clear();
        self.active.clear();
        self.filled = false;
    }

    /// Positions and headings of the pedestrians closest to `p` (diagnostics).
    pub fn nearest(&self, p: Vec3, n: usize) -> Vec<(Vec3, Vec3, String)> {
        let mut v: Vec<&Ped> = self.list.iter().collect();
        v.sort_by(|a, b| a.pos.distance_squared(p).partial_cmp(&b.pos.distance_squared(p)).unwrap());
        v.into_iter().take(n).map(|q| (q.pos, q.rot * -Vec3::Z, self.dirs[q.ty].0.clone())).collect()
    }

    /// (links with a pavement, links without) among those examined so far.
    pub fn curb_stats(&self) -> (usize, usize) {
        let yes = self.curb.values().filter(|c| c.is_some()).count();
        (yes, self.curb.len() - yes)
    }

    pub fn count(&self) -> usize {
        self.list.len()
    }

    /// Walkers standing at a kerb waiting to cross.
    pub fn waiting(&self) -> usize {
        self.list.iter().filter(|p| p.plan.is_some()).count()
    }

    /// Pedestrians on the sidewalk polygons of the navigation mesh inside the spawn ring.
    fn populate_mesh(&mut self, gpu: &mut Gpu, mesh: &NavMesh, col: &WorldCollider, center: Vec3) {
        let unspawn = self.tune.unspawn;
        let active = &mut self.active;
        self.list.retain(|p| {
            let keep = p.pos.distance_squared(center) < unspawn * unspawn;
            if !keep {
                active.remove(&p.key);
            }
            keep
        });
        // the first time everything around is filled in, afterwards only the far ring
        let inner = if self.filled { self.tune.spawn_inner } else { 5.0 };
        let outer = self.tune.spawn_outer;
        let per_m2 = self.tune.density * self.districts.density.at(self.hour) / PAVEMENT_WIDTH;
        let mut polys: Vec<u32> = Vec::new();
        mesh.near(center, outer, |i, p| {
            if p.sidewalk() && (p.center.y - center.y).abs() <= 25.0 {
                polys.push(i);
            }
        });
        let mut any_ground = false;
        let before = self.list.len();
        for pi in polys {
            let q = &mesh.polys[pi as usize];
            let want = q.area * per_m2;
            let n = want.floor() as u32 + if ((hash(pi, 77) & 0xffff) as f32 / 65536.0) < want.fract() { 1 } else { 0 };
            // collision not streamed in yet: later
            if n == 0 || col.ground(q.center, 1.0, 2.5).is_none() {
                continue;
            }
            any_ground = true;
            for s in 0..n {
                let key = (pi as usize, s);
                if self.active.contains(&key) {
                    continue;
                }
                let h = hash(pi, s + 1);
                let mut pos = q.point(0.15 + 0.7 * (h & 0xffff) as f32 / 65536.0, 0.15 + 0.7 * ((h >> 16) & 0xff) as f32 / 255.0);
                pos.y = q.height(pos.x, pos.z);
                let d = pos.distance(center);
                if d < inner || d > outer {
                    continue;
                }
                if let Some(g) = col.ground(pos, 1.0, 1.5) {
                    if (g.point.y - pos.y).abs() < 0.5 {
                        pos.y = g.point.y;
                    }
                }
                // a type that is not already walking nearby
                let sp = self.tune.type_spacing;
                let mut ti = usize::MAX;
                for attempt in 0..4 {
                    let Some(c) = self.pick_type(pos, hash(h, 9 + attempt)) else { break };
                    ti = c;
                    if !self.list.iter().any(|p| p.ty == c && p.pos.distance_squared(pos) < sp * sp) {
                        break;
                    }
                }
                if ti == usize::MAX {
                    continue;
                }
                if !self.load_type(gpu, ti) {
                    continue;
                }
                let side = 0.1 + 0.8 * ((hash(h, 5) & 0xffff) as f32 / 65536.0);
                let (next, goal, dead) = mesh_next(mesh, pi, NO_POLY, pos, Vec3::ZERO, side, (hash(h, 6) & 0xffff) as f32 / 65536.0);
                // nowhere to go from a lone polygon: stand around
                let loiter = dead || (h >> 25) % 5 == 0;
                let face = if loiter {
                    let a = (hash(h, 7) & 0xffff) as f32 / 65536.0 * std::f32::consts::TAU;
                    Vec3::new(a.sin(), 0.0, a.cos())
                } else {
                    flat(goal - pos).normalize_or(Vec3::Z)
                };
                self.active.insert(key);
                self.list.push(Ped {
                    ty: ti,
                    key,
                    pos,
                    rot: Quat::from_rotation_y((-face.x).atan2(-face.z)),
                    link: 0,
                    fwd: true,
                    lat: 0.0,
                    state: if loiter { State::Idle } else { State::Walk },
                    time: ((h >> 8) & 0xff) as f32 / 40.0,
                    from: Vec::new(),
                    blend: 0.0,
                    cur: Vec::new(),
                    loiter,
                    calm: 0.0,
                    seed: h,
                    plan: None,
                    cross: None,
                    wait: 0.0,
                    poly: pi,
                    prev: NO_POLY,
                    next,
                    entry: pos,
                    goal,
                    travel: face,
                    side,
                });
            }
        }
        if any_ground {
            if !self.filled {
                log::info!("peds: {} placed on the navigation mesh around {:.0},{:.0},{:.0}", self.list.len() - before, center.x, center.y, center.z);
            }
            self.filled = true;
        }
    }

    fn populate(&mut self, gpu: &mut Gpu, nav: &NavSys, col: &WorldCollider, center: Vec3) {
        if let Some(mesh) = self.mesh.take() {
            self.populate_mesh(gpu, &mesh, col, center);
            self.mesh = Some(mesh);
            return;
        }
        let unspawn = self.tune.unspawn;
        let active = &mut self.active;
        self.list.retain(|p| {
            let keep = p.pos.distance_squared(center) < unspawn * unspawn;
            if !keep {
                active.remove(&p.key);
            }
            keep
        });
        // the first time everything around is filled in, afterwards only the far ring
        let inner = if self.filled { self.tune.spawn_inner } else { 5.0 };
        let outer = self.tune.spawn_outer;
        let mut any_ground = false;
        let links: Vec<usize> = nav.links_near(center, outer).collect();
        for li in links {
            let l = &nav.links[li];
            let mid = (l.a + l.b) * 0.5;
            if mid.distance(center) > outer + l.len * 0.5 || (mid.y - center.y).abs() > 25.0 {
                continue;
            }
            let want = l.len * self.tune.density * self.districts.density.at(self.hour);
            let n = want.floor() as u32 + if ((hash(li as u32, 77) & 0xffff) as f32 / 65536.0) < want.fract() { 1 } else { 0 };
            if n == 0 || col.ground(mid, 1.0, 2.5).is_none() {
                continue;
            }
            any_ground = true;
            let Some(lat) = self.curb_offset(nav, col, li) else { continue };
            let right = l.dir().cross(Vec3::Y).normalize_or_zero();
            for s in 0..n {
                let key = (li, s);
                if self.active.contains(&key) {
                    continue;
                }
                let h = hash(li as u32, s + 1);
                let t = 0.1 + 0.8 * (h & 0xffff) as f32 / 65536.0;
                let jitter = (((h >> 16) & 0xff) as f32 / 255.0 - 0.5) * 0.8;
                let pos = l.a.lerp(l.b, t) + right * (lat + jitter);
                let d = pos.distance(center);
                if d < inner || d > outer {
                    continue;
                }
                let Some(g) = col.ground(pos, 1.0, 1.5) else { continue };
                // a type that is not already walking nearby
                let sp = self.tune.type_spacing;
                let mut ti = usize::MAX;
                for attempt in 0..4 {
                    let Some(c) = self.pick_type(pos, hash(h, 9 + attempt)) else { break };
                    ti = c;
                    if !self.list.iter().any(|p| p.ty == c && p.pos.distance_squared(pos) < sp * sp) {
                        break;
                    }
                }
                if ti == usize::MAX {
                    continue;
                }
                if !self.load_type(gpu, ti) {
                    continue;
                }
                let fwd = h & 0x0100_0000 != 0;
                let loiter = (h >> 25) % 5 == 0;
                let face = if loiter { -right } else if fwd { l.dir() } else { -l.dir() };
                self.active.insert(key);
                self.list.push(Ped {
                    ty: ti,
                    key,
                    pos: g.point,
                    rot: Quat::from_rotation_y((-face.x).atan2(-face.z)),
                    link: li,
                    fwd,
                    lat: lat + jitter,
                    state: if loiter { State::Idle } else { State::Walk },
                    time: ((h >> 8) & 0xff) as f32 / 40.0,
                    from: Vec::new(),
                    blend: 0.0,
                    cur: Vec::new(),
                    loiter,
                    calm: 0.0,
                    seed: h,
                    plan: None,
                    cross: None,
                    wait: 0.0,
                    poly: NO_POLY,
                    prev: NO_POLY,
                    next: NO_POLY,
                    entry: g.point,
                    goal: g.point,
                    travel: face,
                    side: 0.5,
                });
            }
        }
        if any_ground {
            self.filled = true;
        }
    }

    pub fn update(&mut self, gpu: &mut Gpu, nav: &NavSys, col: &WorldCollider, obstacles: &dyn Fn(Vec3, f32, &mut Vec<(Vec3, f32)>), center: Vec3, threats: &[Threat], signals: Option<(&crate::signals::Signals, f32)>, hour: f32, dt: f32) {
        self.hour = hour;
        if !self.enabled || self.dirs.is_empty() {
            return;
        }
        self.timer -= dt;
        if self.timer <= 0.0 {
            self.timer = 0.4;
            self.populate(gpu, nav, col, center);
        }
        let mut rng = self.rng;
        let mut rand = move || {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            (rng >> 8) as f32 / 16777216.0
        };
        let mut near: Vec<(Vec3, f32)> = Vec::new();
        for i in 0..self.list.len() {
            let Some(Some(ty)) = self.types.get(&self.list[i].ty) else { continue };
            let p = &mut self.list[i];
            p.calm = (p.calm - dt).max(0.0);
            p.blend = (p.blend - dt).max(0.0);
            // a car about to pass too close?
            if matches!(p.state, State::Walk | State::Idle) && p.calm <= 0.0 && !ty.evade.is_empty() {
                for t in threats {
                    let speed = t.vel.length();
                    if speed < 3.0 {
                        continue;
                    }
                    let v = t.vel / speed;
                    let rel = p.pos - t.pos;
                    let along = rel.dot(v);
                    if rel.y.abs() > 4.0 || along < -1.0 || along > speed * 1.2 + 4.0 {
                        continue;
                    }
                    let side = rel - v * along;
                    if side.length() > self.evade_radius {
                        continue;
                    }
                    let away = if side.length() > 0.2 { side.normalize() } else { v.cross(Vec3::Y) };
                    let from_front = (p.rot * -Vec3::Z).dot(v) < -0.4;
                    // of the game's evade clips, one that actually leads out of the car's way
                    let mut best = (f32::MIN, 0usize);
                    for (k, c) in ty.evade.iter().enumerate() {
                        if c.front && !from_front {
                            continue;
                        }
                        let s = (p.rot * c.travel.normalize_or_zero()).dot(away) + rand() * 0.5;
                        if s > best.0 {
                            best = (s, k);
                        }
                    }
                    p.from = p.cur.clone();
                    p.blend = 0.15;
                    p.state = State::Evade(best.1);
                    p.time = 0.0;
                    self.evades += 1;
                    self.speech.push(Speech { pos: p.pos, voice: ty.voice.clone(), female: ty.female, fast: ty.evade[best.1].fast || speed > 18.0 });
                    break;
                }
            }
            let (clip, looped) = ty.clip(p.state);
            // advance, carrying the animation's own root motion into the world
            let t0 = p.time;
            p.time += dt;
            let last = clip.last_frame();
            let apply = |p: &mut Ped, f0: f32, f1: f32| {
                let (a, ar) = clip.mover(f0);
                let (b, br) = clip.mover(f1);
                p.pos += p.rot * (ar.inverse() * (b - a));
                p.rot = (p.rot * (ar.inverse() * br)).normalize();
            };
            if looped {
                let (f0, f1) = (clip.frame(t0, true), clip.frame(p.time, true));
                if f1 >= f0 {
                    apply(p, f0, f1);
                } else {
                    apply(p, f0, last);
                    apply(p, 0.0, f1);
                }
            } else {
                apply(p, clip.frame(t0, false), clip.frame(p.time, false));
                if p.time >= clip.anim.duration.max(0.05) {
                    p.from = p.cur.clone();
                    p.blend = 0.25;
                    p.time = 0.0;
                    p.calm = 1.5;
                    p.state = match p.state {
                        State::Evade(_) if !ty.angry.is_empty() && rand() < 0.45 => State::Angry((rand() * ty.angry.len() as f32) as usize % ty.angry.len()),
                        _ if p.loiter => State::Idle,
                        _ => State::Walk,
                    };
                }
            }
            // stay upright; walkers steer along the pavement
            let f = p.rot * -Vec3::Z;
            let mut yaw = (-f.x).atan2(-f.z);
            let mesh = self.mesh.as_ref().filter(|_| p.poly != NO_POLY);
            if let (Some(mesh), true) = (mesh, p.state == State::Walk && p.cross.is_none() && p.plan.is_none()) {
                // the end of the stretch: on into the polygon behind the edge, and choose the
                // next edge from there
                let d = flat(p.goal - p.entry);
                let len = d.length();
                let dir = if len > 0.05 { d / len } else { p.travel };
                if flat(p.pos - p.entry).dot(dir) >= len - 0.05 {
                    let at_end = p.next == p.poly;
                    p.prev = if at_end { NO_POLY } else { p.poly };
                    p.poly = p.next;
                    let (next, goal, dead) = mesh_next(mesh, p.poly, p.prev, p.pos, dir, p.side, rand());
                    p.next = next;
                    p.entry = p.pos;
                    p.goal = goal;
                    p.travel = dir;
                    if dead && at_end {
                        // a lone polygon: stay
                        p.loiter = true;
                        p.from = p.cur.clone();
                        p.blend = 0.25;
                        p.time = 0.0;
                        p.state = State::Idle;
                    } else if at_end || (flat(goal - p.pos).normalize_or_zero().dot(dir) < 0.77 && rand() < 0.35) {
                        // where the pavement ends or turns the corner: perhaps over the street
                        if let Some((q, n, straight)) = mesh_crossing(mesh, &self.island, p.poly, p.pos, dir) {
                            p.plan = Some(Plan::Mesh(q, n, straight));
                            p.wait = 0.0;
                        }
                    }
                }
            } else if mesh.is_none() && p.state == State::Walk && p.cross.is_none() && p.plan.is_none() {
                let l = &nav.links[p.link];
                let s = (p.pos - l.a).dot(l.dir());
                // end of the block: carry on along the next link if it has a pavement, cross
                // over to the other side of the street, or turn back
                if (p.fwd && s > l.len - 1.0) || (!p.fwd && s < 1.0) {
                    let cands: &Vec<usize> = if p.fwd { &l.next } else { &l.prev };
                    let mut go = None;
                    for &n in cands {
                        if nav.links[n].dir().dot(l.dir()) > 0.6 {
                            if let Some(Some(lat)) = self.curb.get(&n) {
                                go = Some((n, *lat));
                            }
                        }
                    }
                    let mut over: Option<(f32, Vec3, usize, f32)> = None;
                    if go.is_none() || rand() < 0.35 {
                        for n in nav.links_near(p.pos, 34.0) {
                            let l2 = &nav.links[n];
                            if l2.dir().dot(l.dir()) > -0.8 {
                                continue;
                            }
                            let Some(Some(lat2)) = self.curb.get(&n) else { continue };
                            let s2 = (p.pos - l2.a).dot(l2.dir()).clamp(1.0, (l2.len - 1.0).max(1.0));
                            let q = l2.a + l2.dir() * s2 + l2.dir().cross(Vec3::Y).normalize_or_zero() * *lat2;
                            let d = q.distance(p.pos);
                            if d < 7.0 || d > 34.0 || (q - p.pos).dot(l.dir()).abs() > d * 0.45 || (q.y - p.pos.y).abs() > 2.0 {
                                continue;
                            }
                            if over.map_or(true, |b| d < b.0) {
                                over = Some((d, q, n, *lat2));
                            }
                        }
                    }
                    match (over, go) {
                        (Some((_, q, n, lat)), _) => p.plan = Some(Plan::Cross(q, n, lat)),
                        (None, Some((n, lat))) => p.plan = Some(Plan::Straight(n, lat + ((p.seed >> 16) & 0xff) as f32 / 255.0 * 0.8 - 0.4)),
                        (None, None) => p.fwd = !p.fwd,
                    }
                    p.wait = 0.0;
                }
            }
            // at the kerb: go when the lights (or, without lights, the traffic) allow it
            if let (Some(plan), true) = (p.plan, matches!(p.state, State::Walk | State::Idle)) {
                let travel = if mesh.is_some() {
                    p.travel
                } else {
                    let l = &nav.links[p.link];
                    if p.fwd { l.dir() } else { -l.dir() }
                };
                // signal for the traffic of the own street at the junction ahead
                let sig = signals.and_then(|(sg, t)| sg.ahead(p.pos, travel, t)).filter(|a| a.1 < 30.0).map(|a| a.2);
                let (line, reach) = match plan {
                    Plan::Straight(..) => (travel, 26.0),
                    Plan::Cross(q, ..) | Plan::Mesh(q, ..) => ((q - p.pos).normalize_or_zero(), q.distance(p.pos) + 4.0),
                };
                // a car driving at the stretch about to be walked?
                let mut busy = false;
                for t in threats {
                    let speed = t.vel.length();
                    if speed < 2.5 {
                        continue;
                    }
                    let rel = t.pos - p.pos;
                    let along = rel.dot(line);
                    if rel.y.abs() > 4.0 || along < -3.0 || along > reach {
                        continue;
                    }
                    let side = rel - line * along;
                    // closing in on the crossing line within the next few seconds
                    let closing = -side.normalize_or_zero().dot(t.vel);
                    if side.length() < 3.0 || (closing > 1.0 && side.length() / closing < if sig.is_some() { 1.5 } else { 4.5 }) {
                        busy = true;
                        break;
                    }
                }
                let lights = match (plan, sig) {
                    (_, None) => true,
                    (Plan::Straight(..) | Plan::Mesh(_, _, true), Some(st)) => st == 2,
                    (Plan::Cross(..) | Plan::Mesh(_, _, false), Some(st)) => st == 0,
                };
                if lights && !busy {
                    match plan {
                        Plan::Straight(n, lat) => {
                            p.link = n;
                            p.lat = lat;
                        }
                        Plan::Cross(q, n, lat) => p.cross = Some((q, n, lat)),
                        Plan::Mesh(q, n, _) => p.cross = Some((q, n as usize, 0.0)),
                    }
                    p.plan = None;
                    if p.state == State::Idle {
                        p.from = p.cur.clone();
                        p.blend = 0.25;
                        p.time = 0.0;
                        p.state = State::Walk;
                    }
                } else {
                    p.wait += dt;
                    if p.state == State::Walk {
                        p.from = p.cur.clone();
                        p.blend = 0.25;
                        p.time = 0.0;
                        p.state = State::Idle;
                    }
                    // tired of waiting where there are no lights: walk back
                    // (on the mesh also at lights that never come round for this line)
                    if p.wait > 25.0 && (sig.is_none() || (mesh.is_some() && p.wait > 45.0)) {
                        p.plan = None;
                        p.fwd = !p.fwd;
                        p.from = p.cur.clone();
                        p.blend = 0.25;
                        p.time = 0.0;
                        p.state = State::Walk;
                    }
                }
            }
            if p.state == State::Walk {
                if let Some((q, n, lat)) = p.cross {
                    if Vec3::new(q.x - p.pos.x, 0.0, q.z - p.pos.z).length() < 0.8 {
                        // over: walk away from the corner on the far side
                        if let Some(mesh) = mesh {
                            let dir = flat(q - p.entry).normalize_or(p.travel);
                            p.poly = n as u32;
                            p.prev = NO_POLY;
                            let (next, goal, _) = mesh_next(mesh, p.poly, NO_POLY, p.pos, dir, p.side, rand());
                            p.next = next;
                            p.entry = p.pos;
                            p.goal = goal;
                            p.travel = dir;
                        } else {
                            p.link = n;
                            p.lat = lat;
                        }
                        p.cross = None;
                        self.crossings += 1;
                    }
                }
                let (mut target, right);
                if let Some(mesh) = mesh {
                    // along the line to the point on the edge, looking 2.5 m ahead (and up
                    // to a metre past the edge, into the next polygon)
                    let d = flat(p.goal - p.entry);
                    let len = d.length();
                    let s = flat(p.pos - p.entry).dot(d) / len.max(0.05);
                    target = p.entry + (p.goal - p.entry) * ((s + 2.5).min(len + 1.0) / len.max(0.05));
                    // "towards the buildings" below: towards the middle of the pavement
                    let c = mesh.polys[p.poly as usize].center - p.pos;
                    right = Vec3::new(-f.z, 0.0, f.x) * if c.dot(Vec3::new(-f.z, 0.0, f.x)) < 0.0 { -1.0 } else { 1.0 };
                } else {
                    let l = &nav.links[p.link];
                    let dir = l.dir();
                    right = dir.cross(Vec3::Y).normalize_or_zero();
                    let s = (p.pos - l.a).dot(dir);
                    target = l.a + dir * (s + if p.fwd { 2.5 } else { -2.5 }) + right * p.lat;
                }
                if let Some((q, ..)) = p.cross {
                    target = q;
                }
                // walk around benches, lamp posts, news stands
                near.clear();
                obstacles(p.pos, 2.6, &mut near);
                let side = Vec3::new(-f.z, 0.0, f.x);
                for (o, r) in &near {
                    let rel = Vec3::new(o.x - p.pos.x, 0.0, o.z - p.pos.z);
                    let ahead = rel.dot(f);
                    let lateral = rel.dot(side);
                    let clear = r + 0.45;
                    if ahead > -0.3 && ahead < 2.6 + r && lateral.abs() < clear {
                        // pass on the side we are already on (towards the buildings when dead ahead)
                        let pass = if lateral.abs() < 0.08 { -right.dot(side).signum() } else { lateral.signum() };
                        target -= side * pass * (clear - lateral.abs()) * 2.5;
                    }
                    let d = rel.length();
                    if d < clear * 0.8 && d > 1e-3 {
                        p.pos -= rel / d * (clear * 0.8 - d).min(dt * 2.0);
                    }
                }
                let to = target - p.pos;
                let d = wrap_angle((-to.x).atan2(-to.z) - yaw);
                yaw += d.clamp(-1.8 * dt, 1.8 * dt);
            }
            p.rot = Quat::from_rotation_y(yaw);
            // on the mesh: the polygon's height, refined by the collision where the two agree
            let floor = mesh.filter(|_| p.cross.is_none()).and_then(|m| {
                [p.poly, p.next].into_iter().map(|i| &m.polys[i as usize]).find(|q| q.contains(p.pos.x, p.pos.z)).map(|q| q.height(p.pos.x, p.pos.z))
            });
            let y = match (col.ground(p.pos, 0.7, 1.5), floor) {
                (Some(g), Some(m)) => Some(if (g.point.y - m).abs() < 0.5 { g.point.y } else { m }),
                (Some(g), None) => Some(g.point.y),
                (None, m) => m,
            };
            if let Some(y) = y {
                p.pos.y += (y - p.pos.y) * (dt * 12.0).min(1.0);
            }
        }
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223 + self.list.len() as u32);
    }

    /// Skin everything in view into this frame's vertex buffer.
    pub fn prepare(&mut self, gpu: &Gpu, cam: Vec3, frustum: &Frustum) {
        self.buf.clear();
        let mut local: Vec<(Quat, Vec3)> = Vec::new();
        for p in self.list.iter_mut() {
            let Some(Some(ty)) = self.types.get(&p.ty) else { continue };
            let dist = p.pos.distance(cam);
            let visible = dist < 160.0 && frustum.intersects_aabb(p.pos - Vec3::new(1.2, 0.2, 1.2), p.pos + Vec3::new(1.2, 2.1, 1.2));
            let (clip, looped) = ty.clip(p.state);
            clip.local(&ty.model.bones, clip.frame(p.time, looped), &mut local);
            if p.blend > 0.0 && p.from.len() == local.len() {
                blend_pose(&mut local, &p.from, (p.blend / 0.25).clamp(0.0, 1.0));
            }
            p.cur.clone_from(&local);
            if !visible {
                continue;
            }
            let lod = if dist < ty.lod_switch[0] {
                0
            } else if dist < ty.lod_switch[1] {
                1
            } else {
                2
            };
            self.buf.emit(&ty.model, lod, &local, Mat4::from_rotation_translation(p.rot, p.pos));
        }
        self.buf.upload(gpu);
    }

    /// Draw with the pipeline already set (opaque pass or shadow pass).
    pub fn draw<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, gpu: &'a Gpu, shadow: bool, draws: &mut usize) {
        self.buf.draw(pass, gpu, shadow, draws);
    }

    /// Speech bank of a voice key ("fh_006"), from the converted `ped_*` banks.
    pub fn voice_bank(&mut self, audio_root: &Path, voice: &str, female: bool) -> Option<String> {
        if self.voices.is_none() {
            let mut m = HashMap::new();
            if let Ok(rd) = std::fs::read_dir(audio_root.join("0x051b41b9")) {
                for e in rd.flatten() {
                    let Ok(t) = std::fs::read_to_string(e.path().join("_index.tsv")) else { continue };
                    // "ped_slow_evade_fh_006_06" -> "fh_006"
                    for line in t.lines().take(40) {
                        let name = line.split('\t').next().unwrap_or("");
                        let Some(i) = name.find("_evade_") else { continue };
                        let rest = &name[i + 7..];
                        if let Some(j) = rest.rfind('_') {
                            m.insert(rest[..j].to_string(), format!("0x051b41b9/{}", e.file_name().to_string_lossy()));
                            break;
                        }
                    }
                }
            }
            log::info!("peds: {} speech banks", m.len());
            self.voices = Some(m);
        }
        let m = self.voices.as_ref()?;
        if let Some(b) = m.get(voice).or_else(|| m.get(voice.trim_start_matches("sc_"))) {
            return Some(b.clone());
        }
        // no bank of its own: borrow one of the same sex
        let g = if female { 'f' } else { 'm' };
        let mut same: Vec<&String> = m.iter().filter(|(k, _)| k.trim_start_matches("sc_").starts_with(g)).map(|(_, v)| v).collect();
        same.sort();
        if same.is_empty() {
            return None;
        }
        Some(same[hash(mcla_formats::hash::joaat(voice), 3) as usize % same.len()].clone())
    }
}
