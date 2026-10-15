//! Red light races: a racer waits at certain traffic lights; pull up beside him, flash
//! the lights and it is a sprint across town with no checkpoints but the finish.
//!
//! The places come from the game's start scenes (`racestart/gen_prerace_red_<district>NN_01`:
//! the two cars' movers give the slots at the stop line, the camera the start scene).
//! The finish is one of the eighteen landmarks of the cruising script (`script_obj/game/
//! CruisingHookmen/0x18ac539b`: a table of name, district, the text ids of the
//! announcement, position and radius - Tower Records, the Chateau Marmont, Randy's
//! Donuts, ...).

use crate::ai::AiDriver;
use crate::cine::Cine;
use crate::game::Game;
use crate::races::RaceDef;
use crate::render::Gpu;
use crate::vehicles::{Vehicles, PALETTE};
use glam::{Quat, Vec3};

pub struct Spot {
    pub scene: String,
    pub district: &'static str,
    pub player: (Vec3, f32),
    pub rival: (Vec3, f32),
}

pub struct Landmark {
    /// string table id of the announcement ("PRERACE_RED_TOWER_RECORDS")
    pub text: String,
    /// its "RL_" name, a string table key too ("RL_TowerRecords" = "SUNSET MUSIC"): the start
    /// script's race name (`StringTable_GetString`, on the overhead map's title bar)
    pub key: String,
    pub pos: Vec3,
    pub radius: f32,
}

#[derive(Default)]
pub struct RedLights {
    pub spots: Vec<Spot>,
    pub landmarks: Vec<Landmark>,
    /// announcement of the race just started
    pub announce: Option<String>,
    /// the "RL_" key of the race just started (its name; None racing to another light)
    pub title_key: Option<String>,
    /// the spot whose racer is out waiting
    waiting: Option<usize>,
    /// the districts whose hookman has been found in this session
    found: [bool; 5],
    hint: f32,
    rng: u32,
    /// step 19d: the racer out waiting - (character, vehicle, set-up) -, the difficulty his
    /// car is set up for, and a vehicle to be read before he can come (the loop reads it)
    pub man: Option<(String, String, String)>,
    pub diff: i32,
    pub want: Option<String>,
}

const TAG: &str = "redlight";

/// The red light racers of each district (data +1808 of `<d>_redlight_hk1.sco`: Hollywood
/// 0x18ac539b, Beaches 0x44dd0df5, Hills 0xb84e26f3, Downtown 0x22a3b093, South Central
/// sc/CruisingHookmen/0x47ffe3b8), in the order of `mc4::DISTRICTS`.
pub const RACERS: [&[&str]; 5] = [&["GH_01", "GH_15"], &["GH_02", "GH_17"], &["GH_14", "GH_22"], &["GH_08", "GH_19"], &["GH_35"]];

fn yaw_of(q: Quat) -> f32 {
    2.0 * q.y.atan2(q.w)
}

