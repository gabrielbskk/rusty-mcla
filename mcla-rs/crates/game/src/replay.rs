//! VIEW REPLAY (the race-over menu's last row): the race just run, played again from its
//! grid with the replay's own cameras and the legend of the prompt movie
//! (`template_Replay`; references `work/ui/ref/y_replay*.png`, `vr_*.png`).
//!
//! The race's cars are recorded as they are simulated - thirty states a second each - and
//! put back state by state; nothing is simulated again.
//!
//! The cameras are the car's `tune/camera/<car>_replay.mcreplaycamera{,polar,fixed,heli}`:
//! six places on the car (`SideFwdOffset` ... `HoodLookbackOffset`), a camera that looks at
//! the car from around it, one that stands by the road and one high above; a view is held
//! `AverageCutDuration` +- `CutDurationVariance` seconds. (approx.) The places, lenses and
//! times are the files'; which way a camera on the car looks, where the others stand and
//! the order of the views are set by eye against the captures.

use crate::physics::car::CarState;
use crate::vehicles::CarInstance;
use glam::Vec3;
use std::path::Path;

/// seconds between two recorded states
pub const STEP: f32 = 1.0 / 30.0;
/// the longest race kept (states of one car)
const MOST: usize = 30 * 60 * 12;

#[derive(Default)]
pub struct Recording {
    /// per recorded car: its index among the vehicles and its states (with the bike's lean)
    pub tracks: Vec<(usize, Vec<(CarState, f32)>)>,
    acc: f32,
    /// the race clock when the last state was taken (a clock that runs back is a new race)
    clock: f32,
    /// the replay was left: the cars go back to where the race ended
    pub restore: bool,
    /// the race's countdown when the recording began (it runs down with the recording)
    pub countdown0: f32,
}

impl Recording {
    pub fn clear(&mut self) {
        self.tracks.clear();
        self.acc = 0.0;
        self.clock = 0.0;
    }

    pub fn frames(&self) -> usize {
        self.tracks.first().map_or(0, |t| t.1.len())
    }

    /// length in seconds
    pub fn len(&self) -> f32 {
        self.frames().saturating_sub(1) as f32 * STEP
    }

    /// One simulated step of a race: `clock` its time, `who` the cars that take part.
    pub fn record(&mut self, dt: f32, clock: f32, countdown: f32, cars: &[CarInstance], who: &[usize]) {
        if clock < self.clock || self.tracks.is_empty() {
            self.clear();
            self.countdown0 = countdown.max(0.0);
            self.tracks = who.iter().filter(|&&i| i < cars.len()).map(|&i| (i, Vec::new())).collect();
            self.acc = STEP;
        }
        self.clock = clock;
        self.acc += dt;
        if self.acc < STEP || self.frames() >= MOST {
            return;
        }
        self.acc = (self.acc - STEP).min(STEP);
        for (i, states) in self.tracks.iter_mut() {
            if let Some(c) = cars.get(*i) {
                states.push((c.state.clone(), c.lean));
            } else if let Some(last) = states.last().cloned() {
                states.push(last);
            }
        }
    }

    /// The state of a recorded car `t` seconds in.
    pub fn sample(states: &[(CarState, f32)], t: f32) -> Option<(CarState, f32)> {
        let last = states.len().checked_sub(1)?;
        let x = (t / STEP).clamp(0.0, last as f32);
        let i = (x.floor() as usize).min(last);
        let j = (i + 1).min(last);
        let f = x - i as f32;
        let (a, b) = (&states[i], &states[j]);
        let mut s = a.0.clone();
        s.pos = a.0.pos.lerp(b.0.pos, f);
        s.rot = a.0.rot.slerp(b.0.rot, f);
        s.vel = a.0.vel.lerp(b.0.vel, f);
        s.steer = a.0.steer + (b.0.steer - a.0.steer) * f;
        s.rpm = a.0.rpm + (b.0.rpm - a.0.rpm) * f;
        for k in 0..4 {
            let (wa, wb) = (&a.0.wheels[k], &b.0.wheels[k]);
            let w = &mut s.wheels[k];
            w.compression = wa.compression + (wb.compression - wa.compression) * f;
            w.spin = wa.spin + (wb.spin - wa.spin) * f;
            w.steer = wa.steer + (wb.steer - wa.steer) * f;
            w.world_pos = wa.world_pos.lerp(wb.world_pos, f);
            w.contact = wa.contact.lerp(wb.contact, f);
        }
        Some((s, a.1 + (b.1 - a.1) * f))
    }

