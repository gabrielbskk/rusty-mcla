//! Occlusion culling from the original's occluder file (`city/sc/city.occluder` of the cache).
//!
//! The file lists 5614 closed polyhedra ("Type box": mostly 8-vertex boxes, some prisms with
//! more corners) that stand inside the city's buildings. The rule below is the original's, read
//! from the recompiled executable (work/map/lod_rule.md, "Occlusion culling in the port"):
//!
//! * per view, the occluders whose bounding-sphere centre is within 40 m of the camera in the
//!   ground plane are candidates (none at all when the camera is above y = 1000); at most 64
//!   volumes are kept per view;
//! * a candidate that is outside the view frustum is skipped;
//! * its volume is a set of at most 16 planes: the planes of the polygons that face the camera
//!   (`n . (eye - v) >= 1e-6`) and, for every edge between a facing and a non-facing polygon,
//!   the plane through the camera and that edge. No facing polygon (camera inside) or more
//!   than 16 planes: the occluder is not used;
//! * something is hidden when it lies behind every plane of one volume.
//!
//! What is the port's own: the candidates are taken nearest first (the original walks the
//! result of a spatial query), a non-convex or open polyhedron is dropped at load (the plane
//! set is only a correct occlusion volume for a convex body), an axis-aligned box is tested by
//! its farthest corner per plane (the original tests spheres and boxes the same way), and the
//! test keeps a 2 cm margin. The original's two savings - leaving out silhouette edges that are
//! outside the frustum and dropping occluders hidden by other occluders - are not done; both
//! only reduce work.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use glam::{DMat3, DVec3, Mat4, Vec3, Vec4};

use crate::render::Frustum;

/// Ground-plane distance from the camera to an occluder's sphere centre within which it is used
/// (0x82007A48 = 40.0, times the scale 0x827D2958 = 1.0).
pub(crate) const RANGE: f32 = 40.0;
/// No occluders above this camera height (0x82002C64).
const MAX_CAM_Y: f32 = 1000.0;
/// Volumes per view (the view's list holds 64).
const MAX_VOLUMES: usize = 64;
/// Planes per volume.
const MAX_PLANES: usize = 16;
/// A polygon faces the camera when the camera is at least this far in front of its plane.
const FACING_EPS: f32 = 1.0e-6;
/// Safety margin of the hidden test, metres (the port's own).
const MARGIN: f32 = 0.02;

struct Face {
    /// outward normal and offset: `n . p + d > 0` in front of the face
    plane: Vec4,
}

struct Occluder {
    centre: Vec3,
    min: Vec3,
    max: Vec3,
    /// mean of the vertices, strictly inside the body
    inside: Vec3,
    verts: Vec<Vec3>,
    faces: Vec<Face>,
    /// (vertex a, vertex b, face, face)
    edges: Vec<(u16, u16, u16, u16)>,
}

/// The occluders of the city, loaded once.
pub struct Occluders {
    list: Vec<Occluder>,
    /// polyhedra in the file, including the ones dropped as not convex / not closed
    pub in_file: usize,
}

/// The occlusion volumes of one view (one frame, one camera).
pub struct Selected {
    volumes: Vec<Vec<Vec4>>,
}

struct Tokens<'a> {
    it: std::str::SplitAsciiWhitespace<'a>,
}

impl<'a> Tokens<'a> {
    fn next(&mut self) -> Result<&'a str, String> {
        self.it.next().ok_or_else(|| "unexpected end of file".to_string())
    }
    fn expect(&mut self, what: &str) -> Result<(), String> {
        let t = self.next()?;
        if t == what { Ok(()) } else { Err(format!("expected `{what}`, found `{t}`")) }
    }
    fn num<T: std::str::FromStr>(&mut self) -> Result<T, String> {
        let t = self.next()?;
        t.parse().map_err(|_| format!("bad number `{t}`"))
    }
    /// `<key> <count> {`
    fn counted(&mut self, key: &str) -> Result<usize, String> {
        self.expect(key)?;
        let n = self.num()?;
        self.expect("{")?;
        Ok(n)
    }
}

