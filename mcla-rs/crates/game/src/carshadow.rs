//! The solids the original adds to a car in its shadow passes (LIGHTING_WORK.md, car round 13): the
//! `occluder_0` box of the car's files (`0x159c6aaa.r63`, "boxoccluder": 24 vertices) - on the Audi R8 as wide
//! as the wheels' middles + 2 cm, from 7 cm behind the rear axle to 5 cm before the front one, from 0.17 under
//! the wheels' middles to 0.44 above them - and a sphere in each wheel (0.48 - 0.58 m across, at the wheel's
//! middle): no light under the body between the wheels or through the rims.
//! (approx.) the box is built from the wheels' places with the R8's margins, not read from the file; the
//! spheres are 0.8 of the wheel's radius. `MCLA_CAR_SOLID_NOSHADOW=1`: off.

use crate::render::{Gpu, Vertex};
use glam::Vec3;
use std::sync::Mutex;
use wgpu::util::DeviceExt;

pub struct Solid {
    pub vb: wgpu::Buffer,
    pub ib: wgpu::Buffer,
    /// index ranges: the box (car space), the front wheels' sphere, the rear wheels' (unit wheel space)
    pub ranges: [(u32, u32); 3],
}

static REG: Mutex<Option<std::collections::HashMap<String, &'static Solid>>> = Mutex::new(None);

pub fn off() -> bool {
    static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *V.get_or_init(|| std::env::var("MCLA_CAR_SOLID_NOSHADOW").is_ok())
}

fn quad(v: &mut Vec<Vertex>, ix: &mut Vec<u32>, p: [Vec3; 4]) {
    let b = v.len() as u32;
    for q in p {
        v.push(Vertex { pos: q.to_array(), normal: [0.0, 1.0, 0.0], uv0: [0.0; 2], uv1: [0.0; 2], color: [255; 4], color1: [255; 4], uv2: [0.0; 2] });
    }
    // (both windings: whatever the pass culls)
    ix.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3, b, b + 2, b + 1, b, b + 3, b + 2]);
}

/// The solids of a car model, made once: `hubs` the wheels' middles in car space, `wheel` (radius, width)
/// front and rear.
pub fn get(gpu: &Gpu, key: &str, hubs: [Vec3; 4], wheel: [(f32, f32); 2]) -> &'static Solid {
    let mut g = REG.lock().unwrap();
    let map = g.get_or_insert_with(Default::default);
    if let Some(s) = map.get(key) {
        return s;
    }
    let (mut v, mut ix) = (Vec::new(), Vec::new());
    let hx = hubs.iter().map(|h| h.x.abs()).sum::<f32>() / 4.0 + 0.02;
    let hy = hubs.iter().map(|h| h.y).sum::<f32>() / 4.0;
    // (forward is -z: 5 cm before the front axle, 7 cm behind the rear one)
    let z0 = hubs.iter().map(|h| h.z).fold(f32::MAX, f32::min) - 0.05;
    let z1 = hubs.iter().map(|h| h.z).fold(f32::MIN, f32::max) + 0.07;
    let (y0, y1) = (hy - 0.17, hy + 0.44);
    let c = |i: u32| Vec3::new(if i & 1 == 0 { -hx } else { hx }, if i & 2 == 0 { y0 } else { y1 }, if i & 4 == 0 { z0 } else { z1 });
    for f in [[0, 1, 3, 2], [4, 5, 7, 6], [0, 1, 5, 4], [2, 3, 7, 6], [0, 2, 6, 4], [1, 3, 7, 5]] {
        quad(&mut v, &mut ix, [c(f[0]), c(f[1]), c(f[2]), c(f[3])]);
    }
    let mut ranges = [(0u32, ix.len() as u32); 3];
    for (k, (r, w)) in wheel.iter().enumerate() {
        let first = ix.len() as u32;
        // a sphere 0.8 R in the wheel's own measure (the wheel's matrix scales by 2 w, 2 R, 2 R)
        let (rx, ryz) = (0.8 * r / (2.0 * w.max(0.05)), 0.4);
        let p = |a: usize, b: usize| {
            let (t, f) = (a as f32 / 6.0 * std::f32::consts::PI, b as f32 / 10.0 * std::f32::consts::TAU);
            Vec3::new(rx * t.cos(), ryz * t.sin() * f.cos(), ryz * t.sin() * f.sin())
        };
        for a in 0..6 {
            for b in 0..10 {
                quad(&mut v, &mut ix, [p(a, b), p(a + 1, b), p(a + 1, b + 1), p(a, b + 1)]);
            }
        }
        ranges[k + 1] = (first, ix.len() as u32);
    }
    let vb = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("car shadow solids"), contents: bytemuck::cast_slice(&v), usage: wgpu::BufferUsages::VERTEX });
    let ib = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("car shadow solids"), contents: bytemuck::cast_slice(&ix), usage: wgpu::BufferUsages::INDEX });
    let s: &'static Solid = Box::leak(Box::new(Solid { vb, ib, ranges }));
    map.insert(key.to_string(), s);
    s
}