    /// Puts the recorded cars where they were `t` seconds in.
    pub fn apply(&self, t: f32, cars: &mut [CarInstance]) {
        for (i, states) in &self.tracks {
            let (Some(c), Some((s, lean))) = (cars.get_mut(*i), Self::sample(states, t)) else { continue };
            c.state = s;
            c.lean = lean;
            c.pose_history.begin(&c.state, c.lean);
        }
    }
}

/// The car's replay camera files.
#[derive(Clone, Debug)]
pub struct CamTune {
    /// SideFwd, SideRear, WheelFwd, WheelRear, Roof, HoodLookback: x, y up, z to the rear
    pub mounts: [Vec3; 6],
    /// average, variance, least, most seconds of a view
    pub cut: [f32; 4],
    pub polar_fov: f32,
    pub fixed_fov: f32,
    /// the helicopter's lens: least, most, usual
    pub heli_fov: [f32; 3],
    pub heli_cut: [f32; 4],
}

impl Default for CamTune {
    /// (the Audi R8's files)
    fn default() -> Self {
        CamTune {
            mounts: [
                Vec3::new(-1.199245, 0.8420561, 0.2),
                Vec3::new(-1.099245, 0.8420561, 2.05057),
                Vec3::new(-1.199245, 0.52649701, 0.0),
                Vec3::new(-1.15, 0.52649701, 2.05057),
                Vec3::new(-0.99924529, 1.1916997, 1.025285),
                Vec3::new(-0.99924529, 1.1916997, -2.3400674),
            ],
            cut: [5.0, 3.0, 1.0, 10.0],
            polar_fov: 47.0,
            fixed_fov: 65.0,
            heli_fov: [4.0, 20.0, 7.0],
            heli_cut: [15.0, 5.0, 5.0, 30.0],
        }
    }
}

impl CamTune {
    pub fn load(data_dir: &str, car: &str) -> CamTune {
        let mut t = CamTune::default();
        let read = |ext: &str| -> Vec<(String, Vec<f32>)> {
            let h = mcla_formats::hash::joaat(&format!("{}_replay.mcreplaycamera{}", car.to_ascii_lowercase(), ext));
            let text = std::fs::read_to_string(Path::new(data_dir).join("tune/camera").join(format!("0x{h:08x}"))).unwrap_or_default();
            text.lines()
                .filter_map(|l| {
                    let mut it = l.split_whitespace();
                    let key = it.next()?.to_string();
                    let v: Vec<f32> = it.filter_map(|x| x.parse().ok()).collect();
                    (!v.is_empty()).then_some((key, v))
                })
                .collect()
        };
        const MOUNTS: [&str; 6] = ["SideFwdOffset", "SideRearOffset", "WheelFwdOffset", "WheelRearOffset", "RoofOffset", "HoodLookbackOffset"];
        const CUT: [&str; 4] = ["AverageCutDuration", "CutDurationVariance", "MinCutDuration", "MaxCutDuration"];
        for (k, v) in read("") {
            if let (Some(i), true) = (MOUNTS.iter().position(|m| *m == k), v.len() >= 3) {
                t.mounts[i] = Vec3::new(v[0], v[1], v[2]);
            } else if let Some(i) = CUT.iter().position(|m| *m == k) {
                t.cut[i] = v[0];
            }
        }
        for (k, v) in read("polar") {
            if k == "CameraFOV" {
                t.polar_fov = v[0];
            }
        }
        for (k, v) in read("fixed") {
            if k == "CameraFOV" {
                t.fixed_fov = v[0];
            }
        }
        for (k, v) in read("heli") {
            match k.as_str() {
                "MinFOV" => t.heli_fov[0] = v[0],
                "MaxFOV" => t.heli_fov[1] = v[0],
                "DefaultFOV" => t.heli_fov[2] = v[0],
                _ => {
                    if let Some(i) = CUT.iter().position(|m| *m == k) {
                        t.heli_cut[i] = v[0];
                    }
                }
            }
        }
        t
    }
}

/// the views NEXT CAMERA steps through: the six on the car, around it, by the road, above
pub const VIEWS: usize = 9;
/// seconds of the opening shot from the air before the recording's first state (the
/// replay's clock starts that much below zero; reference `y_replay_intro`)
pub const OPENING: f32 = 4.0;
const POLAR: usize = 6;
const FIXED: usize = 7;
const HELI: usize = 8;
/// (approx.) the lens of the cameras on the car: their file names none
const MOUNT_FOV: f32 = 62.0;

pub struct Playback {
    pub t: f32,
    pub paused: bool,
    /// Back steps: the whole legend, the one line HIDE HUD, nothing at all
    pub legend: u8,
    /// the views change by themselves (DISABLE / ENABLE CAMERA SWITCHING)
    pub auto_cam: bool,
    pub view: usize,
    /// seconds left of the view
    cut: f32,
    /// the replay's PAUSE panel and its cursor
    pub menu: Option<usize>,
    rng: u32,
    /// where the camera by the road stands, and the one above
    anchor: Option<Vec3>,
    heli: Option<Vec3>,
    /// the angle about the car of the camera around it
    az: f32,
    pub tune: CamTune,
    /// the other traffic was taken off the road
    pub started: bool,
}

