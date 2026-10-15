//! City light sources (`resources/city/sc/0xaf69cf2f.r226`): root+0x50 -> array of
//! 0x60-byte records, u16 count at root+0x54.
//!
//! ```text
//! +00 vec3 position   +10 vec3 colour   +20 vec3 direction
//! +40 f32 range       +44 f32 cone angle (360 = omni)   +50 f32 intensity   +58 u16 type
//! ```

use super::Rd;
use crate::rsc::Resource;
use glam::Vec3;

#[derive(Debug, Clone, Copy)]
pub struct CityLight {
    pub pos: Vec3,
    pub color: Vec3,
    pub dir: Vec3,
    pub range: f32,
    pub cone: f32,
    pub intensity: f32,
    pub kind: u16,
    /// +4C / +50: the cone as the night lamp table takes it, `sat(cos x scale + bias)` (0, 1 = none)
    pub cone_sb: [f32; 2],
    /// +48: the penumbra, degrees (added to the cone angle: the fully lit inner cone)
    pub penumbra: f32,
}

impl CityLight {
    /// Byte +59: zero = the lamp is off (the game's grid builder and table writer take its
    /// intensity as 0).
    pub fn on(&self) -> bool {
        self.kind & 0xff != 0
    }

    /// The cone as the night lamp table holds it, `sat(cos x scale + bias)` of the angle to the
    /// axis: spots (type byte +58 = 1) scale = 1 / (cos(inner / 2) - cos(outer / 2)), bias =
    /// -cos(outer / 2) x scale with outer = cone angle + max(penumbra, 0), inner = cone angle +
    /// min(penumbra, 0), a penumbra of 0 taken as 1 degree (the records' penumbra is positive - 130 + 30:
    /// fully lit to 130, out at 160; the prop light files' is negative); the others (0, 1).
    /// Checked against the 943 cone lamps of the running game's table.
    pub fn table_cone(&self) -> [f32; 2] {
        if self.kind >> 8 != 1 || self.cone >= 359.0 {
            return [0.0, 1.0];
        }
        let (outer, inner) = if self.penumbra >= 0.0 { (self.cone + self.penumbra.max(1.0), self.cone) } else { (self.cone, (self.cone + self.penumbra).max(1.0)) };
        let (co, ci) = ((outer.to_radians() * 0.5).cos(), (inner.to_radians() * 0.5).cos());
        let scale = 1.0 / (ci - co).max(1e-4);
        [scale, -co * scale]
    }
}

pub fn parse_lights(res: &Resource) -> Vec<CityLight> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let Some(arr) = rd.ptr(0x50) else { return out };
    let count = rd.u16(0x54) as usize;
    for i in 0..count {
        let o = arr + i * 0x60;
        if !rd.has(o, 0x60) {
            break;
        }
        let v = |k: usize| Vec3::new(rd.f32(o + k), rd.f32(o + k + 4), rd.f32(o + k + 8));
        let l = CityLight { pos: v(0), color: v(0x10), dir: v(0x20), range: rd.f32(o + 0x40), cone: rd.f32(o + 0x44), intensity: rd.f32(o + 0x50), kind: rd.u16(o + 0x58), cone_sb: [rd.f32(o + 0x4c), rd.f32(o + 0x50)], penumbra: rd.f32(o + 0x48) };
        if l.pos.is_finite() && l.color.is_finite() && l.range.is_finite() {
            out.push(l);
        }
    }
    out
}