impl Occluders {
    /// Parse the text of an occluder file.
    ///
    /// ```text
    /// version: 4
    /// Occluders <n>
    /// {
    ///     Occluder <i>
    ///     {
    ///         Type box
    ///         Name <name>
    ///         BoundingSphere <x> <y> <z> <r>
    ///         Vertices <nv> { <x> <y> <z> ... }
    ///         Edges <ne> { <a> <b> ... }
    ///         Polygons <np> { Vertices <k> { <vertex> ... } Edges <k> { <edge> ... } ... }
    ///     }
    ///     ...
    /// }
    /// ```
    pub fn parse(text: &str) -> Result<Occluders, String> {
        let mut t = Tokens { it: text.split_ascii_whitespace() };
        t.expect("version:")?;
        let version: u32 = t.num()?;
        if version != 4 {
            return Err(format!("occluder file version {version}, expected 4"));
        }
        let count = t.counted("Occluders")?;
        let mut list = Vec::with_capacity(count);
        for _ in 0..count {
            t.expect("Occluder")?;
            let _index: u32 = t.num()?;
            t.expect("{")?;
            t.expect("Type")?;
            let kind = t.next()?;
            t.expect("Name")?;
            let _name = t.next()?;
            t.expect("BoundingSphere")?;
            let centre = Vec3::new(t.num()?, t.num()?, t.num()?);
            let _radius: f32 = t.num()?;
            let nv = t.counted("Vertices")?;
            let mut verts = Vec::with_capacity(nv);
            for _ in 0..nv {
                verts.push(Vec3::new(t.num()?, t.num()?, t.num()?));
            }
            t.expect("}")?;
            let ne = t.counted("Edges")?;
            for _ in 0..ne * 2 {
                let _: u32 = t.num()?;
            }
            t.expect("}")?;
            let np = t.counted("Polygons")?;
            let mut polys: Vec<Vec<u16>> = Vec::with_capacity(np);
            for _ in 0..np {
                let k = t.counted("Vertices")?;
                let mut p = Vec::with_capacity(k);
                for _ in 0..k {
                    let v: u16 = t.num()?;
                    if v as usize >= nv {
                        return Err(format!("polygon vertex {v} out of range"));
                    }
                    p.push(v);
                }
                t.expect("}")?;
                let k = t.counted("Edges")?;
                for _ in 0..k {
                    let _: u32 = t.num()?;
                }
                t.expect("}")?;
                polys.push(p);
            }
            t.expect("}")?;
            t.expect("}")?;
            if kind != "box" {
                continue;
            }
            if let Some(o) = Occluder::build(centre, verts, &polys) {
                list.push(o);
            }
        }
        t.expect("}")?;
        Ok(Occluders { list, in_file: count })
    }

    pub fn load(data_dir: &str) -> Option<Occluders> {
        let path = std::path::Path::new(data_dir).join("city/sc/city.occluder");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("occluders: {}: {e}", path.display());
                return None;
            }
        };
        match Occluders::parse(&text) {
            Ok(o) => {
                log::info!("occluders: {} of {} usable (convex, closed)", o.list.len(), o.in_file);
                Some(o)
            }
            Err(e) => {
                log::warn!("occluders: {}: {e}", path.display());
                None
            }
        }
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    /// The occlusion volumes for the camera of `view_proj` (a perspective view).
    pub fn select(&self, view_proj: Mat4, frustum: &Frustum) -> Selected {
        match eye_of(view_proj) {
            Some(eye) => self.select_at(eye, frustum),
            None => Selected::none(),
        }
    }

    /// The occlusion volumes for a camera at `eye`.
    pub fn select_at(&self, eye: Vec3, frustum: &Frustum) -> Selected {
        let mut volumes = Vec::new();
        if !(eye.y <= MAX_CAM_Y) || !eye.is_finite() {
            return Selected { volumes };
        }
        let mut near: Vec<(f32, &Occluder)> = Vec::new();
        for o in &self.list {
            let dx = o.centre.x - eye.x;
            let dz = o.centre.z - eye.z;
            let d2 = dx * dx + dz * dz;
            if d2 <= RANGE * RANGE {
                near.push((d2, o));
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, o) in near {
            if volumes.len() >= MAX_VOLUMES {
                break;
            }
            if !frustum.intersects_aabb(o.min, o.max) {
                continue;
            }
            if let Some(v) = o.volume(eye) {
                volumes.push(v);
            }
        }
        Selected { volumes }
    }

    /// The volumes of a view without a frustum (the pedestrian spawner's view 2, built in
    /// sub_822C1FA8 by sub_825F2E10): occluders whose centre is within `range` in the ground
    /// plane, at most 64 volumes; nearest first (the original's query order is not decoded).
    pub fn select_around(&self, eye: Vec3, range: f32) -> Selected {
        let mut volumes = Vec::new();
        if !(eye.y <= MAX_CAM_Y) || !eye.is_finite() {
            return Selected { volumes };
        }
        let mut near: Vec<(f32, &Occluder)> = self.list.iter().map(|o| ((o.centre.x - eye.x).powi(2) + (o.centre.z - eye.z).powi(2), o)).filter(|n| n.0 <= range * range).collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, o) in near {
            if volumes.len() >= MAX_VOLUMES {
                break;
            }
            if let Some(v) = o.volume(eye) {
                volumes.push(v);
            }
        }
        Selected { volumes }
    }
}