impl Playback {
    pub fn new(tune: CamTune, seed: u32) -> Playback {
        let mut p = Playback { t: -OPENING, paused: false, legend: 0, auto_cam: true, view: FIXED, cut: 0.0, menu: None, rng: seed | 1, anchor: None, heli: None, az: 0.0, tune, started: false };
        p.cut = p.hold();
        p
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / (1u32 << 24) as f32
    }

    /// how long the view now on is held
    fn hold(&mut self) -> f32 {
        let c = if self.view == HELI { self.tune.heli_cut } else { self.tune.cut };
        (c[0] + (self.rand() * 2.0 - 1.0) * c[1]).clamp(c[2], c[3].max(c[2]))
    }

    pub fn restart(&mut self) {
        self.t = -OPENING;
        self.paused = false;
        self.menu = None;
        self.set_view(FIXED);
    }

    pub fn set_view(&mut self, v: usize) {
        self.view = v % VIEWS;
        self.anchor = None;
        self.heli = None;
        self.az = (self.rand() - 0.5) * 1.2;
        self.cut = self.hold();
    }

    pub fn next_view(&mut self) {
        self.set_view(self.view + 1);
    }

    /// The camera of this frame: eye, what it looks at, the lens in degrees.
    pub fn camera(&mut self, car: &CarState, dt: f32) -> (Vec3, Vec3, f32) {
        if self.auto_cam && !self.paused {
            self.cut -= dt;
            if self.cut <= 0.0 {
                let mut v = (self.rand() * VIEWS as f32) as usize % VIEWS;
                if v == self.view {
                    v = (v + 1) % VIEWS;
                }
                self.set_view(v);
            }
        }
        let fwd = car.forward();
        let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
        if self.t < 0.0 {
            // the opening: down out of the sky onto the grid, as a race opens (main.rs)
            let k = 1.0 + self.t / OPENING;
            let e = (1.0 - (std::f32::consts::PI * k).cos()) * 0.5;
            let side = Vec3::new(-flat.z, 0.0, flat.x);
            let high = car.pos - flat * 260.0 + side * 120.0 + Vec3::Y * 520.0;
            let low = car.pos - flat * 6.0 + Vec3::Y * 58.0;
            return (high.lerp(low, e), car.pos, 50.0);
        }
        let up = car.rot * Vec3::Y;
        let right = fwd.cross(up).normalize_or(Vec3::X);
        match self.view {
            POLAR => {
                let side = Vec3::new(-flat.z, 0.0, flat.x);
                let dir = -flat * self.az.cos() + side * self.az.sin();
                (car.pos + dir * 6.5 + Vec3::Y * 1.5, car.pos + Vec3::Y * 0.6, self.tune.polar_fov)
            }
            FIXED => {
                // by the road ahead of the car; a new place once the car is well past it
                let gone = self.anchor.map_or(true, |a| {
                    let d = a - car.pos;
                    d.length() > 140.0 || (d.dot(flat) < 0.0 && d.length() > 45.0)
                });
                if gone {
                    let side = Vec3::new(-flat.z, 0.0, flat.x) * if self.rand() < 0.5 { -4.5 } else { 4.5 };
                    let ahead = (car.vel.length() * 2.5).clamp(30.0, 90.0);
                    self.anchor = Some(car.pos + flat * ahead + side + Vec3::Y * 0.35);
                }
                (self.anchor.unwrap_or(car.pos), car.pos + Vec3::Y * 0.5, self.tune.fixed_fov)
            }
            HELI => {
                let want = car.pos - flat * 110.0 + Vec3::Y * 95.0;
                let k = 1.0 - (-dt).exp();
                let eye = match self.heli {
                    Some(e) if !self.paused => e + (want - e) * k,
                    Some(e) => e,
                    None => want,
                };
                self.heli = Some(eye);
                (eye, car.pos, self.tune.heli_fov[2].clamp(self.tune.heli_fov[0], self.tune.heli_fov[1]))
            }
            m => {
                // on the car: the files' -x is the car's right (the captures show the car's
                // right side from every one of them)
                let o = self.tune.mounts[m.min(5)];
                let eye = car.pos - right * o.x + up * o.y - fwd * o.z;
                let look = if m == 5 { eye - fwd * 10.0 } else { eye + fwd * 10.0 };
                (eye, look, MOUNT_FOV)
            }
        }
    }
}
