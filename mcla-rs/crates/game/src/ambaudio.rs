//! The original's district ambience (the ambience manager, sub_821EB810 and friends): the district whose circles
//! are nearest picks the bed (day / night), its distant one-shots and surround "quadshots" fire on random timers
//! 6 km away in random directions, the two nearest location emitters play at their places, faded by distance, and
//! a district's line emitter follows the listener along its polyline. Rules: `docs/reference/audio/AMBIENCE_RULE.md`.
//!
//! Not ported (open there): the bed duck near an emitter (a dynamic mixer, MC4_HANGOUT_DYNMIXER_APPLY), the
//! district cross-fade, the freeway / tunnel / interior / mall districts (no circles: chosen by something else),
//! map-placed emitters, rain / wind / thunder.

use crate::sndobj::{Handle, Req, SoundLib};
use glam::Vec3;
use mcla_formats::hash::joaat;

fn be_u32(o: &[u8], at: usize) -> u32 {
    o.get(at..at + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap())).unwrap_or(0)
}
fn be_f32(o: &[u8], at: usize) -> f32 {
    f32::from_bits(be_u32(o, at))
}
fn be_u16(o: &[u8], at: usize) -> u16 {
    o.get(at..at + 2).map(|b| u16::from_be_bytes(b.try_into().unwrap())).unwrap_or(0)
}

const NULL_SOUND: &str = "NULL_SOUND";

#[derive(Clone, Copy, Debug)]
struct Shot {
    flags: u8,
    sound: u32,
    /// dB
    volume: f32,
    /// degrees, None = any direction
    centre: Option<f32>,
    spread: f32,
    min: f32,
    max: f32,
}

#[derive(Clone, Copy, Debug)]
struct Loc {
    flags: u8,
    pos: Vec3,
    outer: f32,
    inner: f32,
    /// day sound, day companion, night sound, night companion
    sounds: [u32; 4],
}

#[derive(Clone, Debug)]
struct District {
    name: String,
    bed: [u32; 2],
    line_dist: f32,
    line_sound: u32,
    circles: Vec<(f32, f32, f32)>,
    shots: Vec<Shot>,
    quads: Vec<Shot>,
    locs: Vec<Loc>,
    line: Vec<Vec3>,
}

fn parse_district(name: &str, o: &[u8]) -> Option<District> {
    if o.first() != Some(&5) {
        return None;
    }
    let mut p = 59;
    let na = *o.get(p)? as usize;
    p += 1;
    let circles = (0..na).map(|k| (be_f32(o, p + 12 * k), be_f32(o, p + 12 * k + 4), be_f32(o, p + 12 * k + 8))).collect();
    p += 12 * na;
    let mut shots = [Vec::new(), Vec::new()];
    for list in shots.iter_mut() {
        let n = *o.get(p)? as usize;
        p += 1;
        for k in 0..n {
            let e = p + 19 * k;
            let c = be_u16(o, e + 7);
            list.push(Shot {
                flags: o[e],
                sound: be_u32(o, e + 1),
                volume: be_u16(o, e + 5) as i16 as f32 * 0.01,
                centre: (c != 0xFFFF).then_some(c as f32),
                spread: be_u16(o, e + 9) as i16 as f32,
                min: be_f32(o, e + 11),
                max: be_f32(o, e + 15),
            });
        }
        p += 19 * n;
    }
    let nd = *o.get(p)? as usize;
    p += 1;
    let locs = (0..nd)
        .map(|k| {
            let e = p + 41 * k;
            Loc {
                flags: o[e],
                pos: Vec3::new(be_f32(o, e + 1), be_f32(o, e + 5), be_f32(o, e + 9)),
                outer: be_f32(o, e + 17),
                inner: be_f32(o, e + 21),
                sounds: [be_u32(o, e + 25), be_u32(o, e + 29), be_u32(o, e + 33), be_u32(o, e + 37)],
            }
        })
        .collect();
    p += 41 * nd;
    let ne = o.get(p).copied().unwrap_or(0) as usize;
    p += 1;
    let line = (0..ne).map(|k| Vec3::new(be_f32(o, p + 12 * k), be_f32(o, p + 12 * k + 4), be_f32(o, p + 12 * k + 8))).collect();
    let [shots, quads] = shots;
    Some(District { name: name.to_string(), bed: [be_u32(o, 10), be_u32(o, 14)], line_dist: be_f32(o, 39), line_sound: be_u32(o, 43), circles, shots, quads, locs, line })
}

struct Slot {
    /// (district, location record)
    key: (usize, usize),
    sound: Handle,
    companion: Option<Handle>,
}

