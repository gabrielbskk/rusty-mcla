//! AI driver: follows the directed road-link graph with a look-ahead pursuit controller.

use crate::physics::{CarInput, CarState};
use glam::Vec3;
use mcla_formats::rage::navsys::NavSys;

#[derive(Debug, Clone, Default)]
pub struct AiDriver {
    pub link: Option<usize>,
    pub t: f32,
    /// planned route: links after the current one
    pub route: Vec<usize>,
    pub target_speed: f32, // m/s
    pub aggression: f32,   // 0..1
    pub stuck_timer: f32,
    pub reverse_timer: f32,
    pub rng: u32,
    pub last_target: Vec3,
    /// fixed route (race): links to follow in order
    pub route_override: Option<Vec<usize>>,
    /// cursor into `route_override` (links may repeat across laps)
    pub route_pos: usize,
    /// race line: polyline to follow (takes precedence over the link route)
    pub path: Option<std::sync::Arc<Vec<Vec3>>>,
    /// index of the path segment we are on (segment i = path[i] -> path[i + 1])
    pub path_i: usize,
    /// time spent (nearly) stationary while trying to drive
    pub stuck_long: f32,
    /// position the car has to get 6 m away from to count as making progress
    pub stuck_anchor: Vec3,
    /// set when the car should be put back on the race line: (position, yaw)
    pub reset_to: Option<(Vec3, f32)>,
    /// racing: how far beside the line to drive just now (metres, + = right) to get past traffic
    pub dodge: f32,
    /// pursuit target (position, velocity): police cruisers
    pub chase: Option<(Vec3, Vec3)>,
    /// the car as its driver knows it: the game's calibration (speed for a bend, braking, steering)
    pub tune: Option<std::sync::Arc<mcla_formats::rage::aitune::AiTune>>,
    /// the speed asked for already allows for the bends ahead (set while following a race line)
    line_speed: bool,
    /// racing: seconds boxed in behind something standing
    pub blocked: f32,
    /// diagnostics: the speed last aimed for
    pub last_v_target: f32,
    pub chase_replan: f32,
    /// forward phase of a three-point turn: no new reverse before this runs out
    pub kturn_cool: f32,
}

