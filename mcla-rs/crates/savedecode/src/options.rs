//! The `LocalOptions` entry (port of `tools/mc4save/options.py`): the options, the garage header, the per-model purchase records, the damage
//! table, the collectables and the ability levels. The garage vehicle records themselves are in `vehicle.rs`.

use crate::tables::VEHICLES;
use crate::util::*;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const GARAGE_AT: usize = 1360;
pub const SLOT: usize = 8176;
pub const SLOTS: usize = 30;
const RECORDS_AT: usize = 0x3C370;
const RECORD: usize = 84;
const RECORDS: usize = 77;
const DAMAGE_AT: usize = 0x3DCB4;
const COLLECT_AT: usize = 0x3DF84;
const COLLECT_N: usize = 60;
const ABILITY_AT: usize = 0x3DFC2;
const PLAYLIST_AT: usize = 0x3DFC8;
const PLAYLIST_N: usize = 32;

const CONTROL_CONFIGS: &[&str] = &[
    "Throttle_Stick", "Throttle_Trigger", "Throttle_Stick_Manual", "Throttle_Trigger_Manual", "Custom_Pad", "Steering_Wheel", "Steering_Wheel_Manual",
    "Custom_Steering_Wheel", "Throttle_Stick_Hydraulics",
];
/// Mapping slot -> action (table 0x8204FF90 of the executable).
const ACTIONS: &[(usize, &str)] = &[
    (0, "STEER_AXIS"), (1, "STEER_AXIS_UP_DN"), (2, "GAS"), (3, "BRAKE"), (4, "SHIFT_UP"), (5, "SHIFT_DOWN"), (6, "HANDBRAKE"), (7, "WEIGHT_XFER"),
    (8, "NITRO"), (9, "HEADLIGHTS"), (10, "HORN"), (11, "IN_ZONE"), (12, "CAM_UP"), (13, "CAM_DOWN"), (14, "MAP"), (15, "CAM_ZOOM_IN"),
    (16, "CAM_ZOOM_OUT"), (17, "PAUSE"), (18, "TALK"), (19, "MUSIC_UP"), (20, "MUSIC_DOWN"), (21, "MUSIC_PAUSE"), (22, "CAM_PAN_LEFT"),
    (23, "CAM_PAN_RIGHT"), (24, "CAM_PAN_UP"), (25, "CAM_PAN_DOWN"), (26, "LOOKBACK"), (27, "CAMERA_LEFT"), (28, "CAMERA_RIGHT"), (29, "CAMERA_UP"),
    (30, "POPUP_LEFT"), (31, "POPUP_RIGHT"), (32, "POPUP_UP"), (33, "POPUP_DOWN"), (34, "POPUP_ANALOG_LEFT_RIGHT"), (35, "POPUP_ANALOG_UP_DOWN"),
    (36, "POPUP_SELECT"), (37, "POPUP_BACK"), (38, "WHEEL_FRONT_LEFT"), (39, "WHEEL_FRONT_RIGHT"), (40, "WHEEL_BACK_LEFT"), (41, "WHEEL_BACK_RIGHT"),
    (42, "HUD_PLAYERLIST"), (43, "CONVERTIBLE_UP_DN"), (44, "HYDRAULICS"), (45, "UI_START"), (46, "HUD_CYCLE"), (64, "PHOTO_CAM_MOVE_UP"),
    (65, "PHOTO_CAM_MOVE_DN"), (66, "PHOTO_CAM_MOVE_LT"), (67, "PHOTO_CAM_MOVE_RT"), (68, "PHOTO_CAM_ZOOM_IN"), (69, "PHOTO_CAM_ZOOM_OUT"),
    (70, "PHOTO_CAM_PAN_UP"), (71, "PHOTO_CAM_PAN_DN"), (72, "PHOTO_CAM_PAN_LT"), (73, "PHOTO_CAM_PAN_RT"),
];
const BUTTONS: &[&str] = &["L2", "R2", "L1", "R1", "RUP", "RRIGHT", "RDOWN", "RLEFT", "SELECT", "L3", "R3", "START", "LUP", "LRIGHT", "LDOWN", "LLEFT"];
const CAMERAS: &[&str] = &["Bumper", "POV", "Boom 1", "Boom 2", "Boom 3"];
const ARROWS: &[&str] = &["option_arrow_compass", "option_arrow_top", "option_arrow_none"];
const GENRES: &[&str] = &[
    "MUSIC_GENRE_ECLECTIC", "MUSIC_GENRE_ELECTRONIC", "MUSIC_GENRE_HARDROCK", "MUSIC_GENRE_HIPHOP", "MUSIC_GENRE_ROCK", "MUSIC_GENRE_TECHNO",
    "MUSIC_GENRE_WEST_RAP", "MUSIC_GENRE_ALL", "MUSIC_GENRE_ON_THE_FLY",
];
const ABILITIES: &[&str] = &["zone", "agro", "roar", "pulse"];