impl Occluder {
    /// Face planes and edges of a closed convex polyhedron; None when it is not one.
    fn build(centre: Vec3, verts: Vec<Vec3>, polys: &[Vec<u16>]) -> Option<Occluder> {
        if verts.len() < 4 || polys.len() < 4 || polys.len() > u16::MAX as usize {
            return None;
        }
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let mut sum = DVec3::ZERO;
        for v in &verts {
            if !v.is_finite() {
                return None;
            }
            min = min.min(*v);
            max = max.max(*v);
            sum += v.as_dvec3();
        }
        let inside = (sum / verts.len() as f64).as_vec3();
        let size = (max - min).length();
        let tol = (size * 1.0e-3).max(1.0e-3);
        let mut faces = Vec::with_capacity(polys.len());
        let mut edge_faces: std::collections::BTreeMap<(u16, u16), Vec<u16>> = Default::default();
        for (fi, p) in polys.iter().enumerate() {
            if p.len() < 3 {
                return None;
            }
            // Newell normal, about the polygon's own mean
            let mean = p.iter().map(|&i| verts[i as usize].as_dvec3()).sum::<DVec3>() / p.len() as f64;
            let mut n = DVec3::ZERO;
            for k in 0..p.len() {
                let a = verts[p[k] as usize].as_dvec3() - mean;
                let b = verts[p[(k + 1) % p.len()] as usize].as_dvec3() - mean;
                n += a.cross(b);
                let (i, j) = (p[k], p[(k + 1) % p.len()]);
                if i == j {
                    return None;
                }
                edge_faces.entry((i.min(j), i.max(j))).or_default().push(fi as u16);
            }
            if n.length() < 1.0e-9 {
                return None;
            }
            let mut n = n.normalize();
            if n.dot(inside.as_dvec3() - mean) > 0.0 {
                n = -n;
            }
            let d = -n.dot(mean);
            // convex: every vertex on or behind every face; the face's own vertices on it
            for (vi, v) in verts.iter().enumerate() {
                let s = (n.dot(v.as_dvec3()) + d) as f32;
                if s > tol || (p.contains(&(vi as u16)) && s < -tol) {
                    return None;
                }
            }
            if (n.dot(inside.as_dvec3()) + d) as f32 > -tol {
                return None;
            }
            faces.push(Face { plane: Vec4::new(n.x as f32, n.y as f32, n.z as f32, d as f32) });
        }
        // closed: every edge between exactly two faces
        let mut edges = Vec::with_capacity(edge_faces.len());
        for ((a, b), f) in edge_faces {
            if f.len() != 2 || f[0] == f[1] {
                return None;
            }
            edges.push((a, b, f[0], f[1]));
        }
        Some(Occluder { centre, min, max, inside, verts, faces, edges })
    }

