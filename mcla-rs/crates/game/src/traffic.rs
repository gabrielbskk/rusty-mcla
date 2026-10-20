//! Ambient traffic as the original drives it (`mcAmbientCar`, vtable 0x820c2b0c, with its
//! `mcAmbientBehaviorDrive2` 0x820d9dfc; per frame sub_826EA968: the behaviour sub_827A0138,
//! the acceleration sub_826EA420, the speed sub_826EA240). A car rides its lane kinematically
//! (the rail follower advances by speed x dt, sub_82798F18) with no pedals or steering; it
//! only turns physical when something meets it (base slot 18 sub_826E96E0), and then the
//! port's physical driver takes it on. Rules decoded 2026-10-19: docs/PEOPLE_TRAFFIC.md
//! "Ambient traffic driving".

use super::Vehicles;
use crate::physics::collision::WorldCollider;
use glam::{Quat, Vec3};
use mcla_formats::rage::navsys::{NavSys, DEFAULT_ROAD_SPEED};

const MPH: f32 = 0.44704;
/// The slowest a car is ever asked to drive (sub_827A2B38: 10 mph).
const MIN_WANT: f32 = 4.4704;
/// Braking below this stops the car (sub_826EA240: 0.5 mph).
const STOP_SPEED: f32 = 0.22352;
/// Occluders are taken within 400 m of the focus, as the pedestrian spawner's (peds.rs
/// `OCC_RANGE_SCALE`).
const OCC_RANGE: f32 = crate::occluder::RANGE * 10.0;

/// A traffic type's driving values (`mcAmbientType.xml`; type +124 .. +148).
#[derive(Clone, Copy, Debug)]
pub struct AmbType {
    pub max_speed: f32,
    pub min_accel_band: f32,
    pub min_accel: f32,
    pub max_accel: f32,
    pub min_decel_band: f32,
    pub min_decel: f32,
    pub max_decel: f32,
    /// the type's pool (`iInstanceCount`, shared with the parked cars), its bound group
    /// (`iBoundGroup`, 2: buses, semis, limo, garbage truck) and `bFreewayOnly`
    pub instances: u32,
    pub bound_group: u32,
    pub freeway_only: bool,
}

impl Default for AmbType {
    fn default() -> Self {
        // (the values every sedan of the set carries)
        AmbType { max_speed: 40.0, min_accel_band: 1.0, min_accel: 1.0, max_accel: 5.0, min_decel_band: 0.2, min_decel: 3.0, max_decel: 22.5, instances: 20, bound_group: 0, freeway_only: false }
    }
}

impl AmbType {
    pub fn load(root: &std::path::Path, model: &str) -> AmbType {
        let d = AmbType::default();
        let Ok(t) = std::fs::read_to_string(root.join("tune/ambients/traffic").join(model).join("mcAmbientType.xml")) else { return d };
        let v = |tag: &str, def: f32| crate::ambient::xml_value(&t, tag).unwrap_or(def);
        AmbType {
            max_speed: v("fMaxSpeed", d.max_speed),
            min_accel_band: v("fMinAccelBand", d.min_accel_band),
            min_accel: v("fMinAccel", d.min_accel),
            max_accel: v("fMaxAccel", d.max_accel),
            min_decel_band: v("fMinDecelBand", d.min_decel_band),
            min_decel: v("fMinDecel", d.min_decel),
            max_decel: v("fMaxDecel", d.max_decel),
            instances: v("iInstanceCount", 20.0) as u32,
            bound_group: v("iBoundGroup", 0.0) as u32,
            freeway_only: t.contains("<bFreewayOnly value=\"true\""),
        }
    }
}

/// A traffic car on its lane.
#[derive(Clone, Debug)]
pub struct Rider {
    pub ty: AmbType,
    /// the link it is on and the metres along it
    pub link: usize,
    pub s: f32,
    /// the first link of the lane it takes after this one (chosen when the lane is entered)
    pub next: Option<usize>,
    /// speed (car +412)
    pub v: f32,
    /// the speed offset (car +424: -6, 0 or +6 m/s) and seconds to its next draw
    pub off: f32,
    pub off_timer: f32,
    /// acceleration bonus (car +428: U - 0.5, drawn once)
    pub bonus: f32,
    /// the standing gap behind the car ahead (behaviour +460: 2 .. 5 m)
    pub gap: f32,
    /// start delay when moving off (car +444: U x 0.8 s; < 0 not drawn)
    pub wait: f32,
    /// the constraint entries switched on (hysteresis sub_826E91F8): car ahead, stop line, slower lane
    pub on: [bool; 3],
    /// met by something: the physics has it from now on
    pub physical: bool,
    rng: u32,
    id: u32,
    /// (diagnostics) the car ahead last seen: centre distance and margin
    pub ahead: (f32, f32),
    /// (diagnostics) the acceleration asked for (car +408)
    pub a: f32,
    /// when it was made and how long it lives at least on a listed road (car +464 / +468:
    /// 8 + 4U s)
    pub born: f32,
    pub life: f32,
}

impl Rider {
    pub fn new(nav: &NavSys, ty: AmbType, link: usize, s: f32, seed: u32) -> Rider {
        let mut r = Rider { ty, link, s, next: None, v: 0.0, off: 0.0, off_timer: 0.0, bonus: 0.0, gap: 3.5, wait: -1.0, on: [false; 3], physical: false, rng: seed.max(1), id: seed, ahead: (0.0, 0.0), a: 0.0, born: 0.0, life: 8.0 };
        r.life = 8.0 + 4.0 * r.rand();
        r.bonus = r.rand() - 0.5;
        r.gap = 2.0 + 3.0 * r.rand();
        r.draw_offset();
        r.choose_next(nav);
        // (spawned moving: the rule that sets the first speed is not decoded - the lane's own)
        r.v = rail_speed(nav, link).0;
        r
    }

    /// a stable id for traces
    pub fn rng_id(&self) -> u32 {
        self.id
    }

    fn rand(&mut self) -> f32 {
        // (the car's own MWC is not reproduced: any uniform draw)
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / 16777216.0
    }

    /// sub_826EA240: every 3 + 3U s a new offset, -6, +6 or 0 m/s by thirds.
    fn draw_offset(&mut self) {
        let u = self.rand();
        self.off = if u < 1.0 / 3.0 { -6.0 } else if u < 2.0 / 3.0 { 6.0 } else { 0.0 };
        self.off_timer = 3.0 + 3.0 * self.rand();
    }

