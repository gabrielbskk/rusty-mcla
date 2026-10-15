//! Roadside parking spots (`resources/city/sc/0x2e3038ce.r62`): root+0x18 -> array of
//! 64-byte records, u16 count at root+0x1c.
//!
//! ```text
//! +28 u8 kind (1 = kerbside, 0 = car park)   +30 vec3 position   +3c f32 heading
//! ```
//! The heading is in radians measured from +x towards +z, i.e. the parked car points
//! along `(cos h, 0, sin h)`.

use super::Rd;
use crate::rsc::Resource;
use glam::Vec3;

#[derive(Debug, Clone, Copy)]
pub struct ParkingSpot {
    pub pos: Vec3,
    pub heading: f32,
    pub kind: u8,
}

impl ParkingSpot {
    /// Direction the parked car faces.
    pub fn dir(&self) -> Vec3 {
        Vec3::new(self.heading.cos(), 0.0, self.heading.sin())
    }
}

pub fn parse_parking(res: &Resource) -> Vec<ParkingSpot> {
    let rd = Rd::new(res);
    let mut out = Vec::new();
    let Some(arr) = rd.ptr(0x18) else { return out };
    let count = rd.u16(0x1c) as usize;
    for i in 0..count {
        let o = arr + i * 64;
        if !rd.has(o, 64) {
            break;
        }
        let pos = Vec3::new(rd.f32(o + 0x30), rd.f32(o + 0x34), rd.f32(o + 0x38));
        let heading = rd.f32(o + 0x3c);
        if pos.is_finite() && heading.is_finite() {
            out.push(ParkingSpot { pos, heading, kind: rd.u8(o + 0x28) });
        }
    }
    out
}
