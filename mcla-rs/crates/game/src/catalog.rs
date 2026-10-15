//! The game's vehicle catalogue (`tune/vehicle/0xe94bde63` + the DUB / South Central
//! list `0xccb5daee`): class, performance group, price and showroom stats per car, and
//! the reputation thresholds that unlock each group (`tune/career/0x327a2cd4`,
//! system "Unlocking": C0 at 10 %, B0 at 34 %, A0 at 60 % of 39 800 rep).

use hashbrown::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct CarInfo {
    pub name: String,
    pub class: String,
    pub manufacturer: String,
    /// performance group the car races in (D0, C0, B0, A0)
    pub group: String,
    /// group that has to be unlocked before the car can be bought
    pub unlock_group: String,
    pub price: i64,
    pub base_accel: f32,
    pub mods_accel: f32,
    /// top speed in mph, stock and fully upgraded
    pub base_speed: f32,
    pub mods_speed: f32,
    pub base_handling: f32,
    pub mods_handling: f32,
}

#[derive(Default)]
pub struct Catalog {
    cars: HashMap<String, CarInfo>,
}

/// total reputation of the career (progression system `MaxAmount` of the Complete
/// Edition's table; the original release had 39 800)
pub const MAX_REP: i64 = 33_500;

/// Reputation needed before cars of `group` can be bought.
pub fn group_rep(group: &str) -> i64 {
    let pct = match group.to_ascii_uppercase().as_str() {
        "C0" => 0.10,
        "B0" => 0.34,
        "A0" => 0.60,
        _ => 0.0,
    };
    (MAX_REP as f64 * pct) as i64
}

fn parse_blocks(text: &str, out: &mut HashMap<String, CarInfo>) {
    let mut cur: Option<CarInfo> = None;
    let mut pending = false;
    for line in text.lines() {
        let l = line.trim();
        if l == "Vehicle" {
            pending = true;
            continue;
        }
        if l == "{" {
            if pending {
                cur = Some(CarInfo::default());
                pending = false;
            }
            continue;
        }
        pending = false;
        if l == "}" {
            if let Some(c) = cur.take() {
                if !c.name.is_empty() {
                    out.insert(c.name.to_ascii_lowercase(), c);
                }
            }
            continue;
        }
        let Some(c) = cur.as_mut() else { continue };
        let mut it = l.split_whitespace();
        let (Some(k), Some(v)) = (it.next(), it.next()) else { continue };
        let f = || v.parse::<f32>().unwrap_or(0.0);
        match k {
            "Name" => c.name = v.to_string(),
            "Class" => c.class = v.to_string(),
            "Manufacturer" => c.manufacturer = v.to_string(),
            "Group" => c.group = v.to_string(),
            "UnlockingGroup" => c.unlock_group = v.to_string(),
            "Price" => c.price = v.parse().unwrap_or(0),
            "BaseAccel" => c.base_accel = f(),
            "ModsAccel" => c.mods_accel = f(),
            "BaseSpeed" => c.base_speed = f(),
            "ModsSpeed" => c.mods_speed = f(),
            "BaseHndlg" => c.base_handling = f(),
            "ModsHndlg" => c.mods_handling = f(),
            _ => {}
        }
    }
}

impl Catalog {
    pub fn load(data_dir: &str) -> Catalog {
        let dir = Path::new(data_dir).join("tune/vehicle");
        let mut cars = HashMap::new();
        for f in ["0xe94bde63", "0xccb5daee", "0x3cb91343"] {
            if let Ok(bytes) = std::fs::read(dir.join(f)) {
                parse_blocks(&String::from_utf8_lossy(&bytes), &mut cars);
            }
        }
        log::info!("catalog: {} vehicles", cars.len());
        Catalog { cars }
    }

    /// Catalogue entry for a vehicle directory name.
    pub fn get(&self, dir_name: &str) -> Option<&CarInfo> {
        self.cars.get(&dir_name.to_ascii_lowercase())
    }

    pub fn len(&self) -> usize {
        self.cars.len()
    }
}

