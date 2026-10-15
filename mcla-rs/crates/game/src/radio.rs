//! The police radio: `tune/audio/PoliceScript.xml` holds the calls the dispatcher and
//! the officers make (scenarios such as `REPORT_SUSPECT_FLEES` or
//! `DISPATCH_REPORT_VIOLATION_LA_RIVER`, each with several wordings: a squelch, "any
//! unit", the violation code, the place, how to respond). The words are clips named
//! `joaat("<ASSET>_NN")` in the `police_radio` banks, one bank per voice.
//!
//! Besides fixed assets a wording has "special cases" the game fills in:
//! * `DIRECTION_PLAYER` (and `_STREETRACE`, `DIRECTION_CHANGE_*`) -> `..._NORTH` etc.
//! * `LOCATION1` / `LOCATION2` -> `LOCATION1_SUNSET_BLVD`: the street and the cross
//!   street, named as in the road network's zones ("HW_oh_Sunset Blvd.")
//! * `VEHICLE_MODEL` -> `MODEL_VP_VW_GOLF_83`
//! * `CALL` / `REPLY` / `ACKNOWLEDGE` -> the officer's plain line or the dispatcher's
//!   with a unit's call sign (`CALL_3ADAM5`)
//! * `PRE_SQUELCH` / `POST_SQUELCH` -> the clicks of the dispatcher's or the car's set
//! (`CALL_NEIGHBORHOOD` and a few rare ones have no recognisable assets: left out.)

use crate::audio::Audio;
use glam::Vec3;
use hashbrown::{HashMap, HashSet};
use mcla_formats::hash::joaat;
use std::collections::VecDeque;
use std::path::Path;

#[derive(Clone, Debug)]
enum Step {
    /// an asset spoken by the sender (0) or the receiver (1) of the call
    Play { asset: String, group: String, role: u8 },
    Special { name: String, role: u8 },
}

/// What a call may mention.
#[derive(Clone, Debug, Default)]
pub struct Ctx {
    /// which way the suspect is heading
    pub heading: Vec3,
    /// zone names of the street he is on and of a street crossing nearby
    pub street: String,
    pub cross: String,
    /// his car (`vp_vw_golf_83`)
    pub car: String,
    pub hour: f32,
}

#[derive(Default)]
pub struct Radio {
    /// scenario -> its wordings
    scenarios: HashMap<String, Vec<Vec<Step>>>,
    /// clip hash -> clips of the voices that have it ("police_radio/<bank>/<file>")
    clips: HashMap<u32, Vec<String>>,
    /// the dispatchers' banks (the others are officers)
    dispatchers: Vec<String>,
    /// clips to play, in order
    queue: VecDeque<(String, f32)>,
    wait: f32,
    /// seconds until the next unprompted call may start
    quiet: f32,
    rng: u32,
}

fn attr<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    let k = format!("{name}=\"");
    let i = s.find(&k)? + k.len();
    let e = s[i..].find('"')?;
    Some(&s[i..i + e])
}

const UNITS: [&str; 6] = ["3ADAM5", "1LINCOLN4", "4ADAM8", "4ADAM13", "2ADAM6", "1EDWARD7"];

/// "HW_oh_Sunset Blvd." -> "SUNSET_BLVD"
fn street_asset(zone: &str) -> String {
    let name = zone.splitn(3, '_').nth(2).unwrap_or(zone);
    name.replace('.', "").trim().to_ascii_uppercase().replace(' ', "_")
}

fn bank_of(key: &str) -> &str {
    key.rsplit_once('/').map_or("", |x| x.0)
}

