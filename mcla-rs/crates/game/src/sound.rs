//! Maps what happens in the game onto the original sound banks.

use crate::ability::Abilities;
use crate::audio::Audio;
use crate::game::{Game, Mode};
use crate::police::{Heat, Police};
use crate::vehicles::Vehicles;
use glam::Vec3;

const AUX: &str = "vehicles/vehicle_aux";
const CAR_BANG: &str = "bangers/car_bangers";
const PROP_BANG: &str = "bangers/generic_prop_bangers";
const COLL: &str = "bangers/collisions";
const MENUS: &str = "ui/menus";
const FX: &str = "special_effects/bnk_special_effects";

/// Sound category of a prop (set from its name when the props load).
pub fn prop_sound(name: &str) -> u8 {
    let n = name.to_ascii_lowercase();
    if n.contains("gfx") || n.contains("bush") || n.contains("hedge") {
        9
    } else if n.contains("trash") || n.contains("dumpster") || n.contains("barrel") {
        2
    } else if n.contains("parking_meter") || n.contains("meter") {
        3
    } else if n.contains("news") || n.contains("vending") || n.contains("phone") || n.contains("utility_box") {
        4
    } else if n.contains("fence") || n.contains("barrier") {
        5
    } else if n.contains("hydrant") {
        6
    } else if n.contains("cone") || n.contains("plastic") {
        7
    } else if n.contains("light") || n.contains("signal") || n.contains("sign") || n.contains("pole") {
        8
    } else if n.contains("bollard") {
        10
    } else if n.contains("bus_") || n.contains("glass") || n.contains("booth") {
        11
    } else if n.contains("cardboard") || n.contains("box") {
        12
    } else if n.contains("bench") || n.contains("chair") || n.contains("table") || n.contains("palette") || n.contains("wood") || n.contains("crate") {
        1
    } else {
        0
    }
}

/// A prop was hit: (position, sound category, impact speed, knocked over?)
#[derive(Clone, Copy, Debug)]
pub struct PropHit {
    pub pos: Vec3,
    pub kind: u8,
    pub speed: f32,
    pub knocked: bool,
}

#[derive(Default)]
pub struct Sound {
    /// each ambient traffic model's sound class (`szAudioType` of its mcAmbientType.xml):
    /// 0 sedan, 1 SUV, 2 pickup, 3 truck or bus
    pub amb_types: std::collections::HashMap<String, u8>,
    gear: i32,
    damage: f32,
    impact_cool: f32,
    nitro_on: bool,
    /// a count to vary the hydraulics' samples
    hyd_count: u32,
    throttle: f32,
    cp: usize,
    count_tick: i32,
    mode: Option<Mode>,
    menu_sel: usize,
    garage_row: usize,
    race_sel: usize,
    selected_car: usize,
    heat: Option<Heat>,
    radio: f32,
    /// the scripted police radio is running: no random chatter from here
    pub scripted_radio: bool,
    ability_on: bool,
    ability_ready: bool,
    rep: i64,
    money: i64,
    /// the HUD's overhead map was up
    map_big: bool,
    horn: bool,
    skid: f32,
    airborne: bool,
    backfire_cool: f32,
    traffic_horn: f32,
    started: bool,
    /// ambience bed name -> stream file (relative to the audio root)
    beds: Option<hashbrown::HashMap<String, String>>,
}