/// Part-ownership record: category -> (offset in the record, bits). Sorted by category.
const CATEGORIES: &[(u32, usize, usize)] = &[
    (1, 21, 6), (2, 22, 6), (3, 23, 6), (4, 24, 6), (5, 80, 6), (6, 79, 6), (7, 50, 16), (8, 52, 150), (9, 71, 10), (10, 30, 80), (11, 14, 6),
    (12, 13, 6), (13, 15, 6), (14, 12, 6), (16, 20, 6), (17, 16, 6), (18, 17, 6), (19, 18, 6), (21, 25, 6), (30, 19, 6), (35, 73, 45), (43, 26, 6),
    (44, 27, 6), (45, 28, 6), (46, 29, 6), (52, 40, 80), (53, 81, 6),
];
fn category_name(c: u32) -> String {
    match c {
        1 => "intercooler", 2 => "bike_tail", 3 => "bike_tank", 4 => "bike_cowl", 5 => "paint", 6 => "vinyl", 7 => "tires", 8 => "rims", 9 => "brakes",
        10 => "exhaust", 11 => "bumper_front", 12 => "bumper_rear", 13 => "spoiler", 14 => "hood", 16 => "blower", 17 => "skirts", 18 => "headlight",
        19 => "taillight", 21 => "body", 30 => "frontgrill", 35 => "performance", 43 => "seats_f", 44 => "stereo", 45 => "steer_whl", 46 => "post_gauge",
        52 => "side_exhaust", 53 => "scissor_door",
        other => return format!("cat{other}"),
    }
    .to_string()
}

fn bits(b: &[u8], n: usize) -> Vec<usize> {
    (0..n).filter(|i| b.get(i >> 3).map(|x| x >> (i & 7) & 1 == 1).unwrap_or(false)).collect()
}

fn mapping(b: B, at: usize) -> Value {
    let mut out = Map::new();
    for i in 0..74usize {
        let (dvc, key) = (b.u32(at + 8 * i), b.u32(at + 8 * i + 4));
        let name = ACTIONS.iter().find(|a| a.0 == i).map(|a| a.1.to_string());
        if dvc == 0 && key == 0 && name.is_none() {
            continue;
        }
        let device = match dvc {
            0 => json!("none"),
            8 => json!("button"),
            9 => json!("axis"),
            other => json!(other),
        };
        let mut e = Map::new();
        e.insert("slot".into(), json!(i));
        e.insert("device".into(), device);
        e.insert("key".into(), json!(key));
        if dvc == 8 && key < 16 {
            e.insert("button_guess".into(), json!(BUTTONS[key as usize]));
        }
        out.insert(name.unwrap_or_else(|| format!("slot{i}")), Value::Object(e));
    }
    Value::Object(out)
}

fn names_or_null(names: &[&str], v: i64) -> Value {
    if v >= 0 && (v as usize) < names.len() {
        json!(names[v as usize])
    } else {
        Value::Null
    }
}

/// One per-model purchase record.
pub struct PurchaseRecord {
    /// The record as the importer's `mc4` block keeps it.
    pub json: Value,
    pub in_use: bool,
    /// The vehicle's name, or its hash as text, or None when the hash is 0.
    pub vehicle: Option<String>,
    /// Owned parts by category name, in the order of the names.
    pub owned: BTreeMap<String, Vec<usize>>,
}

pub struct Options {
    pub options: Value,
    pub unknown: Value,
    pub current_slot: i32,
    pub name_hashes: Vec<String>,
    pub records: Vec<PurchaseRecord>,
    /// Per garage slot: (damage, the four zones).
    pub damage: Vec<(f64, [f64; 4])>,
    pub collectables_found: Vec<usize>,
    pub ability_levels: Value,
}