impl Radio {
    pub fn load(data_dir: &str, audio_root: &Path) -> Radio {
        let mut r = Radio { rng: 0x9e37_79b9, ..Default::default() };
        if let Ok(rd) = std::fs::read_dir(audio_root.join("police_radio")) {
            for e in rd.flatten() {
                let bank = e.file_name().to_string_lossy().to_string();
                let Ok(t) = std::fs::read_to_string(e.path().join("_index.tsv")) else { continue };
                for line in t.lines() {
                    let mut c = line.split('\t');
                    let (Some(name), Some(hash)) = (c.next(), c.next()) else { continue };
                    if let Ok(h) = u32::from_str_radix(hash.trim(), 16) {
                        r.clips.entry(h).or_default().push(format!("police_radio/{bank}/{name}"));
                    }
                }
            }
        }
        // only the dispatchers call "any unit"
        let mut d: Vec<String> = r.clips.get(&joaat("CALL_ANY_01")).map(|c| c.iter().map(|k| bank_of(k).to_string()).collect()).unwrap_or_default();
        d.sort();
        d.dedup();
        r.dispatchers = d;
        let Ok(bytes) = std::fs::read(Path::new(data_dir).join("tune/audio/PoliceScript.xml")) else { return r };
        let t = String::from_utf8_lossy(&bytes);
        for sc in t.split("<Scenario ").skip(1) {
            let Some(name) = attr(sc, "name") else { continue };
            let body = sc.split("</Scenario>").next().unwrap_or("");
            let mut wordings = Vec::new();
            for script in body.split("<Script>").skip(1) {
                let script = script.split("</Script>").next().unwrap_or("");
                let mut steps = Vec::new();
                for line in script.lines() {
                    let l = line.trim();
                    let role = if attr(l, "type") == Some("Receiver") { 1 } else { 0 };
                    if l.starts_with("<PlayAsset") {
                        if let Some(asset) = attr(l, "name") {
                            steps.push(Step::Play { asset: asset.to_string(), group: attr(l, "voicegroup").unwrap_or("ALL").to_string(), role });
                        }
                    } else if l.starts_with("<SpecialCase") {
                        if let Some(n) = attr(l, "name") {
                            steps.push(Step::Special { name: n.to_string(), role });
                        }
                    }
                }
                if steps.iter().any(|s| matches!(s, Step::Play { .. })) {
                    wordings.push(steps);
                }
            }
            if !wordings.is_empty() {
                r.scenarios.insert(name.to_ascii_uppercase(), wordings);
            }
        }
        log::info!("police radio: {} calls, {} clips, {} dispatchers", r.scenarios.len(), r.clips.len(), r.dispatchers.len());
        r
    }

    pub fn loaded(&self) -> bool {
        !self.scenarios.is_empty() && !self.clips.is_empty()
    }

    pub fn busy(&self) -> bool {
        !self.queue.is_empty() || self.wait > 0.0
    }

    fn rand(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(1664525).wrapping_add(1013904223);
        self.rng >> 16
    }

    /// Every take of an asset, by any voice.
    fn takes(&self, asset: &str) -> Vec<String> {
        let mut out = Vec::new();
        for n in 1..=12 {
            match self.clips.get(&joaat(&format!("{asset}_{n:02}"))) {
                Some(c) => out.extend(c.iter().cloned()),
                None => break,
            }
        }
        out
    }