    /// The planes behind which nothing can be seen from `eye` (`n . p + d <= 0` behind).
    fn volume(&self, eye: Vec3) -> Option<Vec<Vec4>> {
        let mut facing = [false; 64];
        if self.faces.len() > facing.len() {
            return None;
        }
        let mut planes = Vec::with_capacity(MAX_PLANES);
        for (i, f) in self.faces.iter().enumerate() {
            if f.plane.truncate().dot(eye) + f.plane.w >= FACING_EPS {
                facing[i] = true;
                planes.push(f.plane);
            }
        }
        if planes.is_empty() {
            return None;
        }
        let e = eye.as_dvec3();
        let inside = self.inside.as_dvec3();
        for &(a, b, f0, f1) in &self.edges {
            if facing[f0 as usize] == facing[f1 as usize] {
                continue;
            }
            if planes.len() >= MAX_PLANES {
                return None;
            }
            let a = self.verts[a as usize].as_dvec3();
            let b = self.verts[b as usize].as_dvec3();
            let n = (a - e).cross(b - e);
            let l = n.length();
            if l < 1.0e-6 * (a - e).length() * (b - e).length() || l == 0.0 {
                return None;
            }
            let mut n = n / l;
            let s = n.dot(inside - e);
            if s.abs() < 1.0e-4 {
                return None;
            }
            if s > 0.0 {
                n = -n;
            }
            planes.push(Vec4::new(n.x as f32, n.y as f32, n.z as f32, -(n.dot(e)) as f32));
        }
        Some(planes)
    }
}

impl Selected {
    pub fn none() -> Selected {
        Selected { volumes: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.volumes.len()
    }

    /// True when the box is entirely behind one occluder (never true for something visible).
    pub fn hides_aabb(&self, mn: Vec3, mx: Vec3) -> bool {
        'volume: for planes in &self.volumes {
            for p in planes {
                let v = Vec3::new(
                    if p.x >= 0.0 { mx.x } else { mn.x },
                    if p.y >= 0.0 { mx.y } else { mn.y },
                    if p.z >= 0.0 { mx.z } else { mn.z },
                );
                if !(p.truncate().dot(v) + p.w <= -MARGIN) {
                    continue 'volume;
                }
            }
            return true;
        }
        false
    }

    /// True when a sphere lies `r` or more behind every plane of one volume (sub_82175238,
    /// occluder part).
    pub fn hides_sphere(&self, c: Vec3, r: f32) -> bool {
        self.volumes.iter().any(|planes| planes.iter().all(|p| p.truncate().dot(c) + p.w <= -r))
    }

    /// True when all the boxes together are behind one occluder: the test for a sector, which
    /// the original hides or shows as a whole (its parts may be set back behind a wall's face,
    /// like shop windows, and must not be tested one by one).
    pub fn hides_all(&self, boxes: impl Iterator<Item = (Vec3, Vec3)>) -> bool {
        if self.volumes.is_empty() {
            return false;
        }
        let mut mn = Vec3::splat(f32::INFINITY);
        let mut mx = Vec3::splat(f32::NEG_INFINITY);
        for (a, b) in boxes {
            mn = mn.min(a);
            mx = mx.max(b);
        }
        mn.cmple(mx).all() && self.hides_aabb(mn, mx)
    }
}

/// Centre of projection of a perspective view-projection matrix: the point whose clip x, y and w
/// are all zero.
fn eye_of(m: Mat4) -> Option<Vec3> {
    let (r0, r1, r3) = (m.row(0).as_dvec4(), m.row(1).as_dvec4(), m.row(3).as_dvec4());
    let a = DMat3::from_cols(r0.truncate(), r1.truncate(), r3.truncate()).transpose();
    let det = a.determinant();
    let scale = r0.truncate().length() * r1.truncate().length() * r3.truncate().length();
    if !(det.abs() > 1.0e-9 * scale) {
        return None;
    }
    let e = a.inverse() * -DVec3::new(r0.w, r1.w, r3.w);
    e.is_finite().then(|| e.as_vec3())
}

/// The city's occluders, loaded on first use; None when the file is missing or
/// `MCLA_NO_OCCLUDE` is set.
pub fn city(data_dir: &str) -> Option<&'static Occluders> {
    static CITY: OnceLock<Option<Occluders>> = OnceLock::new();
    CITY.get_or_init(|| {
        if std::env::var("MCLA_NO_OCCLUDE").is_ok() {
            log::info!("occluders: off (MCLA_NO_OCCLUDE)");
            return None;
        }
        Occluders::load(data_dir)
    })
    .as_ref()
}