    /// The last link of the lane the car is on.
    fn lane_end(&self, nav: &NavSys) -> usize {
        let mut k = self.link;
        let lane = nav.links[k].lane;
        while let Some(&n) = nav.links[k].next.first() {
            if nav.links[n].lane != lane || n == k {
                break;
            }
            k = n;
        }
        k
    }

    /// Which lane follows this one (the original's choice of turn is not decoded: any of the
    /// successors, at random).
    fn choose_next(&mut self, nav: &NavSys) {
        let end = self.lane_end(nav);
        let lane = nav.links[end].lane;
        let mut succ: Vec<usize> = nav.links[end].next.iter().copied().filter(|&n| nav.links[n].lane != lane).collect();
        if succ.is_empty() {
            // (a lane end the port's graph left unconnected - the original's rails all join:
            // the lane starts within reach of it)
            succ = nav.links[end].hop.clone();
        }
        self.next = if succ.is_empty() { None } else { Some(succ[(self.rand() * succ.len() as f32) as usize % succ.len()]) };
    }

    /// Metres left to the end of the lane.
    fn lane_left(&self, nav: &NavSys) -> f32 {
        let mut k = self.link;
        let lane = nav.links[k].lane;
        let mut d = nav.links[k].len - self.s;
        while let Some(&n) = nav.links[k].next.first() {
            if nav.links[n].lane != lane || n == k {
                break;
            }
            k = n;
            d += nav.links[k].len;
        }
        d.max(0.0)
    }

    /// The link after `k` on the car's way (within the lane, then the chosen next lane).
    fn after(&self, nav: &NavSys, k: usize) -> Option<usize> {
        let l = &nav.links[k];
        match l.next.first() {
            Some(&n) if nav.links[n].lane == l.lane && n != k => Some(n),
            _ if l.lane == nav.links[self.link].lane => self.next,
            _ => l.next.first().copied(),
        }
    }

    /// The point `d` metres ahead along the car's way.
    fn point_ahead(&self, nav: &NavSys, d: f32) -> Vec3 {
        let (mut k, mut s) = (self.link, self.s + d);
        for _ in 0..8 {
            let l = &nav.links[k];
            if s <= l.len {
                return l.a + (l.b - l.a) * (s / l.len.max(1e-3));
            }
            let Some(n) = self.after(nav, k) else { return l.b };
            s -= l.len;
            k = n;
        }
        nav.links[k].b
    }

    pub fn pos(&self, nav: &NavSys) -> Vec3 {
        let l = &nav.links[self.link];
        l.a + (l.b - l.a) * (self.s / l.len.max(1e-3))
    }
}

/// The speed of a lane (sub_82245A28 on the lane's owner, flags at owner +4) and whether it is
/// a junction lane (owner flags & 0x30, or lane flags & 0x1000).
pub fn rail_speed(nav: &NavSys, link: usize) -> (f32, bool) {
    let l = &nav.links[link];
    let f = l.owner_flags;
    let v = if f & 0x20 != 0 && l.owner_w10 == 0 {
        15.0 * MPH
    } else if f & 0x10 != 0 {
        15.0 * MPH
    } else if f & 0x20 != 0 {
        10.0 * MPH
    } else if f & 0x40 != 0 {
        6.0 * MPH
    } else {
        nav.speed_limit_mps(link).unwrap_or(DEFAULT_ROAD_SPEED)
    };
    (v, f & 0x30 != 0 || l.flags & 0x1000 != 0)
}

/// The acceleration that brings speed `v` to `vt` over `dist - margin` metres (sub_82796658).
fn needed(v: f32, vt: f32, dist: f32, margin: f32, ty: &AmbType) -> f32 {
    let den = dist - margin;
    if den == 0.0 {
        return if dist > 0.0 { ty.max_accel } else { -ty.max_decel };
    }
    // (inside the margin: brake if faster than the target, go if slower)
    if den < 0.0 && v > vt {
        return -ty.max_decel;
    }
    if den < 0.0 && vt > v {
        return ty.max_accel;
    }
    (vt - v) * (v + vt) * 0.5 / den
}

/// What another car is to a rider looking for the car ahead.
pub struct Other {
    pub pos: Vec3,
    pub fwd: Vec3,
    pub speed: f32,
    pub half_len: f32,
    /// on a lane, kinematic: (link, metres along it)
    pub on: Option<(usize, f32)>,
    pub player: bool,
}

