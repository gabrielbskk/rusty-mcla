//! The race markers: the green chevron the original stands over every computer racer in a race
//! (the `icon_racer` mesh, tinted by the `mcUIClass` tune of `tune/ui`). Decode:
//! `docs/reference/ui/racer_marker.md`; the functions are the icon manager's (vtable 0x82025BDC):
//! `sub_822272C8` (who gets one), `sub_82226BE0` (colour, fade), `sub_82226180` (matrix),
//! `sub_822256E0` / `sub_82226E40` / `sub_82225170` (queue, far first, drawn through
//! `sub_82723940` with the scene's depth: the GPS map icons' x-ray technique - `hudmap_gpu.rs`).

use glam::{Mat4, Vec3, Vec4};
use std::collections::HashMap;
use std::path::Path;

/// The `mcUIClass` block of `tune/ui` (`tune/ui/0x75b995ab`; fields registered by
/// `sub_82224BE8`, the icon manager holds them 20 bytes further on).
#[derive(Clone, Debug)]
pub struct MarkTune {
    /// HookmanIconOffsetScale: the marker's centre stands this many times the car's bound
    /// height above the car (1.9)
    pub offset_scale: f32,
    /// DepthTestRacerMarker: drawn against the scene's depth (dimmed behind it)
    pub depth_test: bool,
    /// RacerMarkerScale (1, 1.4, 1), applied along the world's axes
    pub scale: Vec3,
    /// RacerMarkerColor, PaybackRacerColor, PaybackBuddyColor
    pub color: [f32; 4],
    pub payback_racer: [f32; 4],
    pub payback_buddy: [f32; 4],
    /// RacerMarkerFadeInBegin / Span, FadeOutBegin / Span (m from the camera)
    pub fade_in: (f32, f32),
    pub fade_out: (f32, f32),
    /// The free-roam 3D icons (hookicons.rs): HookmanIconScale (1.4, the icon frame's scale),
    /// HookmanIconTranslatePivot ((0, 1.07, -0.2), along the frame's axes), IconFadeoutDist
    /// (400 m: no icon beyond)
    pub icon_scale: f32,
    pub icon_pivot: Vec3,
    pub icon_fadeout: f32,
}

impl Default for MarkTune {
    /// The values of the file (what the game ships).
    fn default() -> MarkTune {
        MarkTune {
            offset_scale: 1.9,
            depth_test: true,
            scale: Vec3::new(1.0, 1.4, 1.0),
            color: [0.529412, 0.784314, 0.0, 1.0],
            payback_racer: [0.529412, 0.784314, 0.0, 1.0],
            payback_buddy: [1.0, 0.0, 0.0, 1.0],
            fade_in: (20.0, 45.0),
            fade_out: (175.0, 40.0),
            icon_scale: 1.4,
            icon_pivot: Vec3::new(0.0, 1.07, -0.2),
            icon_fadeout: 400.0,
        }
    }
}

impl MarkTune {
    pub fn load(data_dir: &str) -> MarkTune {
        let mut m: HashMap<String, Vec<f32>> = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(Path::new(data_dir).join("tune/ui")) {
            for e in rd.flatten() {
                let Ok(t) = std::fs::read_to_string(e.path()) else { continue };
                if !t.contains("mcUIClass {") {
                    continue;
                }
                for l in t.lines() {
                    let mut tok = l.split_whitespace();
                    let Some(k) = tok.next() else { continue };
                    let v: Vec<f32> = tok.filter_map(|x| x.parse().ok()).collect();
                    if !v.is_empty() {
                        m.insert(k.to_string(), v);
                    }
                }
            }
        }
        let d = MarkTune::default();
        let f = |k: &str, def: f32| m.get(k).and_then(|v| v.first().copied()).unwrap_or(def);
        let c = |k: &str, def: [f32; 4]| m.get(k).filter(|v| v.len() >= 4).map(|v| [v[0], v[1], v[2], v[3]]).unwrap_or(def);
        MarkTune {
            offset_scale: f("HookmanIconOffsetScale", d.offset_scale),
            depth_test: f("DepthTestRacerMarker", 1.0) != 0.0,
            scale: m.get("RacerMarkerScale").filter(|v| v.len() >= 3).map_or(d.scale, |v| Vec3::new(v[0], v[1], v[2])),
            color: c("RacerMarkerColor", d.color),
            payback_racer: c("PaybackRacerColor", d.payback_racer),
            payback_buddy: c("PaybackBuddyColor", d.payback_buddy),
            fade_in: (f("RacerMarkerFadeInBegin", d.fade_in.0), f("RacerMarkerFadeInSpan", d.fade_in.1)),
            fade_out: (f("RacerMarkerFadeOutBegin", d.fade_out.0), f("RacerMarkerFadeOutSpan", d.fade_out.1)),
            icon_scale: f("HookmanIconScale", d.icon_scale),
            icon_pivot: m.get("HookmanIconTranslatePivot").filter(|v| v.len() >= 3).map_or(d.icon_pivot, |v| Vec3::new(v[0], v[1], v[2])),
            icon_fadeout: f("IconFadeoutDist", d.icon_fadeout),
        }
    }