impl AiDriver {
    pub fn new(seed: u32) -> AiDriver {
        AiDriver { target_speed: 20.0, aggression: 0.5, rng: seed.max(1), ..Default::default() }
    }

    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x % 10000) as f32 / 10000.0
    }

    pub fn locate(&mut self, nav: &NavSys, pos: Vec3, heading: Vec3) {
        if let Some((li, t)) = nav.nearest_link_aligned(pos, heading) {
            self.link = Some(li);
            self.t = t;
            self.route.clear();
        }
    }

    /// Extend the planned route so that it covers at least `dist` metres ahead.
    fn plan(&mut self, nav: &NavSys, dist: f32) {
        let Some(cur) = self.link else { return };
        if let Some(ro) = self.route_override.as_ref() {
            let start = self.route_pos.min(ro.len());
            let win_end = (start + 80).min(ro.len());
            if let Some(off) = ro[start..win_end].iter().position(|&l| l == cur) {
                let pos = start + off;
                self.route_pos = pos;
                self.route = ro[pos + 1..(pos + 1 + 60).min(ro.len())].to_vec();
                return;
            }
            // off the route: rejoin at the nearest upcoming route link
            let p = nav.links[cur].a;
            let mut best = None;
            let mut bd = f32::INFINITY;
            for (i, &l) in ro.iter().enumerate().take(win_end).skip(start) {
                let d = nav.links[l].a.distance(p);
                if d < bd {
                    bd = d;
                    best = Some(i);
                }
            }
            if let Some(i) = best {
                self.route = ro[i..(i + 60).min(ro.len())].to_vec();
                return;
            }
        }
        let mut acc = nav.links[cur].len * (1.0 - self.t);
        let mut last = cur;
        for &l in &self.route {
            acc += nav.links[l].len;
            last = l;
        }
        let mut guard = 0;
        while acc < dist && guard < 32 {
            guard += 1;
            let l = &nav.links[last];
            if l.next.is_empty() {
                break;
            }
            // prefer going straight, with some randomness
            let d = l.dir();
            let mut best = None;
            let mut best_score = f32::NEG_INFINITY;
            for &n in &l.next {
                if self.route.contains(&n) || n == cur {
                    continue;
                }
                let s = nav.links[n].dir().dot(d) + self.rand() * 0.9;
                if s > best_score {
                    best_score = s;
                    best = Some(n);
                }
            }
            let Some(n) = best else { break };
            acc += nav.links[n].len;
            self.route.push(n);
            last = n;
        }
    }

    fn advance(&mut self, nav: &NavSys, pos: Vec3) {
        let Some(cur) = self.link else { return };
        let l = &nav.links[cur];
        self.t = l.project(pos);
        // move to the next link when we pass the end of the current one
        if self.t >= 0.999 || (pos - l.b).dot(l.dir()) > 0.5 {
            if !self.route.is_empty() {
                let n = self.route.remove(0);
                self.link = Some(n);
                self.t = nav.links[n].project(pos);
            }
        }
    }

    /// Point `dist` metres ahead along current link + route.
    fn point_ahead(&self, nav: &NavSys, dist: f32) -> Option<Vec3> {
        let cur = self.link?;
        let l = &nav.links[cur];
        let mut remaining = dist;
        let left = l.len * (1.0 - self.t);
        if left >= remaining {
            return Some(l.point_at(self.t + remaining / l.len.max(0.01)));
        }
        remaining -= left;
        for &r in &self.route {
            let rl = &nav.links[r];
            if rl.len >= remaining {
                return Some(rl.point_at(remaining / rl.len.max(0.01)));
            }
            remaining -= rl.len;
        }
        self.route.last().map(|&r| nav.links[r].b).or(Some(l.b))
    }

    /// Point `dist` metres ahead of `pos` along the race polyline (advances the cursor).
    /// The fastest the car may go now so that every bend of the line ahead can still be
    /// reached at the speed the car's tables give for it (braking at a share of its limit).
    fn path_speed_limit(&self, pos: Vec3, speed: f32) -> Option<f32> {
        let t = self.tune.as_ref()?;
        let path = self.path.as_ref()?;
        if path.len() < 4 {
            return None;
        }
        let brake = t.max_brake_decel * 0.7;
        let reach = speed * speed / (2.0 * brake) + 40.0;
        // sample the line every few metres; a bend is the circle through three samples
        let step = 7.0f32;
        let mut pts: Vec<Vec3> = vec![pos];
        let mut k = self.path_i.min(path.len() - 2);
        let mut cur = {
            let (a, b) = (path[k], path[k + 1]);
            let ab = b - a;
            a + ab * ((pos - a).dot(ab) / ab.length_squared().max(0.01)).clamp(0.0, 1.0)
        };
        let mut left = step;
        let mut total = 0.0;
        while total < reach && k + 1 < path.len() {
            let to_next = path[k + 1] - cur;
            let l = to_next.length();
            if l < left {
                left -= l;
                total += l;
                cur = path[k + 1];
                k += 1;
            } else {
                cur += to_next / l.max(1e-4) * left;
                total += left;
                left = step;
                pts.push(cur);
            }
        }
        if pts.len() < 3 {
            return None;
        }
        let mut limit = f32::INFINITY;
        let mut d = 0.0;
        for w in pts.windows(3) {
            d += w[0].distance(w[1]);
            let (a, b, c) = (w[0], w[1], w[2]);
            let (ab, bc, ca) = (Vec3::new(b.x - a.x, 0.0, b.z - a.z), Vec3::new(c.x - b.x, 0.0, c.z - b.z), Vec3::new(a.x - c.x, 0.0, a.z - c.z));
            let area2 = ab.cross(bc).length();
            let den = ab.length() * bc.length() * ca.length();
            let kappa = if den > 1e-3 { 2.0 * area2 / den } else { 0.0 };
            let v_bend = t.speed_for_curvature(kappa) * 0.9;
            // (the first sample is the car itself: its own place is not a bend of the line)
            let dd = (d - step * 0.5).max(0.0);
            limit = limit.min((v_bend * v_bend + 2.0 * brake * dd).sqrt());
        }
        Some(limit)
    }

    pub fn path_ahead(&mut self, pos: Vec3, dist: f32) -> Option<Vec3> {
        let path = self.path.clone()?;
        if path.len() < 2 {
            return path.first().copied();
        }
        // advance while we are past the end of the current segment (or closer to a later one)
        let mut guard = 0;
        while self.path_i + 2 < path.len() && guard < 64 {
            guard += 1;
            let a = path[self.path_i];
            let b = path[self.path_i + 1];
            let ab = Vec3::new(b.x - a.x, 0.0, b.z - a.z);
            let l2 = ab.length_squared().max(0.01);
            let t = Vec3::new(pos.x - a.x, 0.0, pos.z - a.z).dot(ab) / l2;
            let near_end = Vec3::new(pos.x - b.x, 0.0, pos.z - b.z).length() < 9.0;
            if t >= 1.0 || near_end {
                self.path_i += 1;
            } else {
                break;
            }
        }
        let i = self.path_i.min(path.len() - 2);
        let a = path[i];
        let b = path[i + 1];
        let ab = b - a;
        let len = ab.length().max(0.01);
        let t = ((pos - a).dot(ab) / (len * len)).clamp(0.0, 1.0);
        let mut remaining = dist;
        let left = len * (1.0 - t);
        if left >= remaining {
            return Some(a + ab * (t + remaining / len));
        }
        remaining -= left;
        let mut k = i + 1;
        while k + 1 < path.len() {
            let seg = path[k + 1] - path[k];
            let sl = seg.length().max(0.01);
            if sl >= remaining {
                return Some(path[k] + seg * (remaining / sl));
            }
            remaining -= sl;
            k += 1;
        }
        path.last().copied()
    }

    pub fn drive(&mut self, nav: &NavSys, car: &CarState, dt: f32) -> CarInput {
        let pos = car.pos;
        let speed = car.fwd_speed();
        let look = (7.0 + speed * 0.9).clamp(7.0, 45.0);
        if let Some((tp, tv)) = self.chase {
            let dist = tp.distance(pos);
            self.chase_replan -= dt;
            let mut direct = dist < 55.0;
            if !direct && (self.chase_replan <= 0.0 || self.path.is_none()) {
                self.chase_replan = 1.5;
                match nav.route_points(pos, Some(car.forward()), tp) {
                    Some(pts) if pts.len() >= 2 => {
                        self.path = Some(std::sync::Arc::new(pts));
                        self.path_i = 0;
                    }
                    _ => {
                        self.path = None;
                        direct = true;
                    }
                }
            }
            if direct {
                self.path = None;
                self.stuck_long = 0.0;
                if dist < 6.5 && tv.length() < 3.0 {
                    // boxed the target in (or told to hold): stop
                    return CarInput { brake: 1.0, handbrake: speed.abs() < 2.0, ..Default::default() };
                }
                let lead = (dist / speed.abs().max(12.0)).min(1.2);
                let target = tp + tv * lead;
                let save = self.target_speed;
                if dist < 16.0 {
                    self.target_speed = (tv.length() + 4.0).min(save);
                }
                let inp = self.steer_towards(car, target, target, dt);
                self.target_speed = save;
                return inp;
            }
        }
        if self.path.is_some() {
            // hopelessly stuck (wedged against scenery): go back onto the race line,
            // like the original's reset-to-track
            if pos.distance(self.stuck_anchor) > 6.0 {
                self.stuck_anchor = pos;
                self.stuck_long = 0.0;
            } else {
                self.stuck_long += dt;
            }
            if self.stuck_long > 7.0 {
                self.stuck_long = 0.0;
                if let Some(path) = self.path.clone() {
                    let i = (self.path_i + 1).min(path.len().saturating_sub(2));
                    let a = path[i];
                    let b = path[(i + 1).min(path.len() - 1)];
                    let d = (b - a).normalize_or(car.forward());
                    self.path_i = i;
                    if path.len() > 20 && i + 3 < path.len() {
                        log::info!("ai: stuck at {:.0} {:.0} {:.0} (line point {} of {}), speed {:.1}", pos.x, pos.y, pos.z, i, path.len(), speed);
                    }
                    self.reset_to = Some((a + Vec3::Y * 0.9, (-d.x).atan2(-d.z)));
                    self.stuck_timer = 0.0;
                    self.reverse_timer = 0.0;
                }
            }
            // (aiming far ahead cuts the corners - into the lamp post on the kerb; the far
            // point only sets the speed)
            let near = (5.0 + speed * 0.5).clamp(6.0, 24.0);
            let Some(target) = self.path_ahead(pos, near) else { return CarInput { brake: 1.0, ..Default::default() } };
            let far = self.path_ahead(pos, look * 2.5).unwrap_or(target);
            // (pulled out to pass: aim beside the line)
            let side = car.right() * self.dodge;
            // (with the car's tables the whole line ahead sets the speed)
            let save = self.target_speed;
            if let Some(v) = self.path_speed_limit(pos, speed) {
                self.target_speed = save.min(v.max(7.0));
                self.line_speed = true;
            }
            let inp = self.steer_towards(car, target + side, far + side * 0.5, dt);
            self.target_speed = save;
            self.line_speed = false;
            return inp;
        }
        if self.link.is_none() {
            self.locate(nav, pos, car.forward());
        }
        self.advance(nav, pos);
        self.plan(nav, 120.0);
        // off the road? relocate
        if let Some(cur) = self.link {
            let l = &nav.links[cur];
            if l.point_at(self.t).distance(pos) > 25.0 {
                if let Some(ro) = self.route_override.clone() {
                    // snap to nearest route link
                    let mut best = None;
                    let mut bd = f32::INFINITY;
                    let start = self.route_pos.min(ro.len());
                    let end = (start + 80).min(ro.len());
                    let mut best_i = start;
                    for (i, &li) in ro.iter().enumerate().take(end).skip(start) {
                        let t = nav.links[li].project(pos);
                        let d = nav.links[li].point_at(t).distance(pos);
                        if d < bd {
                            bd = d;
                            best = Some((li, t));
                            best_i = i;
                        }
                    }
                    if let Some((li, t)) = best {
                        self.link = Some(li);
                        self.t = t;
                        self.route_pos = best_i;
                        self.route.clear();
                    }
                } else {
                    self.locate(nav, pos, car.forward());
                }
                self.plan(nav, 120.0);
            }
        }
        let Some(target) = self.point_ahead(nav, look) else {
            self.link = None;
            return CarInput { throttle: 0.3, ..Default::default() };
        };
        let far = self.point_ahead(nav, look * 2.5).unwrap_or(target);
        self.steer_towards(car, target, far, dt)
    }

    /// Steering / throttle towards `target`, slowing for the curvature seen at `far`.
    fn steer_towards(&mut self, car: &CarState, target: Vec3, far: Vec3, dt: f32) -> CarInput {
        let pos = car.pos;
        let speed = car.fwd_speed();
        self.last_target = target;
        let to = target - pos;
        let fwd = car.forward();
        let right = car.right();
        let dist = to.length().max(0.1);
        let angle = (right.dot(to) / dist).clamp(-1.0, 1.0).asin();
        let steer = (angle * 2.5).clamp(-1.0, 1.0);
        // slow down for upcoming curvature
        let to_far = far - pos;
        let curve = 1.0 - fwd.dot(to_far.normalize_or(fwd)).clamp(0.0, 1.0);
        let mut v_target = (self.target_speed * (1.0 - curve * 1.2 - angle.abs() * 0.8)).max(7.0);
        let mut input = CarInput::default();
        if car.veh.is_some() {
            // on the original's simulation the tyres hold about 2 g and the brakes 1.5:
            // the bend ahead (an arc through the far point) sets the speed it can be taken
            // at, the distance to it how much faster the car may still go
            let d = to_far.length().max(1.0);
            let sin = (1.0 - fwd.dot(to_far / d).clamp(-1.0, 1.0).powi(2)).max(0.0).sqrt();
            let kappa = (2.0 * sin / d).max(1e-4);
            // (with the car's own tables: the speed it holds that bend at, and its braking;
            // a margin is kept, as the tables are the car's limit)
            let (v_bend, brake) = match self.tune.as_ref() {
                Some(t) => (t.speed_for_curvature(kappa) * 0.88, t.max_brake_decel * 0.7),
                None => ((15.0f32 / kappa).sqrt(), 11.0),
            };
            let v_now = if self.line_speed { f32::INFINITY } else { (v_bend * v_bend + 2.0 * brake * d * 0.5).sqrt() };
            v_target = self.target_speed.min(v_now).max(7.0);
            if fwd.dot(to_far) < 0.0 {
                v_target = v_target.min(9.0);
            }
            if speed < v_target {
                input.throttle = ((v_target - speed) * 0.5).clamp(0.3, 1.0);
            } else {
                // a little too fast: lift (the engine brakes hard); more: brake
                input.brake = ((speed - v_target - 1.5) * 0.25).clamp(0.0, 1.0);
            }
        } else if speed < v_target {
            input.throttle = ((v_target - speed) * 0.5).clamp(0.25, 1.0);
        } else {
            input.brake = ((speed - v_target) * 0.35).clamp(0.0, 1.0);
        }
        self.last_v_target = v_target;
        input.steer = steer;
        if let (Some(t), true, true) = (self.tune.as_ref(), car.veh.is_some(), std::env::var("MCLA_AI_STEER_OLD").is_err()) {
            // the steering the tables give for the arc to the target (curvature x SteerScalar at
            // this speed), a little over to pull onto the line
            let k = 2.0 * right.dot(to) / (dist * dist);
            input.steer = (k * t.steer_scalar_at(speed) * 1.25).clamp(-1.0, 1.0);
        }
        self.kturn_cool = (self.kturn_cool - dt).max(0.0);
        if fwd.dot(to) < -0.2 * dist {
            input.throttle = if self.chase.is_some() { 0.8 } else { 0.5 };
            input.brake = 0.0;
            input.steer = if right.dot(to) > 0.0 { 1.0 } else { -1.0 };
            // three-point turn: back up with opposite lock, then go forward again
            if self.chase.is_some() && speed.abs() < 3.0 && self.reverse_timer <= 0.0 && self.kturn_cool <= 0.0 {
                self.reverse_timer = 1.5;
                self.kturn_cool = 4.0;
            }
        }
        if speed.abs() < 0.5 && input.throttle > 0.3 {
            self.stuck_timer += dt;
        } else {
            self.stuck_timer = 0.0;
        }
        if self.stuck_timer > 1.5 {
            self.reverse_timer = 1.2;
            self.stuck_timer = 0.0;
        }
        if self.reverse_timer > 0.0 {
            self.reverse_timer -= dt;
            input.throttle = 0.0;
            input.brake = 1.0;
            input.steer = -steer;
        }
        input
    }
}