/// One frame of a rider (sub_827A0138 / sub_826EA420 / sub_826EA240). `others` are all cars
/// (the rider's own index `me` is skipped); `stop` the stop line ahead: metres and whether the
/// signal says stop. Returns the new pose.
pub fn step(r: &mut Rider, nav: &NavSys, me: usize, half_len: f32, others: &[Other], stop: Option<(f32, bool)>, hold: bool, dt: f32) -> (Vec3, Vec3) {
    let ty = r.ty;
    let v = r.v;
    r.off_timer -= dt;
    if r.off_timer <= 0.0 {
        r.draw_offset();
    }
    // wanted speed (sub_827A2B38): the lane's speed and the next lane's, with the offset
    // except on junction lanes
    let (cur, junction) = rail_speed(nav, r.link);
    let want = if junction { cur.max(MIN_WANT) } else { (cur + r.off).max(MIN_WANT) };
    let next_want = match (junction, r.next) {
        (true, _) | (_, None) => want,
        (false, Some(n)) => {
            let (nv, nj) = rail_speed(nav, n);
            if nj { nv.max(MIN_WANT) } else { (nv + r.off).max(MIN_WANT) }
        }
    };
    let pos = r.pos(nav);
    let fwd = (r.point_ahead(nav, 1.5) - pos).normalize_or(nav.links[r.link].dir());
    // the car ahead (sub_827A6448): on this lane or the next, else something else in the lane
    let lane = nav.links[r.link].lane;
    let next_lane = r.next.map(|n| nav.links[n].lane);
    let mut ahead: Option<(f32, f32, f32, bool)> = None; // (centre distance, its speed along us, its half length, player)
    let mut player_across = false;
    for (j, o) in others.iter().enumerate() {
        if j == me {
            continue;
        }
        let rel = o.pos - pos;
        let along = rel.dot(fwd);
        if along <= 0.0 || along > 80.0 || rel.y.abs() > 4.0 {
            continue;
        }
        let mine = match o.on {
            Some((k, _)) => nav.links[k].lane == lane || Some(nav.links[k].lane) == next_lane,
            None => {
                let side = (rel - fwd * along).length();
                // (the original's lane-width test is not decoded: a lane's half width)
                if o.player && side < 3.0 && o.fwd.dot(fwd).abs() < 0.7071 {
                    player_across = true;
                }
                side < 1.8
            }
        };
        if !mine {
            continue;
        }
        let d = rel.length();
        if ahead.map_or(true, |a| d < a.0) {
            // (its speed along our heading, but not for a car turning through a junction)
            let vt = match o.on {
                Some((k, _)) if rail_speed(nav, k).1 => o.speed,
                _ => (o.fwd * o.speed).dot(fwd),
            };
            ahead = Some((d, vt.max(0.0), o.half_len, o.player));
        }
    }
    let max_decel = if player_across { ty.max_decel * 3.0 } else { ty.max_decel };
    let tyd = AmbType { max_decel, ..ty };
    // the constraint entries: (acceleration, gap, player)
    let mph = v / MPH;
    let mut entries: [Option<(f32, f32, bool)>; 3] = [None; 3];
    if let Some((d, vt, oh, player)) = ahead {
        let margin = half_len + oh + r.gap * (1.0 + 2.0 * (mph / 70.0).min(1.0));
        entries[0] = Some((needed(v, vt, d, margin, &tyd), d - margin, player));
        r.ahead = (d, margin);
    }
    // (an entry that is on stays in the reckoning when its reason goes - the light turns
    // green, the car is down to the slow lane's speed - and pulls away at the full
    // acceleration until the hysteresis lets it go: the recorded moving-off accelerations,
    // 5.7 m/s^2 with entry 1 or 2 on against 3.2 with entry 0; inferred, not read in the code)
    if let Some((d, red)) = stop {
        if d - half_len > -1.0 && (red || r.on[1]) {
            entries[1] = Some((needed(v, if red { 0.0 } else { want }, d, half_len, &tyd), d - half_len, false));
        }
    }
    if next_want <= v || r.on[2] {
        let d = r.lane_left(nav);
        entries[2] = Some((needed(v, next_want, d, 0.0, &tyd), d, false));
    }
    let (on_a, off_a) = (ty.min_accel + ty.min_accel_band * 0.5, ty.min_accel - ty.min_accel_band * 0.5);
    let (on_d, off_d) = (ty.min_decel + ty.min_decel_band * 0.5, ty.min_decel - ty.min_decel_band * 0.5);
    let mut best_neg: Option<f32> = None;
    let mut best_pos: Option<(f32, f32)> = None;
    for k in 0..3 {
        let Some((mut a, gap, player)) = entries[k] else {
            r.on[k] = false;
            continue;
        };
        if !r.on[k] {
            r.on[k] = a > on_a || -a > on_d || gap <= 0.0;
        } else if gap > 0.0 && ((a >= 0.0 && a < off_a) || (a < 0.0 && -a < off_d)) {
            r.on[k] = false;
        }
        if !r.on[k] {
            continue;
        }
        if player && -a > 1.25 * max_decel {
            a = -1.5 * max_decel;
        } else {
            a = a.clamp(-max_decel, ty.max_accel + r.bonus);
        }
        if a < 0.0 {
            best_neg = Some(best_neg.map_or(a, |b: f32| b.min(a)));
        } else if best_pos.map_or(true, |b| gap < b.1) {
            best_pos = Some((a, gap));
        }
    }
    let mut a = match (best_neg, best_pos) {
        (Some(n), _) => n,
        (None, Some((p, _))) => p,
        _ => ((want - v) / 1.0).clamp(-(ty.min_decel + ty.max_decel) * 0.5, (ty.min_accel + ty.max_accel + r.bonus) * 0.5),
    };
    // moving off: a short start delay
    if v == 0.0 && a > 0.0 {
        if r.wait < 0.0 {
            r.wait = r.rand() * 0.8;
        }
        r.wait -= dt;
        if r.wait > 0.0 {
            a = 0.0;
        }
    } else if v > 0.0 {
        r.wait = -1.0;
    }
    if hold {
        a = -max_decel;
    }
    // the speed (sub_826EA240)
    r.a = a;
    a = a.min(ty.max_accel);
    let mut nv = (v + a * dt).min(ty.max_speed);
    if a < 0.0 && nv < STOP_SPEED {
        nv = 0.0;
    }
    r.v = nv.max(0.0);
    // along the lane
    r.s += r.v * dt;
    for _ in 0..8 {
        let len = nav.links[r.link].len;
        if r.s <= len {
            break;
        }
        let Some(n) = r.after(nav, r.link) else {
            if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() && r.v > 0.0 {
                let l = &nav.links[r.link];
                log::info!("traffic: dead end at {:.1} {:.1} lane {} owner {:#x} next {:?} hop {:?}", l.b.x, l.b.z, l.lane, l.owner_flags, l.next, l.hop);
            }
            r.s = len;
            r.v = 0.0;
            break;
        };
        let new_lane = nav.links[n].lane != nav.links[r.link].lane;
        r.s -= len;
        r.link = n;
        if new_lane {
            r.choose_next(nav);
        }
    }
    let p = r.pos(nav);
    let f = (r.point_ahead(nav, 1.5) - p).normalize_or(fwd);
    (p, f)
}

/// The rotation of a car facing `f` (3D) with the world's up as near up as it goes.
pub fn facing(f: Vec3) -> Quat {
    let right = f.cross(Vec3::Y).normalize_or(Vec3::X);
    let up = right.cross(f).normalize_or(Vec3::Y);
    Quat::from_mat3(&glam::Mat3::from_cols(right, up, -f)).normalize()
}