    /// The marker's colour for a racer of icon type `icon` (`Racer_SetIconType`): 10 and 18 (the
    /// payback client) PaybackRacerColor, 13 (the payback chasers) PaybackBuddyColor, the rest
    /// RacerMarkerColor.
    pub fn colour(&self, icon: u8) -> [f32; 4] {
        match icon {
            10 | 18 => self.payback_racer,
            13 => self.payback_buddy,
            _ => self.color,
        }
    }

    /// The alpha factor at `d` metres from the camera, None = not drawn: nothing nearer than
    /// FadeInBegin, in over FadeInSpan, out from FadeOutBegin over FadeOutSpan, nothing beyond.
    /// Icon type 18 (the payback client being chased) is always drawn, whole.
    pub fn fade(&self, icon: u8, d: f32) -> Option<f32> {
        if icon == 18 {
            return Some(1.0);
        }
        let (ib, is) = self.fade_in;
        let (ob, os) = self.fade_out;
        // (the order of the original's tests, sub_82226BE0 0x82226CF8..0x82226D80)
        if ib <= d && d <= ib + is {
            Some((d - ib) / is)
        } else if ob <= d && d <= ob + os {
            Some(1.0 - (d - ob) / os)
        } else if ib + is < d && d < ob {
            Some(1.0)
        } else {
            None
        }
    }

    /// The marker's world matrix for a car standing at `car` (its matrix's translation) whose
    /// composite bound is `height` tall, seen from a camera at `cam` looking along `fwd`:
    /// the flat `icon_racer` quad turned upright (x about 90 degrees: z -> -y), faced to the
    /// camera (right = |fwd x Y|, up = |right x fwd|), scaled along the world's axes, centred
    /// `offset_scale x height` above the car. (RacerMarkerTranslate goes into the translation row,
    /// which the position then overwrites - it has no effect.)
    pub fn matrix(&self, fwd: Vec3, car: Vec3, height: f32) -> Mat4 {
        let f = fwd.normalize_or_zero();
        let r = f.cross(Vec3::Y).normalize_or_zero();
        let u = r.cross(f).normalize_or_zero();
        let s = self.scale;
        // mesh (x, y, z) -> x r + (-z) u + y f, then the world's axes scaled
        let cx = r * s;
        let cy = f * s;
        let cz = -u * s;
        let p = Vec3::new(car.x, car.y + height * self.offset_scale, car.z);
        Mat4::from_cols(cx.extend(0.0), cy.extend(0.0), cz.extend(0.0), Vec4::new(p.x, p.y, p.z, 1.0))
    }
}

/// One marker to draw this frame: world matrix, colour (alpha faded), distance from the camera.
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub m: Mat4,
    pub colour: [f32; 4],
    pub dist: f32,
}

/// A racer the markers consider: where its car stands, its bound's height, its icon type.
pub struct Racer {
    pub pos: Vec3,
    pub height: f32,
    pub icon: u8,
}

/// How far below its tyre's crown the lowest wheel reaches in the car's composite bound as the
/// running game holds it. Read live from four cars: the wheel parts themselves are boxes of
/// +-crown radius about the hub, yet the composite's min y = hub - crown - 0.500 on the S281 and
/// the R8 (to 0.1 mm), 0.524 on the Murcielago and 0.508 on the Ducati 999R - as if the wheels
/// hung about half a metre down when the box was made (it never changes afterwards).
/// *(approx.: where this comes from was not found; 0.5 leaves those two 2.4 / 1.1 cm short,
/// 4.6 / 2.1 cm in the marker's height)*.
pub const WHEEL_BOX_DROP: f32 = 0.5;

/// The height of the car's composite bound as the running game has it (`sub_8255B9A8` +16 y
/// minus +32 y), a constant of the car from its set-up on (100 live samples, at rest and racing):
/// the top is the body hull's own box (+0x10 y, margin in - S281 1.4117 = its hull part's 1.412),
/// the bottom the wheel parts' - each at the model's hub (the file's part matrix), its tyre's
/// crown radius (`vehsim::crown` of the fitted TireRadius / TireWidth, front = -z) and
/// `WHEEL_BOX_DROP` below.
pub fn bound_height(c: &crate::vehicles::CarInstance) -> f32 {
    let Some(h) = c.model.params.hull.as_ref() else { return c.model.max.y - c.model.min.y };
    let top = h.bbox.map_or(c.model.max.y, |(hi, _)| hi.y);
    let tyres = match (c.setup.as_ref(), c.model.simtunes.as_ref()) {
        (Some(f), _) => Some((f.radius, f.width)),
        (None, Some(s)) => Some(([s.base.wheel_front.radius, s.base.wheel_back.radius], [s.base.wheel_front.width, s.base.wheel_back.width])),
        _ => None,
    };
    let Some((r, w)) = tyres.filter(|_| !h.wheel_parts.is_empty()) else { return top - c.model.min.y };
    let bottom = box_bottom(&h.wheel_parts, r, w);
    if std::env::var("MCLA_MARK_LOG").is_ok() {
        log::info!("race marker bound {}: top {:.4} bottom {:.4} h {:.4} tyres {:?} {:?}", c.model.name, top, bottom, top - bottom, r, w);
    }
    top - bottom
}