impl Sound {
    #[allow(clippy::too_many_arguments)]
    pub fn update(&mut self, a: &mut Audio, vehicles: &Vehicles, game: &Game, police: &Police, ability: &Abilities, prop_hits: &[PropHit], cam_pos: Vec3, cam_right: Vec3, rain: f32, horn: bool, night: bool, dt: f32) {
        a.set_listener(cam_pos, cam_right);
        let in_menu = game.in_menu();
        let driving = matches!(game.mode, Mode::FreeRoam | Mode::Race);
        self.impact_cool = (self.impact_cool - dt).max(0.0);
        self.backfire_cool = (self.backfire_cool - dt).max(0.0);

        // ---- front end ------------------------------------------------------------
        if self.started {
            if self.mode != Some(game.mode) {
                let back = matches!((self.mode, game.mode), (Some(Mode::Garage), Mode::MainMenu) | (Some(Mode::RaceSelect), Mode::MainMenu) | (Some(Mode::Paused), Mode::FreeRoam) | (Some(Mode::Paused), Mode::Race) | (Some(Mode::Garage), Mode::Paused));
                a.play(&format!("{MENUS}/{}", if back { "ui_menu_back_l" } else { "ui_menu_select_l" }), 0.6);
                if game.mode == Mode::Results {
                    let won = game.last_results.first().map_or(false, |r| r.0.starts_with("YOU"));
                    a.play(&format!("{FX}/{}", if won { "end_of_race_01.l" } else { "end_of_race_05.l" }), 0.8);
                }
            } else if in_menu && (self.menu_sel != game.menu_sel || self.garage_row != game.garage_row || self.race_sel != game.race_sel || self.selected_car != game.selected_car) {
                a.play(&format!("{MENUS}/ui_menu_nav_l"), 0.5);
            }
            // the overhead map up / down (hud.sc.xml: the transitions' playSound UI_MENU_SELECT /
            // UI_MENU_BACK - D-pad up and the scripts' mapGoBig / mapGoNormal alike)
            if game.hud_map_big != self.map_big {
                a.play(&format!("{MENUS}/{}", if game.hud_map_big { "ui_menu_select_l" } else { "ui_menu_back_l" }), 0.6);
            }
            if game.rep > self.rep && game.rep / 2000 != self.rep / 2000 {
                a.play(&format!("{MENUS}/rank_up"), 0.7);
            }
            if game.money < self.money && in_menu {
                a.play(&format!("{MENUS}/ui_menu_select_l"), 0.7);
            }
        }
        self.started = true;
        self.mode = Some(game.mode);
        self.menu_sel = game.menu_sel;
        self.garage_row = game.garage_row;
        self.race_sel = game.race_sel;
        self.selected_car = game.selected_car;
        self.rep = game.rep;
        self.money = game.money;
        self.map_big = game.hud_map_big;

        // ---- race flow ------------------------------------------------------------
        if let Some(r) = game.race.as_ref() {
            let tick = if r.countdown > 0.0 { r.countdown.ceil() as i32 } else { 0 };
            if tick != self.count_tick && game.mode == Mode::Race {
                if tick == 0 {
                    a.play(&format!("{MENUS}/321go_big_go_04_fin.l"), 0.8);
                } else if tick <= 3 {
                    a.play(&format!("{MENUS}/321go_big_04_fin.l"), 0.7);
                }
            }
            self.count_tick = tick;
            let pi = vehicles.cars.iter().position(|c| c.is_player).unwrap_or(0);
            let cp = r.next.get(pi).copied().unwrap_or(0);
            // (the player's checkpoint, not the finish: HUD_CHECKPOINT_MASTER - OrderedCheckpointCore
            // 0x2c9a / 0x2d6a, the unordered core 0x1f4a; sounds.dat: CHECKPOINT_BIGWAY_MULTI =
            // UI_MENUS_CHECKPOINT_BIG_WAY_2 twice - their own fields differ, not decoded - and
            // UI_MENUS_CHECK_POIMT_SWTNR)
            if cp > self.cp && cp < r.checkpoints.len() && game.mode == Mode::Race && game.flag.is_none() && r.kind != crate::game::RaceKind::Payback {
                a.play(&format!("{MENUS}/checkpoint_big_way_2"), 0.6);
                a.play(&format!("{MENUS}/check_poimt_swtnr"), 0.4);
            }
            self.cp = cp;
        } else {
            self.cp = 0;
            self.count_tick = 0;
        }

        // ---- the player's car -----------------------------------------------------
        let Some(p) = vehicles.player() else {
            a.end_frame();
            return;
        };
        let st = &p.state;
        let speed = st.speed();
        let grounded = st.wheels.iter().any(|w| w.grounded);
        let name = p.model.name.clone();
        let throttle = if in_menu { 0.0 } else { p.input.throttle };
        let eng_vol = if in_menu { 0.28 } else { 0.62 };
        // engine sits a little towards the centre of the mix whatever the camera does
        a.engine(0, &name, st.rpm, throttle, (eng_vol, eng_vol));

        if driving {
            // gear changes
            if st.gear != self.gear && self.gear > 0 && st.gear > 0 {
                let k = if st.rpm > 5000.0 { "gearshift_hi_ck_1" } else { "gearshift_ck_1" };
                a.play(&format!("{AUX}/{k}"), 0.5);
                if st.gear > self.gear && throttle > 0.7 && p.model.params.accel.first().copied().unwrap_or(0.0) > 4.5 {
                    // forced-induction cars vent on the shift
                    if let Some(k) = a.pick(AUX, "blow_off_valve_new") {
                        a.play(&k, 0.35);
                    }
                }
            }
            // lifting off at high revs: the exhaust pops
            if self.throttle > 0.8 && throttle < 0.2 && st.rpm > p.model.params.redline * 0.75 && self.backfire_cool <= 0.0 {
                if let Some(k) = a.pick(AUX, "backfire_0") {
                    a.play(&k, 0.45);
                    self.backfire_cool = 2.5;
                }
            }
            // hydraulics: the pump as the car is switched up or down, each ram as it goes
            // (the events are cleared by the caller once heard)
            if let Some(h) = st.veh.as_ref().map(|v| v.hyd) {
                if h.ev_switch > 0 {
                    a.play(&format!("{AUX}/hydro_transition_up"), 0.6);
                } else if h.ev_switch < 0 {
                    a.play(&format!("{AUX}/hydro_transition_down"), 0.6);
                }
                if h.ev_rise != 0 {
                    a.play(&format!("{AUX}/hydraulic_rise_sweetener_0{}", 1 + self.hyd_count % 2), 0.6);
                    self.hyd_count += 1;
                }
                if h.ev_drop != 0 {
                    a.play(&format!("{AUX}/hydraulic_drop_sweetener_02"), 0.55);
                }
                if h.ev_end != 0 && h.on {
                    a.play(&format!("{AUX}/hydraulic_end_sweetener_0{}", 1 + self.hyd_count % 3), 0.35);
                }
            }
            // nitrous
            let nitro = st.nitro_on();
            if nitro && !self.nitro_on {
                a.play(&format!("{AUX}/nitrous_01_fin_l"), 0.7);
            }
            if !nitro && self.nitro_on {
                a.play(&format!("{AUX}/nitrous_tail"), 0.4);
            }
            self.nitro_on = nitro;
            // tyres
            let slip = st.wheels.iter().filter(|w| w.grounded).map(|w| (w.slip - 0.92).max(0.0)).fold(0.0, f32::max);
            let skid_t = if grounded && speed > 4.0 { (slip * 2.2).min(1.0) } else { 0.0 };
            if skid_t > 0.25 && self.skid < 0.05 {
                if let Some(k) = a.pick(AUX, "tire_chirp") {
                    a.play(&k, 0.4);
                }
            }
            self.skid += (skid_t - self.skid) * (dt * 10.0).min(1.0);
            if self.skid > 0.02 {
                a.set_loop(1, &format!("{AUX}/skids_01"), (self.skid * 0.5, self.skid * 0.5), 0.9 + (speed / 60.0).min(0.35));
            }
            // road and wind
            let roll = (speed / 55.0).clamp(0.0, 1.0);
            if grounded && roll > 0.03 {
                // (the tyre loop of what the car is rolling on - the terrain recordings of the game)
                use crate::physics::collision::Surface;
                let under = st.wheels.iter().find(|w| w.grounded).map(|w| Surface::from_u8(w.surface).kind());
                let surface = match under {
                    Some(Surface::Grass) => "terrain_grass",
                    Some(Surface::Gravel) | Some(Surface::Dirt) | Some(Surface::Railroad) => "terrain_gravel",
                    Some(Surface::Sand) => "terrain_sand",
                    Some(Surface::Sidewalk) => "terrain_sidewalk",
                    Some(Surface::Brick) | Some(Surface::Cobblestone) => "terrain_brick_01_fin",
                    Some(Surface::Wood) => "terrain_woodloop_01_fin",
                    Some(Surface::Stairs) => "terrain_stair_loop_fin_01",
                    Some(Surface::River) => "terrain_river_",
                    Some(Surface::Water) => "terrain_water_tireloop_fin",
                    Some(Surface::WaterDeep) => "terrain_ocean_deep_01",
                    _ if rain > 0.5 => "terrain_water_tireloop_fin",
                    _ => "gnr_terrain_1",
                };
                a.set_loop(2, &format!("{AUX}/{surface}"), (roll * 0.3, roll * 0.3), 0.7 + roll * 0.6);
            }
            let wind = ((speed - 18.0) / 60.0).clamp(0.0, 1.0);
            if wind > 0.01 {
                a.set_loop(3, &format!("{AUX}/wind_01_l"), (wind * 0.35, wind * 0.2), 0.8 + wind * 0.5);
                a.set_loop(4, &format!("{AUX}/wind_01_r"), (wind * 0.2, wind * 0.35), 0.8 + wind * 0.5);
            }
            if wind > 0.6 {
                let w = (wind - 0.6) * 2.0;
                a.set_loop(5, &format!("{AUX}/wind_high_01_l"), (w * 0.3, w * 0.3), 1.0);
            }
            // landings
            if self.airborne && grounded && st.airborne_time < 0.05 {
                if let Some(k) = a.pick(AUX, "suspension_car_down") {
                    a.play(&k, 0.6);
                }
            }
            self.airborne = !grounded && st.airborne_time > 0.35;
            // crashes
            let dd = st.damage - self.damage;
            if (dd > 0.004 || st.last_impact > 0.45) && self.impact_cool <= 0.0 {
                let hard = dd > 0.04 || speed > 22.0;
                let k = if hard { a.pick(CAR_BANG, "vehicle_bangers_impact") } else { a.pick(CAR_BANG, "jw_mc4_body_dent") };
                if let Some(k) = k {
                    a.play(&k, if hard { 0.9 } else { 0.6 });
                }
                if hard {
                    if let Some(k) = a.pick(CAR_BANG, "vehicle_debris") {
                        a.play(&k, 0.5);
                    }
                    if dd > 0.08 {
                        if let Some(k) = a.pick(CAR_BANG, "vehicle_glass_break") {
                            a.play(&k, 0.5);
                        }
                    }
                    a.play("sweeteners/low_end/swtnr_crnch_gnrc_01", 0.5);
                }
                self.impact_cool = 0.35;
            }
            // horn
            if horn {
                let key = format!("{AUX}/mustang_horn_fin");
                a.set_loop(6, &key, (0.5, 0.5), 1.0);
            }
            self.horn = horn;
        }
        self.gear = st.gear;
        self.damage = st.damage;
        self.throttle = throttle;

        // ---- props ----------------------------------------------------------------
        for h in prop_hits {
            let vol = (0.35 + h.speed / 40.0).min(1.0);
            let key = match h.kind {
                1 => a.pick(PROP_BANG, "bngr_prop_wood_0"),
                2 => a.pick(PROP_BANG, "bngr_prop_grbgcan").or_else(|| a.pick(PROP_BANG, "bngr_prop_dmpstr")),
                3 => a.pick(PROP_BANG, "bngr_prop_prkngmtr"),
                4 => a.pick(PROP_BANG, "bngr_prop_nwsstnd"),
                5 => a.pick(PROP_BANG, "fence_0"),
                6 => a.pick(PROP_BANG, "firehydrant"),
                7 => a.pick(PROP_BANG, "trfk_cone").or_else(|| a.pick(PROP_BANG, "plastic_slap")),
                8 => a.pick(COLL, "strt_lit_col").or_else(|| a.pick(PROP_BANG, "bngr_prop_stopsgn")),
                9 => a.pick(PROP_BANG, "bush_impact"),
                10 => Some(format!("{PROP_BANG}/yellow_bollard")),
                11 => a.pick(COLL, "glasspane").or_else(|| a.pick(COLL, "bus_stop")),
                12 => Some("bangers/prop_scrapes/crdbrdscrp_01".to_string()),
                _ => a.pick(COLL, "metal_generic").or_else(|| a.pick(COLL, "metal_reson")),
            };
            if let Some(k) = key {
                a.play_at(&k, h.pos, vol, 9.0);
            }
        }

        // ---- other vehicles --------------------------------------------------------
        if driving {
            let mut near: Vec<(f32, usize)> = vehicles.cars.iter().enumerate().filter(|(_, c)| !c.is_player && c.parked.is_none() && c.placed).map(|(i, c)| (c.state.pos.distance(cam_pos), i)).filter(|(d, _)| *d < 90.0).collect();
            near.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
            for (slot, (_, i)) in near.iter().take(5).enumerate() {
                let c = &vehicles.cars[*i];
                let g = a.gains_at(c.state.pos, 0.9, 7.0);
                let thr = c.input.throttle;
                // racers and cruisers have their own banks; traffic shares the ambient set
                if !a.engine(1 + slot as u64 + (*i as u64 % 7) * 8, &c.model.name, c.state.rpm, thr, g) {
                    let v = c.state.speed();
                    // (the engine recorded for the model's sound class)
                    let key = match (self.amb_types.get(&c.model.name.to_ascii_lowercase()).copied(), *i % 3) {
                        (Some(1), 0) | (None, 1) => "ambients/vehicles/ambveh_esc_steady",
                        (Some(1), _) | (None, 2) => "ambients/vehicles/ambveh_rav4_intosteady_fin",
                        (Some(2), _) => "ambients/vehicles/ambveh_f150_into_steady_fin",
                        (Some(3), _) => "ambients/vehicles/penske_intosteady_fin",
                        (_, 0) => "ambients/vehicles/ambveh_cblt_steady",
                        (_, 1) => "ambients/vehicles/ambveh_volvo_intosteady_fin",
                        _ => "ambients/vehicles/lotus_intosteady_fin",
                    };
                    let k = 0.45 + (v / 30.0).min(1.0) * 0.55;
                    a.set_loop(0x2000 + *i as u64, key, (g.0 * k, g.1 * k), 0.65 + (v / 28.0).min(1.2) * 0.6);
                }
            }
            // sirens
            if police.lights_on() {
                for (i, c) in vehicles.cars.iter().enumerate().filter(|(_, c)| c.cop) {
                    let g = a.gains_at(c.state.pos, 1.1, 18.0);
                    a.set_loop(0x3000 + i as u64, &format!("{AUX}/police_siren_loop_01_fin"), g, 1.0);
                }
            }
            // an impatient driver now and then
            self.traffic_horn -= dt;
            if self.traffic_horn <= 0.0 {
                self.traffic_horn = 9.0 + (a.pick(AUX, "horn_by").map_or(0, |k| k.len()) % 7) as f32 * 2.0;
                if let Some((_, i)) = near.iter().find(|(d, i)| *d > 12.0 && !vehicles.cars[*i].cop && vehicles.cars[*i].state.speed() < 3.0) {
                    if let Some(k) = a.pick(AUX, "horn_by") {
                        a.play_at(&k, vehicles.cars[*i].state.pos, 0.6, 10.0);
                    }
                }
            }
        }

        // ---- police ---------------------------------------------------------------
        // (step 19d: the payback client's friends have no siren and no radio)
        if self.heat != Some(police.heat) && !police.goons {
            match police.heat {
                Heat::PullOver => {
                    a.play(&format!("{AUX}/police_siren_alert_02"), 0.6);
                    self.bullhorn(a);
                }
                Heat::Pursuit => {
                    a.play(&format!("{AUX}/police_siren_alert_01"), 0.7);
                    self.radio = 2.0;
                }
                Heat::Busted => a.play(&format!("{AUX}/radio_cutoff_squelch_01"), 0.6),
                _ => {}
            }
        }
        self.heat = Some(police.heat);
        if matches!(police.heat, Heat::Pursuit | Heat::Search) && !police.goons {
            self.radio -= dt;
            if self.radio <= 0.0 {
                self.radio = 9.0 + (a.pick(AUX, "radio_cutoff_squelch").map_or(0, |k| k.len()) % 5) as f32 * 2.0;
                if !self.scripted_radio {
                    self.police_radio(a);
                }
            }
        }

        // ---- abilities ------------------------------------------------------------
        let on = ability.active > 0.0;
        if on && !self.ability_on {
            let key = match ability.kind {
                crate::ability::Kind::Zone => "special_abilities/zone/in_the_zone_enter_l",
                crate::ability::Kind::Agro => "special_abilities/agro/agro_ignite_oneshots_01.l",
                crate::ability::Kind::Roar => "special_abilities/roar/roar_ignite_master.l",
                crate::ability::Kind::Emp => "special_abilities/pulse/pulse_in.l",
            };
            a.play(key, 0.8);
        }
        if !on && self.ability_on {
            let key = match ability.kind {
                crate::ability::Kind::Zone => "special_abilities/zone/in_the_zone_exit_l",
                crate::ability::Kind::Emp => "special_abilities/pulse/pulse_exit.l",
                _ => "special_abilities/agro/roar_exit.l",
            };
            a.play(key, 0.6);
        }
        self.ability_on = on;
        let ready = ability.ready();
        if ready && !self.ability_ready && driving {
            a.play(&format!("{MENUS}/rep_bar"), 0.5);
        }
        self.ability_ready = ready;

        // ---- weather --------------------------------------------------------------
        if rain > 0.03 && !in_menu {
            a.set_loop(7, &format!("{AUX}/amb_bed_rain_int_fin"), (rain * 0.45, rain * 0.45), 1.0);
        }

        // ---- district ambience -----------------------------------------------------
        if self.beds.is_none() {
            let mut m = hashbrown::HashMap::new();
            if let Ok(rd) = std::fs::read_dir(a.root().join("ambience_stream")) {
                for e in rd.flatten() {
                    let p = e.path();
                    let fname = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                    let Some(stem) = fname.strip_suffix(".mp3.txt") else { continue };
                    let Ok(t) = std::fs::read_to_string(&p) else { continue };
                    let Some(ch) = t.lines().next().and_then(|l| l.split(char::from(9u8)).nth(2)) else { continue };
                    if let Some(bed) = ch.strip_prefix("AMB_BED_") {
                        // drop the speaker suffix (_L, _R, _LS, _RS)
                        let bed = bed.rsplit_once('_').map(|x| x.0).unwrap_or(bed);
                        m.insert(bed.to_string(), format!("ambience_stream/{stem}.mp3"));
                    }
                }
            }
            log::info!("audio: {} ambience beds", m.len());
            self.beds = Some(m);
        }
        let district = game.district_at(st.pos);
        let base = if game.mode == Mode::Garage {
            "GARAGE_01"
        } else if rain > 0.5 {
            "RAIN_EXT_01"
        } else {
            match (district.as_str(), night) {
                ("Hollywood", false) => "OLD_HOLLYWOOD",
                ("Hollywood", true) => "OLD_HOLLYWOOD_NIGHT",
                ("Beaches", false) => "SANTA_MONICA",
                ("Beaches", true) => "SANTA_MONICA_NIGHT",
                ("Downtown", false) => "BUSINESS_NEW",
                ("Downtown", true) => "BUSINESS_NEW_NIGHT",
                ("Hills", false) => "MULLHOLLAND",
                ("Hills", true) => "MULHOLLAND_NIGHT",
                ("SouthCentral", false) => "SOUTH_PARK",
                ("SouthCentral", true) => "SOUTH_PARK_NIGHT",
                (_, false) => "WILSHIRE_01",
                (_, true) => "WILSHIRE_NIGHT",
            }
        };
        let bed = self.beds.as_ref().and_then(|m| m.get(base)).cloned();
        a.set_bed(bed.as_deref(), if in_menu { 0.25 } else { 0.4 });

        a.update_music(driving || game.mode == Mode::Paused);
        a.end_frame();
    }