impl Vehicles {
    /// Move the riding traffic cars along their lanes.
    pub(super) fn ride_traffic(&mut self, nav: &NavSys, _col: &WorldCollider, dt: f32) {
        if !self.cars.iter().any(|c| c.rail.as_ref().is_some_and(|r| !r.physical)) {
            return;
        }
        let others: Vec<Other> = self
            .cars
            .iter()
            .map(|c| {
                let on = c.rail.as_ref().filter(|r| !r.physical).map(|r| (r.link, r.s));
                let speed = c.rail.as_ref().filter(|r| !r.physical).map_or(c.state.speed(), |r| r.v);
                Other { pos: c.state.pos, fwd: c.state.forward(), speed, half_len: half_length(c), on, player: c.is_player }
            })
            .collect();
        let hold = self.frozen || self.grid_hold.is_some();
        // MCLA_TRAFFIC_TRACE=file: every riding car ten times a second (time, id, model, x, y,
        // z, speed, lane speed, junction lane, offset, entries on, standing gap)
        static TRACE: std::sync::Mutex<Option<(std::io::BufWriter<std::fs::File>, f32)>> = std::sync::Mutex::new(None);
        let mut guard = TRACE.lock().unwrap();
        if guard.is_none() {
            if let Ok(p) = std::env::var("MCLA_TRAFFIC_TRACE") {
                if let Ok(f) = std::fs::File::create(p) {
                    *guard = Some((std::io::BufWriter::new(f), -1.0));
                }
            }
        }
        let mut trace = match guard.as_mut() {
            Some((w, last)) if self.clock - *last >= 0.1 => {
                *last = self.clock;
                Some(w)
            }
            _ => None,
        };
        for i in 0..self.cars.len() {
            let half = half_length(&self.cars[i]);
            let c = &mut self.cars[i];
            let Some(r) = c.rail.as_mut().filter(|r| !r.physical) else { continue };
            // the stop line ahead: stop unless the light is green (sub_8273A178)
            let pos = r.pos(nav);
            let fwd = nav.links[r.link].dir();
            let stop = self.signals.ahead(pos, fwd, self.clock).map(|x| (x.1, x.2 != 2));
            let (p, f) = step(r, nav, i, half, &others, stop, hold, dt);
            let v = r.v;
            if let Some(w) = trace.as_mut() {
                use std::io::Write;
                let (lim, junction) = rail_speed(nav, r.link);
                let _ = writeln!(w, "{:.2}	{}	{}	{:.2}	{:.2}	{:.2}	{:.3}	{:.2}	{}	{:.0}	{}{}{}	{:.2}	{:.2}	{:.2}	{:.3}", self.clock, r.rng_id(), c.model.name, p.x, p.y, p.z, v, lim, junction as u8, r.off, r.on[0] as u8, r.on[1] as u8, r.on[2] as u8, r.gap, r.ahead.0, r.ahead.1, r.a);
            }
            let rot = facing(f);
            c.state.relocate(p, rot);
            c.state.vel = f * v;
            c.state.ang_vel = Vec3::ZERO;
            let wr = c.model.params.wheel_radius.max(0.2);
            for w in c.state.wheels.iter_mut() {
                w.spin += v / wr * dt;
                w.steer = 0.0;
                w.grounded = true;
            }
            c.placed = true;
        }
    }
}

impl Vehicles {
    /// Something met a riding car: from now on the physics has it (base slot 18 sub_826E96E0
    /// drops the rail; the original's recovery onto the lane, states 5 / 6, is not ported - the
    /// port's physical driver takes it on from its lane).
    pub(super) fn let_go(&mut self, k: usize, world: &crate::world::World) {
        let c = &mut self.cars[k];
        let Some(r) = c.rail.as_mut() else { return };
        if r.physical {
            return;
        }
        r.physical = true;
        c.state.vel = c.state.forward() * r.v;
        if let (Some(ai), Some(nav)) = (c.ai.as_mut(), world.nav.as_ref()) {
            ai.link = Some(r.link);
            ai.t = r.s / nav.links[r.link].len.max(1e-3);
            ai.route.clear();
            ai.target_speed = rail_speed(nav, r.link).0;
        }
    }
}

