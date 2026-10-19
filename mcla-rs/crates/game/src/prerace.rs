//! The pre-race scenes' own timing (CAREER_RACES.md [N-CR-2], "Pre-race length").
//!
//! Every race-start cine script (`Game/CineScripts/generated/racestart/<scene>_generated.sco`) carries
//! generic events as (name, "", seconds, 0, 0) records. Three of them time the start: `racename` (67: the
//! race's badge, form 70), `countdown` (58: byte +318 = 1 and START `game/racetypes/countdown_321Go_3sec`,
//! so GO is 3 s after it) and `bookmark_cut_to_game` (72) - or `bookmark` + `cam_to_game` in the starter
//! 1x1, grid 4x2_01 and taunt scenes - where the camera goes back to the game. The script runs on to its
//! end after the cut; the countdown runs by itself from its event. Read from the scripts with
//! `work/career2/ev.py` (the delivery intros' events are in `hudmap::DELIVERY_INTROS`).

/// Event times in scene seconds; `racename` is -1 where the scene has none (taunts 01..09).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PreraceEvents {
    pub scene: &'static str,
    pub racename: f32,
    pub cut: f32,
    pub countdown: f32,
}

impl PreraceEvents {
    /// Scene seconds of GO (`countdown_321Go_3sec`).
    pub fn go(&self) -> f32 {
        self.countdown + 3.0
    }
}

/// The scene's events by its name (`racestart/gen_prerace_grid_2x4_01` or the bare name, any case).
pub fn events(scene: &str) -> Option<PreraceEvents> {
    let name = scene.rsplit('/').next().unwrap_or(scene).to_ascii_lowercase();
    PRERACE_EVENTS.iter().copied().find(|e| e.scene == name)
}