    /// The patrol car's loudhailer.
    fn bullhorn(&mut self, a: &mut Audio) {
        // police_bullhorn: one bank per officer, lines "<unit>_bullhorn_NN"
        let root = "0xec3de1f1";
        if let Some(bank) = first_bank(a, root) {
            let idx = a.bank_index(&bank);
            if let Some(n) = idx.first() {
                let prefix = n.rsplit_once('_').map(|x| x.0.to_string()).unwrap_or_default();
                if let Some(k) = a.pick(&bank, &prefix) {
                    a.play(&k, 0.8);
                }
            }
        }
    }

    /// Dispatch chatter during a pursuit.
    fn police_radio(&mut self, a: &mut Audio) {
        a.play(&format!("{AUX}/radio_cutoff_squelch_02"), 0.4);
        if let Some(bank) = random_bank(a, "police_radio") {
            if let Some(k) = a.pick(&bank, "") {
                a.play(&k, 0.6);
            }
        }
    }
}

fn banks_of(a: &Audio, root: &str) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(a.root().join(root)).map(|rd| rd.flatten().filter(|e| e.path().is_dir()).map(|e| format!("{}/{}", root, e.file_name().to_string_lossy())).collect()).unwrap_or_default();
    v.sort();
    v
}

fn first_bank(a: &Audio, root: &str) -> Option<String> {
    banks_of(a, root).into_iter().next()
}

fn random_bank(a: &mut Audio, root: &str) -> Option<String> {
    let v = banks_of(a, root);
    if v.is_empty() {
        return None;
    }
    let k = a.random() as usize % v.len();
    Some(v[k].clone())
}

/// The sound class of every ambient traffic model, from `tune/ambients/traffic/<model>/mcAmbientType.xml`.
pub fn load_amb_types(data_dir: &str) -> std::collections::HashMap<String, u8> {
    let mut out = std::collections::HashMap::new();
    let Ok(rd) = std::fs::read_dir(std::path::Path::new(data_dir).join("tune/ambients/traffic")) else { return out };
    for e in rd.flatten() {
        let Ok(t) = std::fs::read_to_string(e.path().join("mcAmbientType.xml")) else { continue };
        let Some(ty) = t.split("<szAudioType>").nth(1).and_then(|r| r.split('<').next()) else { continue };
        let class = match ty.trim() {
            "AMBIENT_SUV" => 1,
            "AMBIENT_PICKUP" => 2,
            "AMBIENT_SEDAN" => 0,
            _ => 3,
        };
        out.insert(e.file_name().to_string_lossy().to_ascii_lowercase(), class);
    }
    out
}