/// Half the car's length along its axis: the original takes its bound's box (obstacle vfunc
/// 24); the port the model's extent.
fn half_length(c: &super::CarInstance) -> f32 {
    let l = (c.model.max.z - c.model.min.z) * 0.5;
    if l.is_finite() && l > 0.5 { l } else { c.model.params.body_half.z.max(1.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needed_acceleration_like_the_original() {
        let ty = AmbType::default();
        // 10 m/s to a stop over 20 m: -2.5 m/s^2
        assert!((needed(10.0, 0.0, 25.0, 5.0, &ty) + 2.5).abs() < 1e-5);
        // no room left: full braking, or full throttle when the gap is still ahead
        assert_eq!(needed(10.0, 0.0, 5.0, 5.0, &ty), ty.max_accel);
        assert_eq!(needed(10.0, 0.0, 3.0, 5.0, &ty), -ty.max_decel);
    }

    #[test]
    fn offsets_are_minus_six_zero_plus_six() {
        let mut r = Rider { ty: AmbType::default(), link: 0, s: 0.0, next: None, v: 0.0, off: 0.0, off_timer: 0.0, bonus: 0.0, gap: 3.0, wait: -1.0, on: [false; 3], physical: false, rng: 7, id: 7, ahead: (0.0, 0.0), a: 0.0, born: 0.0, life: 8.0 };
        let mut seen = [0usize; 3];
        for _ in 0..3000 {
            r.draw_offset();
            assert!(r.off_timer >= 3.0 && r.off_timer <= 6.0);
            seen[match r.off as i32 { -6 => 0, 6 => 1, 0 => 2, _ => unreachable!() }] += 1;
        }
        assert!(seen.iter().all(|&n| n > 850 && n < 1150), "{seen:?}");
    }
}

// ---------------------------------------------------------------------------------------
// The spawner and unspawner (density manager `mcAmbientDensityTuning`, vtable 0x820C39C4;
// sub_826F79F0: the region list on odd manager frames, sub_826F4E90; spawning sub_826F6920
// and unspawning sub_826F4390 on even ones - 15 Hz each). Decoded 2026-10-19:
// docs/PEOPLE_TRAFFIC.md "Ambient traffic spawner".

/// A road as the spawner weighs it (`mcRnRoadAmbient`).
pub struct Road {
    pub mid: Vec3,
    pub dir: Vec3,
    pub half: glam::Vec2,
    pub speed: f32,
    pub density: i32,
    /// its lanes: first link and length
    pub lanes: Vec<(usize, f32)>,
    pub lane_len: f32,
}

/// `density_tuning.xml`.
#[derive(Clone, Copy)]
pub struct Tune {
    pub box_len: [f32; 2],
    pub box_off: [f32; 2],
    pub box_width: [f32; 2],
    pub perp_len: [f32; 2],
    pub perp_off: [f32; 2],
    pub perp_width: [f32; 2],
    pub spacing: f32,
    pub fov_mph: [f32; 2],
    pub fov_deg: [f32; 2],
    pub density_mph: [f32; 2],
    pub density_at: [f32; 2],
    pub values: [f32; 7],
    pub spawn_inner: f32,
    pub spawn_outer: f32,
    pub unspawn_outer: f32,
}

impl Tune {
    fn load(root: &std::path::Path) -> Tune {
        let t = std::fs::read_to_string(root.join("tune/ambients/density_tuning.xml")).unwrap_or_default();
        let g = |tag: &str, d: f32| crate::ambient::xml_value(&t, tag).unwrap_or(d);
        let mut values = [0.0, 0.02, 0.35, 0.55, 0.70, 1.25, 0.10];
        if let Some(i) = t.find("<DensityValues") {
            let body = &t[i..];
            if let (Some(a), Some(b)) = (body.find('>'), body.find("</DensityValues>")) {
                let v: Vec<f32> = body[a + 1..b].split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if v.len() == 7 {
                    values.copy_from_slice(&v);
                }
            }
        }
        Tune {
            box_len: [g("fTrafficBoxLengthAtLowSpeed", 700.0), g("fTrafficBoxLengthAtHighSpeed", 500.0)],
            box_off: [g("fTrafficBoxOffsetAtLowSpeed", 100.0), g("fTrafficBoxOffsetAtHighSpeed", 100.0)],
            box_width: [g("fTrafficBoxWidthAtLowSpeed", 100.0), g("fTrafficBoxWidthAtHighSpeed", 100.0)],
            perp_len: [g("fTrafficBoxLengthAtLowSpeedPerp", 400.0), g("fTrafficBoxLengthAtHighSpeedPerp", 400.0)],
            perp_off: [g("fTrafficBoxOffsetAtLowSpeedPerp", 0.0), g("fTrafficBoxOffsetAtHighSpeedPerp", 0.0)],
            perp_width: [g("fTrafficBoxWidthAtLowSpeedPerp", 400.0), g("fTrafficBoxWidthAtHighSpeedPerp", 400.0)],
            spacing: g("fMinSpawnTrafficSpacing", 15.0),
            fov_mph: [g("fFOVDegreesLowMPH", 25.0), g("fFOVDegreesHighMPH", 45.0)],
            fov_deg: [g("fFOVDegreesAtLowSpeed", 89.9), g("fFOVDegreesAtHighSpeed", 45.0)],
            density_mph: [g("fDensityLowMPH", 80.0), g("fDensityHighMPH", 200.0)],
            density_at: [g("fDensityAtLowSpeed", 1.0), g("fDensityAtHighSpeed", 0.4)],
            values,
            spawn_inner: g("fTrafficSpawnInnerRadius", 50.0),
            spawn_outer: g("fTrafficSpawnOuterRadius", 250.0),
            unspawn_outer: g("fTrafficUnspawnOuterRadius", 400.0),
        }
    }
}

/// The spawner's state: the roads, the junctions and this frame's region list.
pub struct Spawner {
    pub roads: Vec<Road>,
    /// lane -> road (u32::MAX: a junction's lane)
    pub lane_road: Vec<u32>,
    /// junctions: centre, weight (+1c) and reach (+40)
    pub ints: Vec<(Vec3, f32, f32)>,
    pub tune: Tune,
    pub data_dir: String,
    /// the region list: roads with their weight (target share), and the junctions' weight
    pub list: Vec<(u32, f32)>,
    pub int_weight: f32,
    pub listed: Vec<bool>,
    pub target: f32,
    pub current: u32,
    /// 30 Hz frames run and the time not yet run
    frame: u32,
    step: f32,
    /// script densities by road (`RoadDensity`, road +164)
    pub script: hashbrown::HashMap<u32, i32>,
    /// (diagnostics) spawn tries turned down, by reason
    pub why: [u32; 8],
}

fn lerp(a: [f32; 2], t: f32) -> f32 {
    a[0] + (a[1] - a[0]) * t
}

/// Do two oriented rectangles (centre, unit x axis, half sizes along it and across) meet
/// (sub_8261B200, a 2D separating-axis test)?
fn rects_meet(c1: glam::Vec2, u1: glam::Vec2, h1: glam::Vec2, c2: glam::Vec2, u2: glam::Vec2, h2: glam::Vec2) -> bool {
    let d = c2 - c1;
    let (v1, v2) = (u1.perp(), u2.perp());
    for ax in [u1, v1, u2, v2] {
        let r1 = h1.x * u1.dot(ax).abs() + h1.y * v1.dot(ax).abs();
        let r2 = h2.x * u2.dot(ax).abs() + h2.y * v2.dot(ax).abs();
        if d.dot(ax).abs() > r1 + r2 {
            return false;
        }
    }
    true
}

impl Spawner {
    pub fn new(nav: &NavSys, root: &std::path::Path) -> Spawner {
        let lanes_n = nav.links.iter().map(|l| l.lane + 1).max().unwrap_or(0);
        let mut lane_road = vec![u32::MAX; lanes_n];
        let mut roads = Vec::with_capacity(nav.zones.len());
        for (zi, z) in nav.zones.iter().enumerate() {
            let mut lanes: Vec<(usize, f32)> = Vec::new();
            let mut seen: hashbrown::HashMap<usize, usize> = hashbrown::HashMap::new();
            for &k in &z.links {
                let lane = nav.links[k].lane;
                let e = *seen.entry(lane).or_insert_with(|| {
                    lanes.push((k, 0.0));
                    lanes.len() - 1
                });
                lanes[e].1 += nav.links[k].len;
                if k < lanes[e].0 {
                    lanes[e].0 = k;
                }
                lane_road[lane] = zi as u32;
            }
            let lane_len = lanes.iter().map(|l| l.1).sum();
            // (43 roads carry no rectangle: the lanes' extent instead)
            let (mid, dir, half) = if z.rect_len > 0.0 && z.width > 0.0 {
                (z.mid, z.dir, glam::Vec2::new(z.rect_len * 0.5, z.width * 0.5))
            } else {
                let pts: Vec<Vec3> = z.links.iter().flat_map(|&k| [nav.links[k].a, nav.links[k].b]).collect();
                if pts.is_empty() {
                    (Vec3::ZERO, Vec3::X, glam::Vec2::ZERO)
                } else {
                    let (mn, mx) = pts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), p| (a.min(*p), b.max(*p)));
                    ((mn + mx) * 0.5, Vec3::X, glam::Vec2::new((mx.x - mn.x) * 0.5, (mx.z - mn.z) * 0.5))
                }
            };
            roads.push(Road { mid, dir, half, speed: z.speed_limit_mps(), density: z.density, lanes, lane_len });
        }
        let ints = crate::assets::load_resource_file(&root.join("resources/city/sc/0x9960fcd8.r66"))
            .map(|r| mcla_formats::rage::navsys::parse_intersections(&r))
            .unwrap_or_default();
        let n = roads.len();
        Spawner { roads, lane_road, ints, tune: Tune::load(root), data_dir: root.to_string_lossy().into_owned(), list: Vec::new(), int_weight: 0.0, listed: vec![false; n], target: 0.0, current: 0, frame: 0, step: 0.0, script: Default::default(), why: [0; 8] }
    }

    /// The road a link belongs to.
    pub fn road_of(&self, nav: &NavSys, link: usize) -> Option<u32> {
        self.lane_road.get(nav.links.get(link)?.lane).copied().filter(|&r| r != u32::MAX)
    }

    /// The density value of a road (sub_8273B670): the script's index, else the road's own,
    /// else 3. In free roam with the player at 30 mph or more (mode byte +0x64250; the
    /// mode's meaning is a guess) the indices map 0 -> 0, 1-2 -> 0.55, 3-5 -> 1.25, 6 -> 0.10.
    fn value(&self, r: u32, free_fast: bool) -> f32 {
        let own = self.roads[r as usize].density;
        let idx = self.script.get(&r).copied().unwrap_or(if own >= 0 { own } else { 3 }).clamp(0, 6) as usize;
        if free_fast {
            return match idx {
                0 => 0.0,
                1 | 2 => 0.55,
                3..=5 => 1.25,
                _ => 0.10,
            };
        }
        self.tune.values[idx]
    }
}

