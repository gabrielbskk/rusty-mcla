//! Opponent rubber banding from the game's `mcRubberBandTuning` files
//! (`tune/career/*`, eleven difficulty steps from MaxThrottle 0.75 to 1.0).
//!
//! An opponent ahead of the player eases off between `MinDist` and `MaxDist`
//! (down to `MinThrottle`, never slower than `MinSpeed` of its top speed); one behind
//! pushes up to `BehindMaxThrottle`. `AheadDistOffsets` spread the field by position.

use crate::police::Tuning;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct RubberBand {
    pub max_throttle: f32,
    pub min_speed: f32,
    pub min_dist: f32,
    pub max_dist: f32,
    pub min_throttle: f32,
    pub behind_min_dist: f32,
    pub behind_max_dist: f32,
    pub behind_max_throttle: f32,
    pub home_stretch_behind_max_throttle: f32,
    pub home_stretch_distance: f32,
    pub player_close_max_throttle: f32,
    pub player_close_distance: f32,
    pub ahead_offsets: Vec<f32>,
}

impl Default for RubberBand {
    fn default() -> Self {
        RubberBand {
            max_throttle: 0.85,
            min_speed: 0.51,
            min_dist: 20.0,
            max_dist: 65.0,
            min_throttle: 0.29,
            behind_min_dist: 3.0,
            behind_max_dist: 16.0,
            behind_max_throttle: 0.95,
            home_stretch_behind_max_throttle: 0.78,
            home_stretch_distance: 225.0,
            player_close_max_throttle: 0.995,
            player_close_distance: 20.0,
            ahead_offsets: vec![20.0, 65.0, 110.0, 180.0, 260.0, 380.0, 505.0, 700.0],
        }
    }
}

fn float_array(text: &str, tag: &str) -> Vec<f32> {
    let Some(a) = text.find(&format!("<{tag} ")) else { return Vec::new() };
    let rest = &text[a..];
    let Some(s) = rest.find(">") else { return Vec::new() };
    let Some(e) = rest.find(&format!("</{tag}>")) else { return Vec::new() };
    rest[s + 1..e].split_whitespace().filter_map(|x| x.parse().ok()).collect()
}

/// All rubber-band tunings on the disc, easiest first.
pub fn load_all(data_dir: &str) -> Vec<RubberBand> {
    let dir = Path::new(data_dir).join("tune/career");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if !p.is_file() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            if !text.contains("<mcRubberBandTuning>") {
                continue;
            }
            let t = Tuning::load(&p);
            let d = RubberBand::default();
            let offs = float_array(&text, "AheadDistOffsets");
            out.push(RubberBand {
                max_throttle: t.get("MaxThrottle", d.max_throttle),
                min_speed: t.get("MinSpeed", d.min_speed),
                min_dist: t.get("MinDist", d.min_dist),
                max_dist: t.get("MaxDist", d.max_dist),
                min_throttle: t.get("MinThrottle", d.min_throttle),
                behind_min_dist: t.get("BehindMinDist", d.behind_min_dist),
                behind_max_dist: t.get("BehindMaxDist", d.behind_max_dist),
                behind_max_throttle: t.get("BehindMaxThrottle", d.behind_max_throttle),
                home_stretch_behind_max_throttle: t.get("HomeStretchBehindMaxThrottle", d.home_stretch_behind_max_throttle),
                home_stretch_distance: t.get("HomeStretchDistance", d.home_stretch_distance),
                player_close_max_throttle: t.get("PlayerCloseMaxThrottle", d.player_close_max_throttle),
                player_close_distance: t.get("PlayerCloseDistance", d.player_close_distance),
                ahead_offsets: if offs.is_empty() { d.ahead_offsets } else { offs },
            });
        }
    }
    out.sort_by(|a, b| a.max_throttle.partial_cmp(&b.max_throttle).unwrap_or(std::cmp::Ordering::Equal));
    log::info!("rubber band: {} difficulty steps", out.len());
    out
}

impl RubberBand {
    /// Throttle fraction for an opponent `gap` metres ahead of the player (negative =
    /// behind) running in `position` (0 = leader); `to_finish` is the player's distance
    /// to the finish line.
    pub fn throttle(&self, gap: f32, position: usize, to_finish: f32) -> f32 {
        if gap >= 0.0 {
            let off = self.ahead_offsets.get(position).copied().unwrap_or(0.0) - self.ahead_offsets.first().copied().unwrap_or(0.0);
            let t = ((gap - self.min_dist - off) / (self.max_dist - self.min_dist).max(1.0)).clamp(0.0, 1.0);
            let thr = self.max_throttle + (self.min_throttle - self.max_throttle) * t;
            if gap < self.player_close_distance {
                thr.max(self.player_close_max_throttle.min(self.max_throttle + 0.05))
            } else {
                thr
            }
        } else {
            let d = -gap;
            let t = ((d - self.behind_min_dist) / (self.behind_max_dist - self.behind_min_dist).max(1.0)).clamp(0.0, 1.0);
            let mut thr = self.max_throttle + (self.behind_max_throttle - self.max_throttle).max(0.0) * t;
            if to_finish < self.home_stretch_distance {
                thr = thr.min(self.home_stretch_behind_max_throttle.max(self.max_throttle));
            }
            thr
        }
    }
}