/// The volumes of this frame's main view, from the city's file (loaded on first use).
/// Empty when the file is missing or `MCLA_NO_OCCLUDE` is set.
pub fn frame(data_dir: &str, view_proj: Mat4, frustum: &Frustum) -> Selected {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let Some(city) = city(data_dir) else { return Selected::none() };
    let s = city.select(view_proj, frustum);
    if CALLS.fetch_add(1, Ordering::Relaxed) % 120 == 60 {
        log::info!("occluders: {} volumes in view", s.len());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_text(boxes: &[(Vec3, Vec3)]) -> String {
        let mut s = format!("version: 4\nOccluders {}\n{{\n", boxes.len());
        for (i, (a, b)) in boxes.iter().enumerate() {
            let c = (*a + *b) * 0.5;
            let r = (*b - *a).length() * 0.5;
            s += &format!("\tOccluder {i} \n\t{{\n\t\tType box\n\t\tName test{i}\n\t\tBoundingSphere {}\t{}\t{}\t{} \n", c.x, c.y, c.z, r);
            s += "\t\tVertices 8\n\t\t{\n";
            // the file's own corner order for a box
            for (x, y, z) in [(0, 0, 0), (1, 0, 0), (0, 1, 0), (1, 1, 0), (0, 1, 1), (1, 1, 1), (0, 0, 1), (1, 0, 1)] {
                let p = Vec3::new(if x == 0 { a.x } else { b.x }, if y == 0 { a.y } else { b.y }, if z == 0 { a.z } else { b.z });
                s += &format!("\t\t\t{}\t{}\t{} \n", p.x, p.y, p.z);
            }
            s += "\t\t}\n\t\tEdges 12\n\t\t{\n\t\t\t0 1\n\t\t\t2 3\n\t\t\t4 5\n\t\t\t6 7\n\t\t\t0 2\n\t\t\t1 3\n\t\t\t2 4\n\t\t\t3 5\n\t\t\t4 6\n\t\t\t5 7\n\t\t\t6 0\n\t\t\t7 1\n\t\t}\n";
            s += "\t\tPolygons 6\n\t\t{\n";
            for (v, e) in [("2 3 1 0", "1 5 0 4"), ("4 5 3 2", "2 7 1 6"), ("6 7 5 4", "3 9 2 8"), ("0 1 7 6", "0 11 3 10"), ("3 5 7 1", "7 9 11 5"), ("4 2 0 6", "6 4 10 8")] {
                s += &format!("\t\t\tVertices 4\n\t\t\t{{\n\t\t\t\t{v} \n\t\t\t}}\n\t\t\tEdges 4\n\t\t\t{{\n\t\t\t\t{e} \n\t\t\t}}\n");
            }
            s += "\t\t}\n\t}\n";
        }
        s += "}\n";
        s
    }

    fn view(eye: Vec3, at: Vec3) -> (Mat4, Frustum) {
        let vp = Mat4::perspective_rh(1.0, 16.0 / 9.0, 0.1, 2000.0) * Mat4::look_at_rh(eye, at, Vec3::Y);
        (vp, Frustum::from_view_proj(vp))
    }

    /// a wall 2 m thick, 10 m high, 10 m wide, 10 m in front of the camera along +x
    const WALL: (Vec3, Vec3) = (Vec3::new(10.0, 0.0, -5.0), Vec3::new(12.0, 10.0, 5.0));
    const EYE: Vec3 = Vec3::new(0.0, 2.0, 0.0);

    fn wall_view(eye: Vec3) -> Selected {
        let o = Occluders::parse(&box_text(&[WALL])).unwrap();
        assert_eq!(o.len(), 1);
        let (vp, f) = view(eye, eye + Vec3::X);
        o.select(vp, &f)
    }

    #[test]
    fn eye_from_matrix() {
        let eye = Vec3::new(712.5, 19.25, -803.0);
        let (vp, _) = view(eye, eye + Vec3::new(0.3, -0.1, 0.9));
        assert!(eye_of(vp).unwrap().distance(eye) < 1.0e-3);
        assert!(eye_of(Mat4::orthographic_rh(-1.0, 1.0, -1.0, 1.0, 0.1, 10.0)).is_none());
    }

    #[test]
    fn behind_is_hidden() {
        let s = wall_view(EYE);
        assert_eq!(s.len(), 1);
        assert!(s.hides_aabb(Vec3::new(20.0, 1.0, -1.0), Vec3::new(22.0, 3.0, 1.0)));
        // inside the wall and behind it
        assert!(s.hides_aabb(Vec3::new(11.0, 1.0, -1.0), Vec3::new(20.0, 3.0, 1.0)));
        // far behind, wide, still inside the shadow
        assert!(s.hides_aabb(Vec3::new(100.0, 0.5, -40.0), Vec3::new(110.0, 30.0, 40.0)));
    }

    #[test]
    fn sticking_out_is_not_hidden() {
        let s = wall_view(EYE);
        // sideways, both ways
        assert!(!s.hides_aabb(Vec3::new(20.0, 1.0, 0.0), Vec3::new(22.0, 3.0, 30.0)));
        assert!(!s.hides_aabb(Vec3::new(20.0, 1.0, -30.0), Vec3::new(22.0, 3.0, 0.0)));
        // above
        assert!(!s.hides_aabb(Vec3::new(20.0, 1.0, -1.0), Vec3::new(22.0, 40.0, 1.0)));
        // below the wall's foot (the wall stands on y = 0, the camera is at y = 2)
        assert!(!s.hides_aabb(Vec3::new(20.0, -6.0, -1.0), Vec3::new(22.0, 3.0, 1.0)));
        // in front
        assert!(!s.hides_aabb(Vec3::new(5.0, 1.0, -1.0), Vec3::new(6.0, 3.0, 1.0)));
        // straddling the front face
        assert!(!s.hides_aabb(Vec3::new(9.0, 1.0, -1.0), Vec3::new(11.0, 3.0, 1.0)));
        // straddling the silhouette at the wall's edge
        assert!(!s.hides_aabb(Vec3::new(13.0, 1.0, 4.0), Vec3::new(14.0, 3.0, 8.0)));
        // just touching the back of the front face from outside
        assert!(!s.hides_aabb(Vec3::new(9.99, 1.0, -1.0), Vec3::new(30.0, 3.0, 1.0)));
        // behind the camera
        assert!(!s.hides_aabb(Vec3::new(-22.0, 1.0, -1.0), Vec3::new(-20.0, 3.0, 1.0)));
        // a sector: hidden only as a whole
        let back = (Vec3::new(20.0, 1.0, -1.0), Vec3::new(22.0, 3.0, 1.0));
        let front = (Vec3::new(9.5, 1.0, -1.0), Vec3::new(9.9, 3.0, 1.0));
        assert!(s.hides_all([back, (Vec3::new(11.0, 1.0, 1.0), Vec3::new(12.5, 2.0, 2.0))].into_iter()));
        assert!(!s.hides_all([back, front].into_iter()));
        assert!(!s.hides_all(std::iter::empty()));
    }

    #[test]
    fn camera_inside_or_far_selects_nothing() {
        let s = wall_view(Vec3::new(11.0, 2.0, 0.0));
        assert_eq!(s.len(), 0);
        assert!(!s.hides_aabb(Vec3::new(20.0, 1.0, -1.0), Vec3::new(22.0, 3.0, 1.0)));
        // more than 40 m from the occluder's centre
        let s = wall_view(Vec3::new(-30.0, 2.0, 0.0));
        assert_eq!(s.len(), 0);
        // looking away: the occluder is outside the frustum
        let o = Occluders::parse(&box_text(&[WALL])).unwrap();
        let (vp, f) = view(EYE, EYE - Vec3::X);
        assert_eq!(o.select(vp, &f).len(), 0);
        // above the height limit
        let (_, f) = view(EYE, EYE + Vec3::X);
        assert_eq!(o.select_at(Vec3::new(0.0, 1500.0, 0.0), &f).len(), 0);
    }

    #[test]
    fn corner_view_uses_two_faces() {
        // seen over a corner: two facing sides, the shadow is still exact
        let eye = Vec3::new(0.0, 2.0, -12.0);
        let o = Occluders::parse(&box_text(&[WALL])).unwrap();
        let (vp, f) = view(eye, Vec3::new(11.0, 2.0, 0.0));
        let s = o.select(vp, &f);
        assert_eq!(s.len(), 1);
        assert_eq!(s.volumes[0].len(), 2 + 6);
        assert!(s.hides_aabb(Vec3::new(20.0, 1.0, 9.0), Vec3::new(21.0, 3.0, 10.0)));
        assert!(!s.hides_aabb(Vec3::new(20.0, 1.0, -9.0), Vec3::new(21.0, 3.0, -8.0)));
    }

    /// Does the segment eye -> p pass through the box?
    fn blocked(eye: Vec3, p: Vec3, b: (Vec3, Vec3)) -> bool {
        let d = p - eye;
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for k in 0..3 {
            if d[k].abs() < 1.0e-9 {
                if eye[k] < b.0[k] || eye[k] > b.1[k] {
                    return false;
                }
                continue;
            }
            let (a, c) = ((b.0[k] - eye[k]) / d[k], (b.1[k] - eye[k]) / d[k]);
            t0 = t0.max(a.min(c));
            t1 = t1.min(a.max(c));
        }
        t0 <= t1
    }

    #[test]
    fn hidden_boxes_are_really_hidden() {
        // random boxes against random cameras: whatever is reported hidden must have every
        // corner and many inner points behind the occluder as seen from the camera
        let mut seed = 0x1234_5678_u32;
        let mut rnd = move |lo: f32, hi: f32| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            lo + (hi - lo) * (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let o = Occluders::parse(&box_text(&[WALL])).unwrap();
        let (mut hidden, mut total) = (0, 0);
        for _ in 0..400 {
            let eye = Vec3::new(rnd(-20.0, 30.0), rnd(0.5, 25.0), rnd(-30.0, 30.0));
            let (vp, f) = view(eye, Vec3::new(11.0, 5.0, 0.0));
            let s = o.select(vp, &f);
            for _ in 0..200 {
                let c = Vec3::new(rnd(-40.0, 60.0), rnd(-5.0, 30.0), rnd(-50.0, 50.0));
                let h = Vec3::new(rnd(0.1, 6.0), rnd(0.1, 6.0), rnd(0.1, 6.0));
                let (mn, mx) = (c - h, c + h);
                total += 1;
                if !s.hides_aabb(mn, mx) {
                    continue;
                }
                hidden += 1;
                for i in 0..64 {
                    let p = if i < 8 {
                        Vec3::new(if i & 1 == 0 { mn.x } else { mx.x }, if i & 2 == 0 { mn.y } else { mx.y }, if i & 4 == 0 { mn.z } else { mx.z })
                    } else {
                        Vec3::new(rnd(mn.x, mx.x), rnd(mn.y, mx.y), rnd(mn.z, mx.z))
                    };
                    assert!(blocked(eye, p, WALL), "eye {eye} box {mn} {mx} point {p}");
                }
            }
        }
        assert!(hidden > 200, "only {hidden} of {total} hidden: the test does not exercise the rule");
    }

    #[test]
    fn not_convex_is_dropped() {
        // a box with one corner pulled inwards so far that two faces are no longer planar
        let text = box_text(&[WALL]).replace("12\t10\t5 ", "11\t5\t0 ");
        assert_ne!(text, box_text(&[WALL]));
        let o = Occluders::parse(&text).unwrap();
        assert_eq!((o.len(), o.in_file), (0, 1));
    }

    #[test]
    fn city_file() {
        let Ok(text) = std::fs::read_to_string("D:/MCLA/extracted/cache/city/sc/city.occluder") else { return };
        let o = Occluders::parse(&text).unwrap();
        assert_eq!(o.in_file, 5614);
        println!("city.occluder: {} of {} usable", o.len(), o.in_file);
        assert!(o.len() > 5000);
        let most = o.list.iter().map(|a| o.list.iter().filter(|b| (b.centre.x - a.centre.x).hypot(b.centre.z - a.centre.z) <= RANGE).count()).max().unwrap();
        println!("most occluder centres within {RANGE} m of one: {most}");
    }
}