pub const PRERACE_EVENTS: [PreraceEvents; 82] = [
    PreraceEvents { scene: "gen_prerace_grid_1x1_01", racename: 1.8, cut: 6.3, countdown: 4.33333 },
    PreraceEvents { scene: "gen_prerace_grid_1x1_02", racename: 1.3, cut: 5.66667, countdown: 3.7 },
    PreraceEvents { scene: "gen_prerace_grid_1x1_03", racename: 1.26667, cut: 5.46667, countdown: 3.5 },
    PreraceEvents { scene: "gen_prerace_grid_1x8_07", racename: 1.53333, cut: 6.26667, countdown: 4.3 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_01", racename: 1.03333, cut: 5.16667, countdown: 3.2 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_02", racename: 1.26667, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_03", racename: 1.13333, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_04", racename: 1.33333, cut: 5.66667, countdown: 3.7 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_05", racename: 1.16667, cut: 5.5, countdown: 3.53333 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_06", racename: 1.16667, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_07", racename: 1.13333, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_02", racename: 1.13333, cut: 4.6, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_03", racename: 2.0, cut: 5.0, countdown: 5.0 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_04", racename: 3.66667, cut: 6.1, countdown: 6.1 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_05", racename: 1.3, cut: 5.66667, countdown: 3.7 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_06", racename: 1.16667, cut: 5.66667, countdown: 3.7 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_07", racename: 2.13333, cut: 6.13333, countdown: 6.13333 },
    PreraceEvents { scene: "gen_prerace_grid_2x4_split_08", racename: 1.33333, cut: 5.83333, countdown: 3.86667 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_02", racename: 1.26667, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_03", racename: 1.16667, cut: 5.33333, countdown: 3.36667 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_04", racename: 1.4, cut: 6.1, countdown: 4.13333 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_05", racename: 1.46667, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_06", racename: 1.1, cut: 5.5, countdown: 3.53333 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_07", racename: 1.5, cut: 6.13333, countdown: 4.16667 },
    PreraceEvents { scene: "gen_prerace_grid_3x3_08", racename: 2.0, cut: 6.83333, countdown: 4.86667 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_01", racename: 5.5, cut: 8.0, countdown: 8.0 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_02", racename: 1.33333, cut: 4.6, countdown: 4.6 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_03", racename: 2.0, cut: 5.0, countdown: 5.0 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_04", racename: 3.66667, cut: 6.1, countdown: 6.1 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_05", racename: 2.0, cut: 7.0, countdown: 7.0 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_06", racename: 3.8, cut: 6.8, countdown: 6.8 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_07", racename: 2.13333, cut: 6.13333, countdown: 6.13333 },
    PreraceEvents { scene: "gen_prerace_grid_4x2_08", racename: 2.83333, cut: 6.83333, countdown: 6.83333 },
    PreraceEvents { scene: "gen_prerace_starter_1x1_01", racename: 6.73333, cut: 9.23333, countdown: 9.23333 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_01", racename: 7.7, cut: 12.2, countdown: 10.2 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_02", racename: 3.03333, cut: 8.96667, countdown: 6.96667 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_03", racename: 1.76667, cut: 5.8, countdown: 3.8 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_04", racename: 3.16667, cut: 7.93333, countdown: 5.9 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_05", racename: 4.36667, cut: 10.4, countdown: 8.4 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_06", racename: 2.16667, cut: 6.7, countdown: 4.7 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_07", racename: 2.93333, cut: 9.23333, countdown: 7.23333 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_08", racename: 1.33333, cut: 5.8, countdown: 3.8 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_09", racename: 4.66667, cut: 9.2, countdown: 7.23333 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_10", racename: 1.33333, cut: 6.3, countdown: 4.3 },
    PreraceEvents { scene: "gen_prerace_starter_2x4_11", racename: 5.03333, cut: 14.4333, countdown: 12.4333 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_01", racename: 7.7, cut: 10.2, countdown: 10.2 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_02", racename: 3.03333, cut: 7.0, countdown: 7.0 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_03", racename: 1.33333, cut: 5.9, countdown: 3.83333 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_04", racename: 3.66667, cut: 6.16667, countdown: 6.16667 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_05", racename: 6.0, cut: 8.5, countdown: 8.5 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_06", racename: 2.16667, cut: 4.7, countdown: 4.7 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_07", racename: 2.5, cut: 7.33333, countdown: 7.33333 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_08", racename: 2.16667, cut: 4.66667, countdown: 4.66667 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_09", racename: 4.66667, cut: 7.16667, countdown: 7.16667 },
    PreraceEvents { scene: "gen_prerace_starter_3x3_10", racename: 1.33333, cut: 4.3, countdown: 4.3 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_01", racename: 7.7, cut: 10.2, countdown: 10.2 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_02", racename: 3.03333, cut: 7.0, countdown: 7.0 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_03", racename: 1.33333, cut: 5.9, countdown: 3.83333 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_04", racename: 3.66667, cut: 6.16667, countdown: 6.16667 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_05", racename: 6.0, cut: 8.5, countdown: 8.5 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_06", racename: 2.16667, cut: 4.7, countdown: 4.7 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_07", racename: 2.5, cut: 7.33333, countdown: 7.33333 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_08", racename: 2.16667, cut: 4.66667, countdown: 4.66667 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_09", racename: 4.66667, cut: 7.16667, countdown: 7.16667 },
    PreraceEvents { scene: "gen_prerace_starter_4x2_10", racename: 1.33333, cut: 4.3, countdown: 4.3 },
    PreraceEvents { scene: "gen_prerace_taunt_01", racename: -1.0, cut: 7.2, countdown: 7.2 },
    PreraceEvents { scene: "gen_prerace_taunt_02", racename: -1.0, cut: 6.7, countdown: 6.7 },
    PreraceEvents { scene: "gen_prerace_taunt_03", racename: -1.0, cut: 5.03333, countdown: 5.03333 },
    PreraceEvents { scene: "gen_prerace_taunt_07", racename: -1.0, cut: 8.0, countdown: 8.0 },
    PreraceEvents { scene: "gen_prerace_taunt_09", racename: -1.0, cut: 9.0, countdown: 9.0 },
    PreraceEvents { scene: "gen_prerace_taunt_hp_01", racename: 1.83333, cut: 7.33333, countdown: 5.36667 },
    PreraceEvents { scene: "gen_prerace_taunt_hp_02", racename: 1.33333, cut: 7.66667, countdown: 5.7 },
    PreraceEvents { scene: "gen_prerace_taunt_hp_03", racename: 2.66667, cut: 8.63333, countdown: 6.66667 },
    PreraceEvents { scene: "gen_prerace_taunt_h_01", racename: 1.0, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "gen_prerace_taunt_ph_01", racename: 2.6, cut: 7.33333, countdown: 5.36667 },
    PreraceEvents { scene: "gen_prerace_taunt_ph_02", racename: 1.6, cut: 7.66667, countdown: 5.7 },
    PreraceEvents { scene: "sc_gen_prerace_grid_1x1_01", racename: 1.66667, cut: 6.0, countdown: 4.0 },
    PreraceEvents { scene: "sc_gen_prerace_grid_2x4_01", racename: 1.66667, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "sc_gen_prerace_grid_2x4_02", racename: 1.66667, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "sc_gen_prerace_grid_2x4_03", racename: 1.66667, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "sc_gen_prerace_grid_2x4_split_01", racename: 1.66667, cut: 6.0, countdown: 4.03333 },
    PreraceEvents { scene: "sc_gen_prerace_grid_2x4_split_02", racename: 1.66667, cut: 6.0, countdown: 4.03333 },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prerace_scene_events() {
        // (values as in the scripts; the AROUND HOLLYWOOD recording's placement -> GO 11.7 s fits a
        // starter scene's countdown + 3 plus the 0.8 s from placement to the scene)
        let e = events("racestart/gen_prerace_starter_2x4_01").unwrap();
        assert_eq!((e.racename, e.countdown, e.cut), (7.7, 10.2, 12.2));
        assert!((e.go() - 13.2).abs() < 1e-4);
        assert_eq!(events("gen_prerace_grid_1x1_01").unwrap().countdown, 4.33333);
        // (South Central's scene is not taken for the other city's of the same name)
        assert_eq!(events("racestart/SC_gen_prerace_grid_1x1_01").unwrap().countdown, 4.0);
        assert_eq!(events("racestart/gen_prerace_grid_4x2_01").unwrap().cut, 8.0);
        assert!(events("racestart/gen_prerace_delivery_1x1_01").is_none());
        for e in PRERACE_EVENTS.iter() {
            // the camera never goes back to the game before the countdown starts
            assert!(e.cut >= e.countdown && e.countdown > 3.0 && e.cut - e.countdown < 2.1, "{}", e.scene);
        }
    }
}
