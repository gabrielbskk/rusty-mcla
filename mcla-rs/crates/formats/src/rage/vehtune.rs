//! Vehicle tuning resource (type 50, one per car). The root holds a pointer list at
//! +0x50 to tuning components; each component is `{LE vtable, name ptr, type ptr,
//! source ptr, 0, 1, floats...}` with type names such as `vehSim`, `vehGrip`,
//! `vehGyro`, `vehModel`. Two entries carry no names: the engine/drivetrain tables
//! (base and modded), which start with the top speed in m/s and hold the gear
//! ratios and a 40-sample torque curve.

use super::Rd;
use crate::rsc::Resource;

#[derive(Debug, Clone, Default)]
pub struct Engine {
    /// top speed, m/s
    pub top_speed: f32,
    /// gear ratios, first gear first (highest ratio)
    pub gears: Vec<f32>,
    /// torque curve, 40 samples from idle to redline (arbitrary units)
    pub torque: Vec<f32>,
    /// engine speed scale (the raw value is about redline / 2.5)
    pub rpm_scale: f32,
    /// reverse gear ratio (first entry of the gear list)
    pub reverse: f32,
    /// full-throttle acceleration (m/s^2) against speed: evenly spaced samples from
    /// standstill to the top speed. Integrating it reproduces the real cars
    /// (Golf GTI 0-60 mph 7.8 s, Gallardo 3.8 s, Skyline 4.6 s).
    pub accel: Vec<f32>,
    /// maximum braking deceleration, ft/s^2 in the file, stored here in m/s^2
    pub brake_decel: f32,
}

/// Per-axle wheel data of the `vehSim` component.
#[derive(Debug, Clone, Copy, Default)]
pub struct Axle {
    pub radius: f32,
    /// maximum steering angle (rad); 0 on the rear axle
    pub steer: f32,
    pub brake: f32,
    pub handbrake: f32,
    /// tyre friction coefficient
    pub grip: f32,
}

#[derive(Debug, Clone, Default)]
pub struct VehTune {
    pub name: String,
    pub mass: f32,
    pub mass_mods: f32,
    /// width, height, length of the body (m)
    pub dims: [f32; 3],
    /// 0 = RWD, 1 = FWD, 2 = AWD
    pub drivetrain: u32,
    /// grip triples from vehGrip (lateral/longitudinal multipliers)
    pub grip_a: [f32; 3],
    pub grip_b: [f32; 3],
    pub grip_c: [f32; 2],
    pub engine: Engine,
    pub engine_mods: Engine,
    /// centre of mass offset from the model origin (x, y, z)
    pub com: [f32; 3],
    pub front: Axle,
    pub rear: Axle,
    /// engine: horsepower, idle / redline / maximum rpm and the torque curve in lb-ft
    /// (evenly spaced from idle to the maximum rpm)
    pub horsepower: f32,
    pub idle_rpm: f32,
    pub redline: f32,
    pub max_rpm: f32,
    pub torque_lbft: Vec<f32>,
}

impl VehTune {
    pub fn parse(res: &Resource) -> Option<VehTune> {
        let rd = Rd::new(res);
        let mut t = VehTune::default();
        t.name = rd.ptr(0x04).map(|_| rd.cstr(0x08)).unwrap_or_default();
        let mut engines = Vec::new();
        let mut o = 0x50;
        while let Some(c) = rd.ptr(o) {
            o += 4;
            let is_named = rd.ptr(c + 4).is_some() && rd.ptr(c + 8).is_some();
            if !is_named {
                engines.push(c);
                continue;
            }
            let ty = rd.ptr(c + 8).map(|p| rd.cstr(p)).unwrap_or_default();
            let nm = rd.ptr(c + 4).map(|p| rd.cstr(p)).unwrap_or_default();
            let f = |i: usize| rd.f32(c + 0x18 + i * 4);
            match ty.as_str() {
                "vehSim" => {
                    let mods = nm.ends_with("_mods");
                    let mass = f(22);
                    if mods {
                        t.mass_mods = mass;
                    } else {
                        t.mass = mass;
                        t.dims = [f(10), f(11), f(12)];
                        t.drivetrain = rd.u32(c + 0x18 + 27 * 4);
                        t.com = [f(6), f(7), f(8)];
                        // sub-objects: slot 28 front axle, 29 rear axle, 30 engine
                        let sub = |slot: usize| rd.ptr(c + 0x18 + slot * 4);
                        let axle = |slot: usize| -> Axle {
                            match sub(slot) {
                                Some(a) => {
                                    let g = |i: usize| rd.f32(a + i * 4);
                                    if std::env::var("MCLA_AXLE_DUMP").is_ok() {
                                        eprintln!("axle slot {}: {:?}", slot, (0..30).map(|i| format!("{:.3}", g(i))).collect::<Vec<_>>());
                                    }
                                    Axle { radius: g(1), steer: g(10), brake: g(15), handbrake: g(16), grip: g(24) }
                                }
                                None => Axle::default(),
                            }
                        };
                        t.front = axle(28);
                        t.rear = axle(29);
                        if let Some(e) = sub(30) {
                            let g = |i: usize| rd.f32(e + i * 4);
                            t.horsepower = g(1);
                            t.idle_rpm = g(2);
                            t.redline = g(3);
                            t.max_rpm = g(4);
                            let n = (rd.u32(e + 13 * 4) as usize).min(16);
                            t.torque_lbft = (0..n).map(|i| rd.u32(e + (14 + i) * 4) as f32).collect();
                        }
                    }
                }
                "vehGrip" => {
                    // nested grip object at +0x70
                    let g = c + 0x70;
                    let gf = |i: usize| rd.f32(g + 0x10 + i * 4);
                    t.grip_a = [gf(0), gf(1), gf(2)];
                    t.grip_b = [gf(8), gf(9), gf(10)];
                    t.grip_c = [gf(12), gf(13)];
                }
                _ => {}
            }
            if o > 0x200 {
                break;
            }
        }
        for (k, e) in engines.iter().enumerate().take(2) {
            let f = |i: usize| rd.f32(e + i * 4);
            let mut eng = Engine { top_speed: f(0), rpm_scale: f(8), reverse: f(11), brake_decel: f(3) * 0.3048, ..Default::default() };
            // f[11] is the reverse gear; the forward gears follow, top gear first
            let mut i = 12;
            while i < 32 && f(i) > 0.01 {
                eng.gears.push(f(i));
                i += 1;
            }
            // skip the terminating zero(s)
            while i < 40 && f(i) == 0.0 {
                i += 1;
            }
            // acceleration against speed (up to 128 samples, zero-terminated)
            let mut j = 0;
            while j < 128 && f(i + j) > 0.0 && f(i + j) < 100.0 {
                eng.accel.push(f(i + j));
                j += 1;
            }
            if eng.accel.len() < 16 {
                eng.accel.clear();
            }
            for j in 0..40 {
                eng.torque.push(f(i + j));
            }
            eng.gears.sort_by(|a, b| b.partial_cmp(a).unwrap());
            if k == 0 {
                t.engine = eng;
            } else {
                t.engine_mods = eng;
            }
        }
        if t.mass <= 0.0 || t.engine.top_speed <= 0.0 {
            return None;
        }
        Some(t)
    }
}