/// The lowest wheel part's box bottom: hub - crown radius (tyre radius `r`, width `w`; front
/// = index 0, the parts at -z) - `WHEEL_BOX_DROP`.
pub fn box_bottom(wheel_parts: &[Vec3], r: [f32; 2], w: [f32; 2]) -> f32 {
    wheel_parts.iter().map(|p| {
        let a = if p.z < 0.0 { 0 } else { 1 };
        p.y - crate::physics::vehsim::crown(r[a], w[a], 0.0)
    }).fold(f32::MAX, f32::min) - WHEEL_BOX_DROP
}

/// This frame's markers, far first (`sub_82225138` sorts the queue by distance, descending).
pub fn marks(t: &MarkTune, cam: Vec3, fwd: Vec3, racers: &[Racer]) -> Vec<Mark> {
    let mut out: Vec<Mark> = racers
        .iter()
        .filter_map(|r| {
            let d = cam.distance(r.pos);
            let a = t.fade(r.icon, d)?;
            let mut colour = t.colour(r.icon);
            colour[3] *= a;
            Some(Mark { m: t.matrix(fwd, r.pos, r.height), colour, dist: d })
        })
        .collect();
    out.sort_by(|a, b| b.dist.partial_cmp(&a.dist).unwrap_or(std::cmp::Ordering::Equal));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fade_rule() {
        let t = MarkTune::default();
        assert_eq!(t.fade(1, 10.0), None);
        assert_eq!(t.fade(1, 20.0), Some(0.0));
        assert!((t.fade(1, 42.5).unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(t.fade(1, 100.0), Some(1.0));
        assert!((t.fade(1, 195.0).unwrap() - 0.5).abs() < 1e-6);
        assert_eq!(t.fade(1, 216.0), None);
        assert_eq!(t.fade(18, 500.0), Some(1.0));
        assert_eq!(t.colour(13), [1.0, 0.0, 0.0, 1.0]);
    }

    /// The composite box heights read out of the running original (`work/career2/mk/rec`,
    /// marks.py): S281, R8, Murcielago, Ducati 999R.
    #[test]
    fn live_box_heights() {
        let root = std::env::var("MCLA_DATA_DIR").unwrap_or_else(|_| "D:/MCLA/extracted/cache".into());
        let dir = std::path::Path::new(&root).join("resources/vehicle");
        if !dir.exists() {
            return;
        }
        let assets = crate::assets::Assets::new(&root);
        for (name, top_live, bottom_live) in [("vp_sln_s281_06", 1.4117f32, -0.5112f32), ("vp_aud_r8_08", 1.2772, -0.4759), ("vp_lam_murcielago_06", 1.0885, -0.5459), ("vp_duc_999r_06", 1.1482, -0.5071)] {
            let car = crate::vehicle_model::load_car(&assets, &dir.join(name)).unwrap();
            let h = car.hull.as_ref().unwrap();
            let s = &car.simtune.as_ref().unwrap().base;
            let wheels: Vec<Vec3> = h.wheel_parts.iter().map(|p| Vec3::from(*p)).collect();
            let bottom = box_bottom(&wheels, [s.wheel_front.radius, s.wheel_back.radius], [s.wheel_front.width, s.wheel_back.width]);
            println!("{name}: top {:.4} ({top_live}) bottom {:.4} ({bottom_live}) tyres {} {} / {} {}", h.bbox.0[1], bottom, s.wheel_front.radius, s.wheel_front.width, s.wheel_back.radius, s.wheel_back.width);
            assert!((h.bbox.0[1] - top_live).abs() < 5e-4, "{name} top");
            // (S281 and R8 to 0.1 mm; the Murcielago's box reaches 2.4 cm and the Ducati's 1.1 cm
            // lower than the 0.5 rule - WHEEL_BOX_DROP)
            assert!((bottom - bottom_live).abs() < 0.03, "{name} bottom {bottom}");
        }
    }

    #[test]
    fn matrix_faces_the_camera() {
        let t = MarkTune::default();
        // camera looking down -z: the quad spans x and y, 1.4 times taller than wide
        let m = t.matrix(Vec3::new(0.0, 0.0, -1.0), Vec3::new(5.0, 1.0, -20.0), 2.0);
        let a = m.transform_point3(Vec3::new(1.0, 0.0, -1.0));
        let b = m.transform_point3(Vec3::new(-1.0, 0.0, 1.0));
        assert!((a - Vec3::new(6.0, 1.0 + 3.8 + 1.4, -20.0)).length() < 1e-5, "{a:?}");
        assert!((b - Vec3::new(4.0, 1.0 + 3.8 - 1.4, -20.0)).length() < 1e-5, "{b:?}");
    }
}