/// Is a point seen from the focus (sub_826F0628 / sub_826F0870): nearer than 50 m always;
/// up to 250 m within the field of view either way along the view line (|cos|) and not
/// behind an occluder (sphere r 2, 1.5 m up); beyond, never.
fn seen(t: &Tune, focus: Vec3, view: Vec3, mph: f32, p: Vec3, occ: Option<&crate::occluder::Selected>) -> bool {
    let d = Vec3::new(p.x - focus.x, 0.0, p.z - focus.z);
    let dist = d.length();
    if dist < t.spawn_inner {
        return true;
    }
    if dist > t.spawn_outer {
        return false;
    }
    let k = ((mph - t.fov_mph[0]) / (t.fov_mph[1] - t.fov_mph[0])).clamp(0.0, 1.0);
    let fov = lerp(t.fov_deg, k).to_radians();
    if (d / dist).dot(view).abs() < fov.cos() {
        return false;
    }
    !occ.is_some_and(|o| o.hides_sphere(p + Vec3::Y * 1.5, 2.0))
}

impl Vehicles {
    /// Is car `i` ambient traffic the spawner owns?
    fn ambient_car(&self, i: usize) -> bool {
        let c = &self.cars[i];
        !c.is_player && !c.cop && !c.racer && !c.pinned && c.parked.is_none() && c.rail.is_some()
    }

    fn car_road(&self, sp: &Spawner, nav: &NavSys, i: usize) -> Option<u32> {
        let c = &self.cars[i];
        let link = match c.rail.as_ref() {
            Some(r) if !r.physical => Some(r.link),
            _ => c.ai.as_ref().and_then(|a| a.link),
        }?;
        sp.road_of(nav, link)
    }

    /// The original's traffic spawner and unspawner, run from `manage_traffic`.
    pub(super) fn spawn_traffic(&mut self, gpu: &crate::render::Gpu, world: &crate::world::World, center: Vec3, dt: f32) {
        let Some(nav) = world.nav.as_ref() else { return };
        if self.spawner.is_none() {
            self.spawner = Some(Spawner::new(nav, &world.assets.root));
        }
        let mut sp = self.spawner.take().unwrap();
        // roads a race script set the density of
        for (p, d) in std::mem::take(&mut self.road_density_new) {
            for i in nav.links_near(p, 30.0) {
                let l = &nav.links[i];
                let ab = l.b - l.a;
                let t = ((p - l.a).dot(ab) / ab.length_squared().max(1e-3)).clamp(0.0, 1.0);
                let q = l.a + ab * t;
                if (q.y - p.y).abs() < 7.0 && Vec3::new(q.x - p.x, 0.0, q.z - p.z).length() < 13.0 {
                    self.road_density.insert(l.lane, d);
                    if let Some(r) = sp.road_of(nav, i) {
                        sp.script.insert(r, d);
                    }
                }
            }
        }
        // the focus: the player's car, its heading and speed
        let (view, mph, player_link) = match self.cars.iter().find(|c| c.is_player) {
            Some(c) => {
                let f = Vec3::new(c.state.forward().x, 0.0, c.state.forward().z).normalize_or(Vec3::NEG_Z);
                (f, c.state.speed() / MPH, nav.nearest_link_aligned(c.state.pos, f).map(|x| x.0))
            }
            None => (Vec3::NEG_Z, 0.0, None),
        };
        let free_fast = !self.cars.iter().any(|c| c.pinned) && mph >= 30.0;
        let enabled = self.traffic_target > 0;
        sp.step = (sp.step + dt).min(4.0 / 30.0);
        while sp.step >= 1.0 / 30.0 {
            sp.step -= 1.0 / 30.0;
            sp.frame = sp.frame.wrapping_add(1);
            if sp.frame % 2 == 1 {
                self.region_list(&mut sp, nav, center, view, mph, player_link, free_fast);
                if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() && sp.frame % 30 == 1 {
                    log::info!("traffic regions {} target {:.2} current {} enabled {} mph {:.0} turned down (type, pool, junction, freeway, one way, room, seen) {:?}", sp.list.len(), sp.target, sp.current, enabled, mph, sp.why);
                }
            } else if enabled {
                self.spawn_frame(&mut sp, gpu, world, nav, center, view, mph, player_link);
                self.unspawn_frame(&sp, nav, center, view, mph);
            }
        }
        self.spawner = Some(sp);
    }