    /// Put a call on the air. Chatter (`urgent` false) waits its turn: it is dropped
    /// while another call is running or has only just ended.
    pub fn say(&mut self, scenario: &str, urgent: bool, ctx: &Ctx) -> bool {
        if !urgent && (self.busy() || self.quiet > 0.0) {
            return false;
        }
        let key = scenario.to_ascii_uppercase();
        let Some(n) = self.scenarios.get(&key).map(|w| w.len()) else { return false };
        let pick = self.rand() as usize % n;
        let steps = self.scenarios[&key][pick].clone();
        let unit = UNITS[self.rand() as usize % UNITS.len()];
        let dir = if ctx.heading.x.abs() > ctx.heading.z.abs() {
            if ctx.heading.x > 0.0 { "EAST" } else { "WEST" }
        } else if ctx.heading.z < 0.0 {
            "NORTH"
        } else {
            "SOUTH"
        };
        let dispatchers: HashSet<String> = self.dispatchers.iter().cloned().collect();
        // the dispatcher on shift (two of them, changing at 7 and 19 h)
        let on_shift = self.dispatchers.get(if (7.0..19.0).contains(&ctx.hour) { 1 } else { 0 }.min(self.dispatchers.len().saturating_sub(1))).cloned();
        // who speaks for the sender and the receiver in this call, once they have said something
        let mut voice: [Option<String>; 2] = [None, None];
        // (clip, role) in order; squelches are filled in afterwards, when the voices are known
        let mut line: Vec<(Option<String>, u8, bool)> = Vec::new();
        let mut said: Vec<String> = Vec::new();
        for st in &steps {
            let (cands, role): (Vec<String>, u8) = match st {
                Step::Play { asset, role, .. } => (vec![asset.clone()], *role),
                Step::Special { name, role } => {
                    let c = match name.as_str() {
                        "PRE_SQUELCH" | "POST_SQUELCH" => {
                            line.push((None, *role, name == "PRE_SQUELCH"));
                            continue;
                        }
                        "DIRECTION_PLAYER" | "DIRECTION_STREETRACE" | "DIRECTION_CHANGE_PLAYER" | "DIRECTION_CHANGE_STREETRACE" => vec![format!("{name}_{dir}")],
                        "LOCATION1" if !ctx.street.is_empty() => vec![format!("LOCATION1_{}", street_asset(&ctx.street))],
                        "LOCATION2" if !ctx.cross.is_empty() => vec![format!("LOCATION2_{}", street_asset(&ctx.cross))],
                        "VEHICLE_MODEL" if !ctx.car.is_empty() => vec![format!("MODEL_{}", ctx.car.to_ascii_uppercase())],
                        "CALL" => vec!["CALL_DISPATCH".to_string(), format!("CALL_{unit}")],
                        "REPLY" => vec!["REPLY".to_string(), format!("REPLY_{unit}")],
                        "ACKNOWLEDGE" => vec!["ACKNOWLEDGE".to_string(), format!("ACKNOWLEDGE_{unit}")],
                        "ACKNOWLEDGE_STANDBY" => vec!["ACKNOWLEDGE_STANDBY".to_string()],
                        _ => continue,
                    };
                    (c, *role)
                }
            };
            let all: Vec<String> = cands.iter().flat_map(|a| self.takes(a)).collect();
            if all.is_empty() {
                continue;
            }
            let r = role as usize;
            // the voice already speaking for this side goes on if it has the line; a new voice
            // is a dispatcher when the other side is an officer (and the other way round), and
            // the dispatcher is the one on shift
            let own: Vec<&String> = match voice[r].as_ref() {
                Some(v) => all.iter().filter(|k| bank_of(k) == v).collect(),
                None => Vec::new(),
            };
            let pool: Vec<&String> = if !own.is_empty() {
                own
            } else {
                let other_is_dispatch = voice[1 - r].as_ref().map(|v| dispatchers.contains(v));
                let fits: Vec<&String> = all
                    .iter()
                    .filter(|k| {
                        let d = dispatchers.contains(bank_of(k));
                        let shift_ok = !d || on_shift.as_deref().map_or(true, |s| s == bank_of(k));
                        shift_ok && other_is_dispatch.map_or(true, |o| o != d) && voice[1 - r].as_deref() != Some(bank_of(k))
                    })
                    .collect();
                if fits.is_empty() { all.iter().collect() } else { fits }
            };
            let k = pool[self.rand() as usize % pool.len()].clone();
            voice[r] = Some(bank_of(&k).to_string());
            said.push(cands[0].clone());
            line.push((Some(k), role, false));
        }
        if !line.iter().any(|l| l.0.is_some()) {
            return false;
        }
        if urgent {
            self.queue.clear();
        }
        for (clip, role, pre) in line {
            match clip {
                Some(k) => self.queue.push_back((k, 0.7)),
                None => {
                    // the click of the speaker's set
                    let is_dispatch = voice[role as usize].as_ref().map_or(false, |v| dispatchers.contains(v));
                    let bank = format!("police_radio/0x{:08x}", joaat(if is_dispatch { "POLICE_RADIO_SQUELCH_DISPATCH" } else { "POLICE_RADIO_SQUELCH_OFFICER" }));
                    let takes: Vec<String> = self.takes(if pre { "PRE_SQUELCH" } else { "POST_SQUELCH" }).into_iter().filter(|k| bank_of(k) == bank).collect();
                    if !takes.is_empty() {
                        let k = takes[self.rand() as usize % takes.len()].clone();
                        self.queue.push_back((k, 0.5));
                    }
                }
            }
        }
        self.quiet = 6.0;
        log::info!("police radio: {} (wording {} of {}): {}", key, pick + 1, n, said.join(" / "));
        true
    }

    pub fn stop(&mut self) {
        self.queue.clear();
    }

    pub fn update(&mut self, a: &mut Audio, dt: f32) {
        self.wait -= dt;
        if self.queue.is_empty() {
            self.quiet = (self.quiet - dt).max(0.0);
        }
        while self.wait <= 0.0 {
            let Some((key, vol)) = self.queue.pop_front() else {
                self.wait = 0.0;
                return;
            };
            let dur = a.clip(&key).map_or(0.0, |c| c.data.len() as f32 / c.rate.max(1) as f32);
            if dur > 0.0 {
                a.play(&key, vol);
                self.wait = dur + 0.04;
            }
        }
    }
}