impl RedLights {
    pub fn load(cine: &Cine, data_dir: &str) -> RedLights {
        let mut r = RedLights { rng: 0x2545f491, ..Default::default() };
        // the landmark table: "RL_<name>", ..., text id, raceback text id, then
        // (index, 1, base, field) x, y, z (3, index, 3, base, field) radius
        let script = std::path::Path::new(data_dir).join("script_obj/game/CruisingHookmen/0x18ac539b");
        if let Some(sco) = std::fs::read(script).ok().and_then(|d| mcla_formats::sco::decode(&d).ok()) {
            use mcla_formats::sco::Tok;
            let t = sco.consts();
            for i in 0..t.len() {
                let Tok::Str(name) = &t[i] else { continue };
                if !name.starts_with("RL_") {
                    continue;
                }
                let strs: Vec<(usize, &String)> = t[i + 1..(i + 40).min(t.len())].iter().enumerate().filter_map(|(k, x)| if let Tok::Str(s) = x { Some((i + 1 + k, s)) } else { None }).take(2).collect();
                if strs.len() < 2 || !strs[1].1.contains("RACEBACK") {
                    continue;
                }
                let nums: Vec<f32> = t[strs[1].0 + 1..(strs[1].0 + 20).min(t.len())].iter().map_while(|x| if let Tok::Num(v) = x { Some(*v as f32) } else { None }).collect();
                if nums.len() >= 13 {
                    r.landmarks.push(Landmark { text: strs[0].1.clone(), key: name.clone(), pos: Vec3::new(nums[4], nums[5], nums[6]), radius: nums[12].clamp(8.0, 40.0) });
                }
            }
        }
        let mut scenes = cine.scenes("racestart/gen_prerace_red_");
        scenes.extend(cine.scenes("racestart/sc_gen_prerace_red_"));
        for sc in scenes {
            let (Some(p), Some(o)) = (cine.car_pose(&sc, &|l| l == "drv00", false), cine.car_pose(&sc, &|l| l == "drv01", false)) else { continue };
            let code = sc.rsplit("red_").next().unwrap_or("");
            let district = match &code[..code.len().min(2)] {
                "be" => "Beaches",
                "dt" => "Downtown",
                "hl" => "Hills",
                "sc" => "SouthCentral",
                _ => "Hollywood",
            };
            r.spots.push(Spot { scene: sc, district, player: (p.0, yaw_of(p.1)), rival: (o.0, yaw_of(o.1)) });
        }
        log::info!("red light races: {} places, {} landmarks to race to", r.spots.len(), r.landmarks.len());
        // (test aid MCLA_REDLIGHT_TEST: where the places are)
        if std::env::var("MCLA_REDLIGHT_TEST").is_ok() {
            for s in r.spots.iter().take(4) {
                log::info!("red light: place {} ({}) at {:?}", s.scene, s.district, s.player.0);
            }
        }
        r
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng % 100_000) as f32 / 100_000.0
    }

    /// Take the waiting racer off the street.
    pub fn clear(&mut self, vehicles: &mut Vehicles) {
        if self.waiting.take().is_some() {
            vehicles.cars.retain(|c| c.driver != TAG);
        }
        self.man = None;
    }

    /// Keep a racer waiting at the nearest light; returns (race, start scene) when the
    /// player has pulled up beside him and flashed.
    pub fn update(&mut self, game: &mut Game, vehicles: &mut Vehicles, gpu: &Gpu, flash: bool, dt: f32) -> Option<(RaceDef, String)> {
        let player = vehicles.player()?;
        let (pp, speed, pname) = (player.state.pos, player.state.speed(), player.model.name.clone());
        let near = self.spots.iter().enumerate().map(|(i, s)| (i, s.player.0.distance(pp))).min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())?;
        // the racer comes and goes with the distance
        match self.waiting {
            Some(w) if w != near.0 || near.1 > 380.0 || !vehicles.cars.iter().any(|c| c.driver == TAG) => self.clear(vehicles),
            None if near.1 < 260.0 && (near.1 > 60.0 || std::env::var("MCLA_REDLIGHT_TEST").is_ok()) => {
                let pool: Vec<usize> = (0..vehicles.models.len()).filter(|&i| {
                    let n = &vehicles.models[i].name;
                    !n.starts_with("va_") && !n.contains("police") && !vehicles.models[i].bike && *n != pname
                }).collect();
                // (step 19d: one of the district's racers in a car of his for the difficulty;
                // his vehicle is read first - `want` - and he comes the frame after)
                let s19d_d = crate::mc4::DISTRICTS.iter().position(|n| n.eq_ignore_ascii_case(self.spots[near.0].district)).unwrap_or(0);
                if self.man.is_none() {
                    let list = RACERS[s19d_d.min(4)];
                    let name = list[(self.rand() * list.len() as f32) as usize % list.len()];
                    if let Some((car, cfg, m)) = game.mc4_character_car(name, self.diff) {
                        log::info!("red light: {} in his {} ({}, mod {}) at difficulty {}", name, car, cfg, m, self.diff);
                        self.man = Some((name.to_string(), car, format!("{cfg}#{m}")));
                    }
                }
                let s19d_model = self.man.as_ref().and_then(|m| vehicles.models.iter().position(|v| v.name.eq_ignore_ascii_case(&m.1)));
                if self.man.is_some() && s19d_model.is_none() {
                    self.want = self.man.as_ref().map(|m| m.1.clone());
                    return None;
                }
                if !pool.is_empty() || s19d_model.is_some() {
                    let s = &self.spots[near.0];
                    let (rp, ry) = s.rival;
                    let mi = s19d_model.unwrap_or_else(|| pool[(self.rand() * pool.len() as f32) as usize % pool.len()]);
                    let color = PALETTE[(self.rand() * PALETTE.len() as f32) as usize % PALETTE.len()];
                    let idx = vehicles.spawn(gpu, vehicles.models[mi].clone(), rp + Vec3::Y * 0.6, ry, color, false);
                    if let Some((_, _, cfg)) = self.man.as_ref() {
                        let (cfg, m) = cfg.split_once('#').unwrap_or((cfg.as_str(), "0"));
                        vehicles.cars[idx].vinyl_name = cfg.to_string();
                        vehicles.cars[idx].set_perf(m.parse::<i32>().unwrap_or(0).clamp(0, 100) as f32 / 100.0);
                    }
                    vehicles.cars[idx].driver = TAG.to_string();
                    vehicles.cars[idx].pinned = true;
                    // he holds his place at the line
                    let mut d = AiDriver::new(77);
                    d.chase = Some((rp, Vec3::ZERO));
                    vehicles.cars[idx].ai = Some(d);
                    self.waiting = Some(near.0);
                    log::info!("red light: a racer waits at {} ({:.0} m away)", self.spots[near.0].scene, near.1);
                }
            }
            _ => {}
        }
        let w = self.waiting?;
        let s = &self.spots[w];
        let beside = Vec3::new(pp.x - s.player.0.x, 0.0, pp.z - s.player.0.z).length() < 9.0 && speed < 4.0;
        // (an alternative hookman - ids 350..354 here - who is not marked visible is hidden;
        // within 80 m he is found: the splash HOOKMAN_FOUND_REDLIGHT and mcHookman_Hide(h, 0) -
        // CruisingHookmanCore 0x7ad..0x8aa)
        if let Some(d) = crate::mc4::DISTRICTS.iter().position(|n| n.eq_ignore_ascii_case(s.district)) {
            if !self.found[d] && s.rival.0.distance(pp) < 80.0 {
                self.found[d] = true;
                if !game.mc4_alt_visible(350 + d as u32) {
                    let t = game.career.strings.get("HOOKMAN_FOUND_REDLIGHT").map(|x| x.to_uppercase());
                    if let Some(t) = t {
                        game.show_message(&t);
                    }
                }
            }
        }
        // (the player can flash him: the district's landmark races are unlocked - `mc4`)
        if beside {
            game.mc4_redlight_seen(s.district);
        }
        self.hint -= dt;
        if beside && self.hint <= 0.0 {
            self.hint = 3.0;
            game.show_message("F  RED LIGHT RACE");
        }
        if !(beside && flash) {
            return None;
        }
        // the finish: a landmark some way off (failing that, another light)
        let here = s.player.0;
        let mut cands: Vec<(Vec3, Option<String>, Option<String>)> = self.landmarks.iter().filter(|l| { let d = l.pos.distance(here); d > 500.0 && d < 2600.0 }).map(|l| (l.pos, Some(l.text.clone()), Some(l.key.clone()))).collect();
        if cands.is_empty() {
            cands = self.landmarks.iter().filter(|l| l.pos.distance(here) > 300.0).map(|l| (l.pos, Some(l.text.clone()), Some(l.key.clone()))).collect();
        }
        if cands.is_empty() {
            cands = self.spots.iter().enumerate().filter(|(i, _)| *i != w).map(|(_, x)| (x.player.0, None, None)).collect();
        }
        let pick = (self.rand() * cands.len() as f32) as usize % cands.len();
        let (finish, text, key) = cands[pick].clone();
        self.announce = text;
        self.title_key = key;
        let s = &self.spots[w];
        let def = RaceDef {
            id: "game/Race/RedLight/red_light_race".to_string(),
            name: "red light race".to_string(),
            race_type: "Ordered".to_string(),
            sub_type: "RedLight".to_string(),
            district: s.district.to_string(),
            laps: 1,
            beacon: s.player.0,
            // the rival on pole, the player beside him (the player takes the last slot)
            grid: vec![s.rival, s.player],
            checkpoints: vec![finish],
            opponents: 1,
            ..Default::default()
        };
        log::info!("red light: race from {} to {:?} ({:.0} m as the crow flies)", s.scene, self.announce, finish.distance(here));
        let scene = s.scene.clone();
        // (step 19d: the man races in his car and set-up; the next one's difficulty is drawn
        // afresh - RAND_INT_RANGE(0, 3))
        let man = self.man.clone();
        self.clear(vehicles);
        if let Some((name, car, cfg)) = man {
            let cfg = cfg.split_once('#').map_or(cfg.clone(), |c| c.0.to_string());
            game.opt_x.arcade.d.redlight_man = Some((name, car, cfg));
        }
        self.diff = (self.rand() * 4.0) as i32 % 4;
        Some((def, scene))
    }
}