pub struct Ambience {
    districts: Vec<District>,
    garage: Option<usize>,
    cur: Option<usize>,
    night: bool,
    bed: Option<Handle>,
    timers: Vec<f32>,
    quad_timers: Vec<f32>,
    shots: Vec<Handle>,
    slots: Vec<Slot>,
    line: Option<Handle>,
    landmark_curve: u32,
    null: u32,
    now: f32,
}

/// Voices for the distant one-shots and quadshots at once (the manager's free-slot lists; size not decoded).
const SHOT_SLOTS: usize = 4;

impl Ambience {
    pub fn new(lib: &SoundLib) -> Option<Ambience> {
        let g = &lib.game;
        let glob = g.get_name("AMBIENCE_GLOBAL")?;
        let n = *glob.get(70)? as usize;
        let mut districts: Vec<District> = (0..n).filter_map(|k| {
            let h = be_u32(glob, 71 + 4 * k);
            parse_district(g.name_of(h)?, g.get(h)?)
        }).collect();
        let garage = g.get_name("DISTRICT_GARAGE").and_then(|o| parse_district("DISTRICT_GARAGE", o)).map(|d| {
            districts.push(d);
            districts.len() - 1
        });
        log::info!("audio: ambience - {} districts", districts.len());
        Some(Ambience {
            districts,
            garage,
            cur: None,
            night: false,
            bed: None,
            timers: Vec::new(),
            quad_timers: Vec::new(),
            shots: Vec::new(),
            slots: Vec::new(),
            line: None,
            landmark_curve: joaat("MC4_HANGOUT_LANDMARK_APPLY"),
            null: joaat(NULL_SOUND),
            now: 0.0,
        })
    }

    /// The district the listener is in (sub_821EB810): the lowest squared distance over weight of all circles.
    fn pick(&self, l: Vec3) -> Option<usize> {
        let mut best = None;
        for (i, d) in self.districts.iter().enumerate() {
            for &(x, z, w) in &d.circles {
                let s = ((x - l.x).powi(2) + (z - l.z).powi(2)) / w;
                if best.map_or(true, |(b, _)| s < b) {
                    best = Some((s, i));
                }
            }
        }
        best.map(|b| b.1)
    }

    pub fn district_name(&self) -> Option<&str> {
        self.cur.map(|i| self.districts[i].name.as_str())
    }

    /// Stop everything (out of the city: menus without the garage, cutscenes ...).
    pub fn stop(&mut self, lib: &mut SoundLib) {
        for h in self.bed.take().into_iter().chain(self.line.take()) {
            lib.stop(h);
        }
        for s in self.slots.drain(..) {
            lib.stop(s.sound);
            if let Some(c) = s.companion {
                lib.stop(c);
            }
        }
        self.cur = None;
    }