pub fn decode(p: &[u8]) -> Options {
    let b = B(p);
    let mut unknown: Vec<Value> = Vec::new();
    let mut unk = |off: usize, n: usize, note: &str| unknown.push(json!({"off": off, "len": n, "hex": hex(b.slice(off, n)), "note": note}));
    let cfg_pad = b.u16(0) as i16 as i64;
    let cfg_wheel = b.u16(2) as i16 as i64;
    let cine = b.u32(1192);
    let genre = b.u32(1328);
    let lang = b.u32(1320);
    let language_name = match lang {
        0 => json!("english"),
        1 => json!("spanish"),
        2 => json!("french"),
        3 => json!("german"),
        4 => json!("italian"),
        _ => Value::Null,
    };
    let opts = json!({
        "control_config_pad": cfg_pad,
        "control_config_pad_name": names_or_null(CONTROL_CONFIGS, cfg_pad),
        "control_config_wheel": cfg_wheel,
        "control_config_wheel_name": names_or_null(CONTROL_CONFIGS, cfg_wheel),
        "custom_mapping_pad": mapping(b, 4),
        "custom_mapping_wheel": mapping(b, 596),
        "vibration": b.u32(1188) != 0,
        "cinematics": cine,
        "idle_camera": cine == 1 || cine == 2,
        "camera_shake": cine == 1 || cine == 3,
        "gps_arrow": b.u32(1204),
        "gps_arrow_name": names_or_null(ARROWS, b.u32(1204) as i64),
        "camera": b.u32(1208),
        "camera_name": names_or_null(CAMERAS, b.u32(1208) as i64),
        "steering_sensitivity": round_to(b.f32(1224), 6),
        "convertible_top_down": b.u8(1228) != 0,
        "subtitles": b.u16(1238) != 0,
        "volume_effects": round_to(b.f32(1240), 6),
        "volume_music": round_to(b.f32(1244), 6),
        "volume_voice": round_to(b.f32(1248), 6),
        "volume_player_vehicle": round_to(b.f32(1252), 6),
        "volume_cutscene": round_to(b.f32(1256), 6),
        "online_character": b.u32(1316),
        "language": lang,
        "language_name": language_name,
        "airride_pattern": b.u8(1324),
        "music_shuffle": b.u8(1325) != 0,
        "music_genre": genre,
        "music_genre_name": names_or_null(GENRES, genre as i64),
        "time_of_day_hours": round_to(b.f32(1332), 6),
        "custom_playlist": (0..PLAYLIST_N).filter(|i| b.u32(PLAYLIST_AT + 4 * i) != 0).map(|i| hex32(b.u32(PLAYLIST_AT + 4 * i))).collect::<Vec<_>>(),
    });
    // fields the code only initialises and compares (no reader found): kept raw
    unk(1196, 4, "u32, default 0, cleared by -nocinematics together with 1192; no reader found");
    unk(1200, 4, "u32, default 2; no reader found");
    unk(1212, 4, "u32, default 0; no reader found");
    unk(1216, 4, "u32, default 1; no reader found");
    unk(1220, 4, "u32, default 0; no reader found");
    unk(1232, 4, "f32, default 1.049 (0x8204E3D4); no reader found");
    unk(1236, 2, "u16, default 0; no reader found");
    unk(1260, 4, "u32, default 0; no reader found");
    unk(1264, 10, "char[10], default empty; no writer found");
    unk(1274, 40, "wchar[20], default empty; no writer found");
    unk(1336, 1, "u8, default 1; no reader found");
    unk(0x3DFC0, 2, "u16, only cleared by the reset sub_8238C9B0");
    unk(0x3DFC6, 1, "u8, only cleared by the reset sub_8238C9B0");
    // padding proven by the code: reported only when it is not all zero
    for (off, n, note) in [
        (1229, 3, "after the convertible byte"),
        (1314, 2, "after the wide string"),
        (1326, 2, "after the music bytes"),
        (1337, 3, "after byte 1336"),
        (1352, 8, "alignment of the garage (16)"),
        (0x3DFC7, 1, "before the playlist"),
        (254024, 8, "alignment of the object (16)"),
    ] {
        if b.slice(off, n).iter().any(|c| *c != 0) {
            unk(off, n, &format!("padding expected zero: {note}"));
        }
    }

    let nrec = b.i32(1348);
    let current = b.i32(1344);
    let name_hashes = (0..SLOTS).map(|i| hex32(b.u32(GARAGE_AT + i * SLOT + 8104))).collect();

    let mut records = Vec::with_capacity(RECORDS);
    for i in 0..RECORDS {
        let r = RECORDS_AT + i * RECORD;
        let h = b.u32(r + 4);
        let mut owned = BTreeMap::new();
        let mut owned_json = Map::new();
        for (cat, off, nbits) in CATEGORIES {
            let got = bits(b.slice(r + off, (nbits + 7) / 8), *nbits);
            if !got.is_empty() {
                owned.insert(category_name(*cat), got.clone());
                owned_json.insert(category_name(*cat), json!(got));
            }
        }
        let known = VEHICLES.iter().find(|v| joaat_lower(v) == h);
        let vehicle = match known {
            Some(v) => Some(v.to_string()),
            None if h != 0 => Some(hex32(h)),
            None => None,
        };
        let mut rec = json!({
            "index": i, "in_use": (i as i64) < nrec as i64, "hash": hex32(h), "vehicle": vehicle,
            "refund_value": b.i32(r + 8), "spent_plus_1": b.u16(r + 82), "owned_parts": Value::Object(owned_json),
        });
        if b.u32(r) != 0 {
            rec["vtable_word"] = json!(hex32(b.u32(r)));
        }
        records.push(PurchaseRecord { json: rec, in_use: (i as i64) < nrec as i64, vehicle, owned });
    }

    let damage = (0..SLOTS)
        .map(|i| {
            let o = DAMAGE_AT + 24 * i;
            (round_to(b.f32(o), 6), [round_to(b.f32(o + 4), 6), round_to(b.f32(o + 8), 6), round_to(b.f32(o + 12), 6), round_to(b.f32(o + 16), 6)])
        })
        .collect();

    let collectables_found = (0..COLLECT_N).filter(|i| b.u8(COLLECT_AT + i) != 0).collect();
    let ability_levels = Value::Object(ABILITIES.iter().enumerate().map(|(i, n)| (n.to_string(), json!(b.u8(ABILITY_AT + i)))).collect());

    Options { options: opts, unknown: Value::Array(unknown), current_slot: current, name_hashes, records, damage, collectables_found, ability_levels }
}