    /// sub_826F4E90: the roads in the box ahead of the focus, their weights and the target.
    #[allow(clippy::too_many_arguments)]
    fn region_list(&mut self, sp: &mut Spawner, nav: &NavSys, focus: Vec3, view: Vec3, mph: f32, player_link: Option<usize>, free_fast: bool) {
        let t = sp.tune;
        let k = (mph / 60.0).clamp(0.0, 1.0);
        let (fx, vx) = (glam::Vec2::new(focus.x, focus.z), glam::Vec2::new(view.x, view.z).normalize_or(glam::Vec2::NEG_Y));
        // the player's road; across the view at under 50 mph: the square box round the focus
        let player_road = player_link.and_then(|l| sp.road_of(nav, l));
        let perp = mph < 50.0 && player_road.is_some_and(|r| sp.roads[r as usize].dir.dot(view).abs() < 0.7071);
        let (len, off, width) = if perp { (lerp(t.perp_len, k), lerp(t.perp_off, k), lerp(t.perp_width, k)) } else { (lerp(t.box_len, k), lerp(t.box_off, k), lerp(t.box_width, k)) };
        let bc = fx + vx * off;
        let bh = glam::Vec2::new(len * 0.5, width * 0.5);
        let on_freeway = player_road.is_some_and(|r| sp.roads[r as usize].speed >= 29.0129);
        // (the camera's frustum is not at hand here: a road counts as out of the frustum when
        // its rectangle lies wholly outside 60 degrees either side of the view)
        let occ = crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE));
        let speed_factor = lerp(t.density_at, ((mph - t.density_mph[0]) / (t.density_mph[1] - t.density_mph[0])).clamp(0.0, 1.0));
        sp.list.clear();
        sp.listed.iter_mut().for_each(|x| *x = false);
        let mut target = 0.0;
        for ri in 0..sp.roads.len() {
            let r = &sp.roads[ri];
            if r.half.x <= 0.0 {
                continue;
            }
            let rc = glam::Vec2::new(r.mid.x, r.mid.z);
            if rc.distance(bc) > bh.length() + r.half.length() {
                continue;
            }
            let ru = glam::Vec2::new(r.dir.x, r.dir.z).normalize_or(glam::Vec2::X);
            let overlap = rects_meet(bc, vx, bh, rc, ru, r.half);
            let fw = r.speed >= 29.0129;
            let hidden = || {
                let corners = [rc + ru * r.half.x + ru.perp() * r.half.y, rc + ru * r.half.x - ru.perp() * r.half.y, rc - ru * r.half.x + ru.perp() * r.half.y, rc - ru * r.half.x - ru.perp() * r.half.y];
                let in_frustum = rc.distance(fx) < r.half.length() || corners.iter().any(|c| (*c - fx).normalize_or_zero().dot(vx) > 0.5);
                !in_frustum || occ.as_ref().is_some_and(|o| o.hides_sphere(r.mid + Vec3::Y * 1.0, r.half.length()))
            };
            let accept = if on_freeway && fw {
                true
            } else if !overlap {
                false
            } else if on_freeway {
                !hidden()
            } else {
                let near = rc.distance(fx) <= 200.0;
                let aligned = r.dir.dot(view) > 0.6;
                if fw && !(near || aligned) {
                    false
                } else {
                    (near && aligned) || !hidden()
                }
            };
            if !accept {
                continue;
            }
            let w = r.lane_len * 0.01 * sp.value(ri as u32, free_fast) * speed_factor;
            sp.list.push((ri as u32, w));
            sp.listed[ri] = true;
            target += w;
        }
        // the junctions in the box
        sp.int_weight = 0.0;
        for &(c, w, _) in &sp.ints {
            let p = glam::Vec2::new(c.x, c.z) - bc;
            if p.dot(vx).abs() <= bh.x && p.dot(vx.perp()).abs() <= bh.y {
                let iw = w * 0.01 * t.values[1] * speed_factor;
                sp.int_weight += iw;
                target += iw;
            }
        }
        if target > 0.0 && target < 1.0 {
            target = 1.0;
        }
        sp.target = target;
        // the cars on listed roads
        sp.current = (0..self.cars.len()).filter(|&i| self.ambient_car(i) && self.car_road(sp, nav, i).is_some_and(|r| sp.listed[r as usize])).count() as u32;
    }

    /// sub_826F6920: up to 60 tries, at most min(target - current, 6) cars.
    #[allow(clippy::too_many_arguments)]
    fn spawn_frame(&mut self, sp: &mut Spawner, gpu: &crate::render::Gpu, world: &crate::world::World, nav: &NavSys, focus: Vec3, view: Vec3, mph: f32, player_link: Option<usize>) {
        let mut n = (sp.target as i32 - sp.current as i32).min(6);
        if n <= 0 || sp.list.is_empty() || self.models.is_empty() {
            return;
        }
        // (the original also waits while the player stands in a junction under 30 mph - its
        // player-road object, +320; which road the original assigns the player is not decoded,
        // and the recorded original spawned with the player standing in a junction's box: not
        // ported)
        let _ = player_link;
        let t = sp.tune;
        let occ = crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE));
        let total: f32 = sp.list.iter().map(|x| x.1).sum::<f32>() + sp.int_weight;
        for _ in 0..60 {
            if n <= 0 {
                break;
            }
            // a type (the focus district's list at this hour; trailers and full pools are skipped)
            let r = self.rand();
            let models = &self.models;
            let Some(name) = self.ambient.pick(focus, self.hour, r, &|m| models.iter().any(|x| x.name.eq_ignore_ascii_case(m))).map(|s| s.to_string()) else {
                sp.why[0] += 1;
                continue;
            };
            if super::is_trailer(&name) {
                sp.why[0] += 1;
                continue;
            }
            let Some(mi) = self.models.iter().position(|m| m.name.eq_ignore_ascii_case(&name)) else {
                sp.why[0] += 1;
                continue;
            };
            let mname = self.models[mi].name.clone();
            let ty = *self.amb_types.entry(mname.clone()).or_insert_with(|| AmbType::load(&world.assets.root, &mname));
            let used = self.cars.iter().filter(|c| c.model.name == mname).count() as u32;
            if used >= ty.instances {
                sp.why[1] += 1;
                continue;
            }
            // a region by weight (a junction wastes the try)
            let mut x = self.rand() * total;
            let mut pick = None;
            for &(ri, w) in &sp.list {
                if x < w {
                    pick = Some(ri);
                    break;
                }
                x -= w;
            }
            let Some(ri) = pick else {
                sp.why[2] += 1;
                continue;
            };
            let road = &sp.roads[ri as usize];
            if road.lanes.is_empty() {
                continue;
            }
            // a lane by length, a point along it
            let mut x = self.rand() * road.lane_len;
            let mut lane = road.lanes[0];
            for &l in &road.lanes {
                lane = l;
                if x < l.1 {
                    break;
                }
                x -= l.1;
            }
            let mut s = self.rand() * lane.1;
            let mut link = lane.0;
            let lane_id = nav.links[link].lane;
            while s > nav.links[link].len {
                s -= nav.links[link].len;
                match nav.links[link].next.first() {
                    Some(&nk) if nav.links[nk].lane == lane_id => link = nk,
                    _ => {
                        s = nav.links[link].len;
                        break;
                    }
                }
            }
            let l = &nav.links[link];
            let pos = l.a + (l.b - l.a) * (s / l.len.max(1e-3));
            let dir = l.dir();
            let fw = road.speed >= 29.0129;
            if ty.freeway_only && !fw {
                sp.why[3] += 1;
                continue;
            }
            // one way of a road not crowded over the other
            if !fw {
                let (mut same, mut opp) = (0, 0);
                for i in 0..self.cars.len() {
                    if self.ambient_car(i) && self.car_road(sp, nav, i) == Some(ri) {
                        if self.cars[i].state.forward().dot(dir) > 0.0 {
                            same += 1
                        } else {
                            opp += 1
                        }
                    }
                }
                if same as f32 > 1.5 * opp as f32 && (same > 3 || opp > 3) {
                    sp.why[4] += 1;
                    continue;
                }
            }
            // room: no car within the spacing
            if self.cars.iter().any(|c| Vec3::new(c.state.pos.x - pos.x, 0.0, c.state.pos.z - pos.z).length() < t.spacing && (c.state.pos.y - pos.y).abs() < 5.0) {
                sp.why[5] += 1;
                continue;
            }
            // unseen (a long type at both ends too)
            if seen(&t, focus, view, mph, pos, occ.as_ref()) {
                sp.why[6] += 1;
                continue;
            }
            if ty.bound_group == 2 {
                let m = &self.models[mi];
                let h = (m.max.z - m.min.z) * 0.5;
                if seen(&t, focus, view, mph, pos + dir * h, occ.as_ref()) || seen(&t, focus, view, mph, pos - dir * h, occ.as_ref()) {
                    continue;
                }
            }
            let model = self.models[mi].clone();
            let color = if model.name.contains("taxi") { [0.95, 0.72, 0.05, 0.0] } else { super::TRAFFIC_PALETTE[(self.rand() * super::TRAFFIC_PALETTE.len() as f32) as usize % super::TRAFFIC_PALETTE.len()] };
            let yaw = (-dir.x).atan2(-dir.z);
            let idx = self.spawn(gpu, model, pos, yaw, color, false);
            let seed = (self.rand() * 16777216.0) as u32 | 1;
            let mut rider = Rider::new(nav, ty, link, s, seed);
            rider.born = self.clock;
            self.cars[idx].rail = Some(rider);
            if let Some(ai) = self.cars[idx].ai.as_mut() {
                ai.link = Some(link);
                ai.t = s / l.len.max(1e-3);
            }
            if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() {
                let v = pos - focus;
                log::info!("traffic spawn {} dist {:.1} ahead {:.1} side {:.1} road {} target {:.2} current {}", self.cars[idx].model.name, Vec3::new(v.x, 0.0, v.z).length(), v.dot(view), (v.z * view.x - v.x * view.z).abs(), ri, sp.target, sp.current);
            }
            sp.current += 1;
            n -= 1;
        }
    }

    /// sub_826F4390: 30 cars looked at, at most one removed.
    fn unspawn_frame(&mut self, sp: &Spawner, nav: &NavSys, focus: Vec3, view: Vec3, mph: f32) {
        let amb: Vec<usize> = (0..self.cars.len()).filter(|&i| self.ambient_car(i)).collect();
        if amb.is_empty() {
            return;
        }
        let occ = crate::occluder::city(&sp.data_dir).map(|c| c.select_around(focus + Vec3::Y * 1.75, OCC_RANGE));
        let mut road_cars: hashbrown::HashMap<u32, u32> = hashbrown::HashMap::new();
        for &i in &amb {
            if let Some(r) = self.car_road(sp, nav, i) {
                *road_cars.entry(r).or_default() += 1;
            }
        }
        let weight: hashbrown::HashMap<u32, f32> = sp.list.iter().copied().collect();
        for _ in 0..30 {
            let i = amb[(self.rand() * amb.len() as f32) as usize % amb.len()];
            let c = &self.cars[i];
            let pos = c.state.pos;
            if seen(&sp.tune, focus, view, mph, pos, occ.as_ref()) {
                continue;
            }
            let Some(r) = c.rail.as_ref() else { continue };
            let road = self.car_road(sp, nav, i);
            let listed = road.is_some_and(|x| sp.listed[x as usize]);
            let stuck = if r.physical { c.state.speed() < 6.7056 } else { r.v == 0.0 && r.next.is_none() && r.on == [false; 3] };
            let crowded = road.is_some_and(|x| weight.get(&x).is_some_and(|w| road_cars.get(&x).copied().unwrap_or(0) as f32 > 1.5 * w));
            let off_list = match road {
                Some(_) => !listed,
                None => Vec3::new(pos.x - focus.x, 0.0, pos.z - focus.z).length() >= sp.tune.unspawn_outer,
            };
            let mut remove = off_list || crowded || stuck;
            if remove && listed && self.clock - r.born < r.life {
                remove = false;
            }
            if remove {
                if std::env::var("MCLA_TRAFFIC_TRACE").is_ok() {
                    let v = pos - focus;
                    log::info!("traffic unspawn {} dist {:.1} ahead {:.1} age {:.1} off_list {} crowded {} stuck {}", c.model.name, Vec3::new(v.x, 0.0, v.z).length(), v.dot(view), self.clock - r.born, off_list, crowded, stuck);
                }
                self.cars.remove(i);
                return;
            }
        }
    }
}