    pub fn update(&mut self, lib: &mut SoundLib, listener: Vec3, night: bool, in_garage: bool, dt: f32) {
        self.now += dt;
        let now = self.now;
        let want = if in_garage { self.garage } else { self.pick(listener) };
        let Some(di) = want else {
            self.stop(lib);
            return;
        };
        let changed = self.cur != Some(di);
        if changed || night != self.night {
            // the bed of the district for the time of day (+10 / +14)
            if let Some(h) = self.bed.take() {
                lib.stop(h);
            }
            let snd = self.districts[di].bed[night as usize];
            if snd != self.null {
                self.bed = lib.play_hash(snd, Req::default());
                log::info!("audio: ambience bed {} {}", lib.name_of(snd).unwrap_or("?"), if self.bed.is_some() { "playing" } else { "missing" });
            }
        }
        if changed {
            let d = &self.districts[di];
            log::info!("audio: ambience district {}", d.name);
            // (the timers' start on entering a district is not decoded: each waits one random interval)
            let (shots, quads) = (d.shots.clone(), d.quads.clone());
            self.timers = shots.iter().map(|s| now + lib.rand_float(s.min, s.max)).collect();
            self.quad_timers = quads.iter().map(|s| now + lib.rand_float(s.min, s.max)).collect();
            if let Some(h) = self.line.take() {
                lib.stop(h);
            }
            self.cur = Some(di);
        }
        self.night = night;
        let d = self.districts[di].clone();
        // distant one-shots and quadshots (sub_821E8B18)
        self.shots.retain(|h| lib.playing(*h));
        for (list, timers) in [(&d.shots, 0usize), (&d.quads, 1)] {
            for (k, s) in list.iter().enumerate() {
                let t = if timers == 0 { &mut self.timers[k] } else { &mut self.quad_timers[k] };
                if now < *t {
                    continue;
                }
                *t = now + lib.rand_float(s.min, s.max);
                if self.shots.len() >= SHOT_SLOTS {
                    continue;
                }
                if s.flags & if night { 2 } else { 1 } == 0 {
                    continue;
                }
                let angle = match s.centre {
                    Some(c) => (lib.rand_float(-s.spread, s.spread) + c).to_radians(),
                    None => lib.rand_float(-std::f32::consts::PI, std::f32::consts::PI),
                };
                let pos = listener + Vec3::new(-6000.0 * angle.sin(), 0.0, -6000.0 * angle.cos());
                if let Some(h) = lib.play_hash(s.sound, Req { pos: Some(pos), volume: s.volume, ..Default::default() }) {
                    self.shots.push(h);
                }
            }
        }
        // location emitters (sub_821EA288 -> sub_821E9390): the two nearest inside their outer radius
        let mut cands: Vec<(f32, usize)> = Vec::new();
        for (k, l) in d.locs.iter().enumerate() {
            // (flag bit 0 takes another path, gated by manager +1028 / +1032: not decoded, none in the data)
            if l.flags & 1 != 0 {
                continue;
            }
            let snd = l.sounds[if night { 2 } else { 0 }];
            if snd == self.null || snd == 0 {
                continue;
            }
            let d2 = l.pos.distance_squared(listener);
            if d2 <= l.outer * l.outer {
                cands.push((d2, k));
            }
        }
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        cands.truncate(2);
        // slots whose emitter is no longer among the two stop
        self.slots.retain(|s| {
            let keep = s.key.0 == di && cands.iter().any(|c| c.1 == s.key.1) && lib.playing(s.sound);
            if !keep {
                lib.stop(s.sound);
                if let Some(c) = s.companion {
                    lib.stop(c);
                }
            }
            keep
        });
        for (d2, k) in cands {
            let l = d.locs[k];
            let dist = d2.sqrt();
            let span = (l.outer - l.inner).max(1e-3);
            let t = (dist.clamp(l.inner, l.outer) - l.inner) / span;
            let b = lib.curve_eval(self.landmark_curve, t).unwrap_or(1.0 - t);
            let vol = 20.0 * b.max(1e-5).log10();
            let req = Req { pos: Some(l.pos), volume: vol, ..Default::default() };
            match self.slots.iter().find(|s| s.key == (di, k)) {
                Some(s) => {
                    lib.set(s.sound, req);
                    if let Some(c) = s.companion {
                        lib.set(c, req);
                    }
                }
                None => {
                    let n = night as usize * 2;
                    if let Some(h) = lib.play_hash(l.sounds[n], req) {
                        let companion = (l.sounds[n + 1] != self.null).then(|| lib.play_hash(l.sounds[n + 1], req)).flatten();
                        self.slots.push(Slot { key: (di, k), sound: h, companion });
                    }
                }
            }
        }
        // the line emitter (sub_821E9068): the polyline's nearest point, within +39 m
        if d.line.len() >= 2 && d.line_sound != self.null {
            let mut best = (f32::MAX, Vec3::ZERO);
            for w in d.line.windows(2) {
                let ab = w[1] - w[0];
                let t = ((listener - w[0]).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
                let p = w[0] + ab * t;
                let dd = p.distance(listener);
                if dd < best.0 {
                    best = (dd, p);
                }
            }
            if best.0 <= d.line_dist {
                let req = Req::at(best.1);
                match self.line {
                    Some(h) if lib.playing(h) => lib.set(h, req),
                    _ => self.line = lib.play_hash(d.line_sound, req),
                }
            } else if let Some(h) = self.line.take() {
                lib.stop(h);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc and the converted audio"]
    fn districts_from_disc() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let a = crate::audio::Audio::new_offline(&dir).expect("audio");
        let lib = SoundLib::load(a.root(), &a.cfg).expect("sound objects");
        let amb = Ambience::new(&lib).expect("ambience");
        assert_eq!(amb.districts.len(), 17);
        let skid = amb.districts.iter().find(|d| d.name == "DISTRICT_SKID_ROW").unwrap();
        assert_eq!(skid.circles.len(), 3);
        assert_eq!(skid.shots.len(), 15);
        assert_eq!(skid.quads.len(), 2);
        assert_eq!(skid.locs.len(), 10);
        assert_eq!(skid.locs[0].outer, 66.0);
        assert_eq!(lib.name_of(skid.bed[0]), Some("BED_SANTA_MONICA_CITY_MASTER"));
        // the circles' centres pick their own district
        for (i, d) in amb.districts.iter().enumerate() {
            for &(x, z, _) in &d.circles {
                assert_eq!(amb.pick(Vec3::new(x, 0.0, z)), Some(i), "{}", d.name);
            }
        }
        let sm = amb.districts.iter().find(|d| d.name == "DISTRICT_SANTA_MONICA").unwrap();
        assert_eq!(sm.line.len(), 7);
    }
}
