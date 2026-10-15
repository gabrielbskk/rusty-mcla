//! The garage vehicle records (port of `tools/mc4save/vehicle.py`): one `mcCarConfig` object ("CarCfg", 8176 bytes, the bytes of a disc
//! `.xcc` file) per garage slot, 30 of them from `LocalOptions + 1360`.
//!
//! The Python decoder also lists the bytes it did not name (`_unknown`), the padding it did not expect, the vtable words and the dirty flags;
//! the importer drops all of those before anything is written, so they are not produced here.

use crate::options::{GARAGE_AT, SLOT};
use crate::tables::{BRAKES, PARTS, RIMS, TIRES, VEHICLES};
use crate::util::*;
use serde_json::{json, Map, Value};

const VEHICLE_TABLE: usize = 64;
const PERF_PARTS: &[&str] = &["exhaust", "headers", "intake", "forced_induction", "engine", "electronics", "nitrous", "clutch", "gear_kit", "shocks", "sway_bars", "brakes", "tires"];
const PAINT_FADE: &[&str] = &["None", "ColorShift", "FrontBack", "LeftRight", "TopBottom"];
const PAINT_TYPE: &[&str] = &["DirtyMatte", "Matte", "Gloss", "Metallic", "Pearl", "Sparkle"];
const PAINT_AREA: &[&str] = &["Body", "Doors", "Hood", "Front Bumper", "Rear Bumper", "Skirts"];
const RIM_MATERIAL: &[&str] = &["Painted", "Chrome", "Carbon Fiber", "Brushed Metal"];
const ZONE_START: [usize; 5] = [0, 64, 128, 192, 208];
const ZONE_CAP: [usize; 5] = [64, 64, 64, 16, 16];
const VINYL_BLEND: &[&str] = &["normal", "alpha cutout", "negative alpha cutout"];
const GAUGE_TEXT: &[&str] = &["speedometer", "odometer", "trip", "gear select", "clock"];
const INTERIOR_GROUP: &[&str] = &[
    "group 1", "group 2", "group 3", "group 4", "group 5", "group 6 (hidden from user)", "group 7 (hidden from user)", "group 8 (hidden from user)",
    "group 9 (hidden from user)", "group 10 (hidden from user)",
];
const LAMP: &[&str] = &[
    "brake light", "taillight", "turn signal", "headlight", "high beam", "side marker", "reverse light", "interior light", "fog light", "red reflector",
    "amber reflector", "clear reflector", "third brakelight", "red coplight", "blue coplight", "turn signal FL", "turn signal FR", "turn signal BL",
    "turn signal BR",
];
const KLIGHT: &[&str] = &[
    "kLightBrakelight", "kLightTaillight", "kLightThirdBrakelight", "kLightHeadlight", "kLightHiBeam", "kLightFogLight", "kLightTurnSignal", "kLightPolice0",
    "kLightPolice1", "kLightPolice2", "kLightInterior", "kLightReverseLight", "kLightNone", "kLightTurnSignalFL", "kLightTurnSignalFR", "kLightTurnSignalBL",
    "kLightTurnSignalBR",
];
const ABILITY: &[&str] = &["ZONE", "AGRO", "ROAR", "EMP"];
const UNUSED_LAYER: [u8; 20] = [0x2f, 0xae, 0x2f, 0xae, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0xff, 0xff, 0, 0];

/// The set-up files a record can come from: `<prefix>_<car>`, "" = the car's own.
fn config_prefixes() -> Vec<String> {
    let mut v: Vec<String> = ["", "starter", "pinkslip", "timetrial", "Delivery", "Payback", "pyback", "pyback_gn", "pyback_plr", "FirstRace", "CIN_01", "DETROIT", "SC_delivery", "SC_pinkslip", "SC_timetrial", "SC_tournament", "good_GH_15_2a"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for n in 1..36 {
        for k in 0..5 {
            v.push(format!("GH_{n:02}_{k}"));
        }
    }
    for n in 26..37 {
        for k in 0..4 {
            v.push(format!("SC_GH_{n:02}_{k}"));
        }
    }
    for n in 1..6 {
        for k in 0..4 {
            v.push(format!("SH_{n:02}_{k}"));
        }
    }
    v
}

/// Python's `_f`: a float rounded to 6 places; NaN and infinities as the words the Python decoder used.
fn fv(x: f64) -> Value {
    if x.is_nan() {
        json!("nan")
    } else if x.is_infinite() {
        json!(if x > 0.0 { "inf" } else { "-inf" })
    } else {
        num(round_to(x, 6))
    }
}

fn rounded(x: f64) -> Option<f64> {
    x.is_finite().then(|| round_to(x, 6))
}

fn vec3(b: B, o: usize) -> Value {
    Value::Array((0..3).map(|i| fv(b.f32(o + 4 * i))).collect())
}

fn vec4(b: B, o: usize) -> Value {
    Value::Array((0..4).map(|i| fv(b.f32(o + 4 * i))).collect())
}

fn plate(raw: &[u8]) -> String {
    let mut s = String::new();
    for c in raw {
        match *c {
            0x5B => s.push_str("[sign1]"),
            0x5C => s.push_str("[sign2]"),
            0x5D => s.push_str("[sign3]"),
            0x5E => s.push_str("[sign4]"),
            32..=126 => s.push(*c as char),
            other => s.push_str(&format!("\\x{other:02x}")),
        }
    }
    s
}

/// A vinyl layer record, 20 bytes.
fn layer(raw: &[u8]) -> Map<String, Value> {
    let b = B(raw);
    let shape = b.u16(16);
    let mut m = Map::new();
    m.insert("m_oScale".into(), json!([fv(b.half(0)), fv(b.half(2))]));
    m.insert("m_oPos".into(), json!([fv(b.half(4)), fv(b.half(6))]));
    m.insert("m_Color".into(), json!({"argb_hex": hex(&raw[8..12]), "rgba": [raw[9], raw[10], raw[11], raw[8]]}));
    m.insert("m_fSkew".into(), fv(b.half(12)));
    m.insert("m_fRot".into(), fv(b.half(14)));
    m.insert("m_iShapeIdx".into(), json!(if shape == 0xffff { -1 } else { shape as i64 }));
    m.insert("m_bNegateAlpha".into(), enum_name(VINYL_BLEND, raw[18] as i64));
    m.insert("m_bIsVectorShape".into(), json!(raw[19]));
    m
}

pub struct Slot {
    /// The record as the `mc4` block keeps it (the importer's `strip` already applied; `current` set).
    pub json: Value,
    pub type_name: String,
    pub empty: bool,
    pub brand_raw: Vec<u8>,
    /// The vinyl layers flagged "vector shape" (the port's livery leaves them out).
    pub flagged: Vec<Value>,
}

fn decode_record(rec: &[u8], index: usize, owned: i32) -> Slot {
    let b = B(rec);
    let type_name = cstr(b.slice(8109, 32));
    let profile = cstr(b.slice(8141, 34));
    let vehicle_index = b.i32(8088);
    let mut empty = type_name.is_empty() || type_name == "blank" || vehicle_index < 0;
    empty = empty || !(0 <= index as i64 && (index as i64) < owned as i64);
    let mut out = Map::new();
    out.insert("slot".into(), json!(index));
    out.insert("empty".into(), json!(empty));
    out.insert("TypeName".into(), json!(type_name));
    out.insert("ProfileName".into(), json!(profile));
    out.insert("vehicle_index".into(), json!(vehicle_index));
    out.insert("vehicle_table_name".into(), if vehicle_index >= 0 && (vehicle_index as usize) < VEHICLE_TABLE { json!(VEHICLES[vehicle_index as usize]) } else { Value::Null });

    // ---- CarCfgPerf (+20) ----
    let stages: Vec<u8> = b.slice(28, 13).to_vec();
    let brand: Vec<u8> = b.slice(43, 13).to_vec();
    let zip = |v: &[u8]| Value::Object(PERF_PARTS.iter().zip(v).map(|(k, x)| (k.to_string(), json!(*x))).collect());
    out.insert(
        "performance".into(),
        json!({
            "stages": zip(&stages), "airbags_fitted": b.u8(41), "hydraulics_fitted": b.u8(42), "brand": zip(&brand), "unused_56": hex(b.slice(56, 3)),
            "upgrade_percent": b.i8(59), "upgrade_percent_override": b.i8(60), "unk_percent_61": b.slice(61, 3).to_vec(),
        }),
    );

    // ---- CarCfgExt (+64) ----
    let mut v = Map::new();
    v.insert("m_vInteriorCharacterNeonLightFalloff".into(), fv(b.f32(68)));
    v.insert("m_vInteriorCharacterGaugeConeAng".into(), fv(b.f32(72)));
    v.insert("m_vInteriorCharacterGaugeConeFalloff".into(), fv(b.f32(76)));
    for (name, o) in [
        ("ExtNeonColor", 0x50), ("IntNeonColor", 0x60), ("DashGaugeColor", 0x70), ("DashGaugeColorB", 0x80), ("RimColorA", 0x90), ("RimColorB", 0xa0),
        ("RimColorC", 0xb0), ("RimColorD", 0xc0), ("RimColorE", 0xd0), ("m_vBrushedMetalColorA", 0xe0), ("m_vBrushedMetalColorB", 0xf0),
        ("m_vBrushedMetalColorC", 0x100), ("m_vBrakeCaliperColor", 0x110),
    ] {
        v.insert(name.into(), vec3(b, o));
    }
    for (name, o) in [("m_vHeadlightGlassColorNew", 0x120), ("m_vTaillightGlassColorNew", 0x130), ("m_vBumpedHeadlightColor", 0x140), ("m_vBumpedTaillightColor", 0x150), ("m_vWindowTintColorNew", 0x160)] {
        v.insert(name.into(), vec4(b, o));
    }
    let ext_neon = b.f32(0x170);
    let int_neon = b.f32(0x174);
    v.insert("ExtNeonIntensity".into(), fv(ext_neon));
    v.insert("IntNeonIntensity".into(), fv(int_neon));
    let mut mats = Map::new();
    for (i, k) in "ABCDE".chars().enumerate() {
        mats.insert(k.to_string(), enum_name(RIM_MATERIAL, b.u32(0x178 + 4 * i) as i64));
    }
    v.insert("RimMaterial".into(), Value::Object(mats));
    v.insert("ExtNeonFlashRate".into(), json!(b.u8(0x18c)));
    v.insert("IntNeonFlashRate".into(), json!(b.u8(0x18d)));
    v.insert(
        "gauge_text".into(),
        Value::Array(
            (0..5)
                .map(|k| {
                    let o = 0x190 + 0x20 * k;
                    json!({"element": GAUGE_TEXT[k], "m_fColor": vec4(b, o), "m_fSkew": fv(b.f32(o + 16)), "m_iFontIdx": b.i32(o + 20)})
                })
                .collect(),
        ),
    );
    v.insert(
        "lights".into(),
        Value::Array(
            (0..19)
                .map(|k| {
                    let o = 0x230 + 0x30 * k;
                    json!({"lamp": LAMP[k], "LightBulbColor": vec3(b, o), "LightCoverColor": vec3(b, o + 16), "kind": enum_name(KLIGHT, b.u32(o + 32) as i64)})
                })
                .collect(),
        ),
    );
    v.insert("m_vBaseColor".into(), Value::Array((0..10).map(|k| json!({"group": INTERIOR_GROUP[k], "rgb": vec3(b, 0x5c0 + 16 * k)})).collect()));
    v.insert(
        "paint".into(),
        Value::Array(
            (0..6)
                .map(|k| {
                    let o = 0x660 + 0x30 * k;
                    json!({
                        "area": PAINT_AREA[k], "ColorA": vec3(b, o), "ColorB": vec3(b, o + 16), "PaintType": enum_name(PAINT_TYPE, b.u32(o + 32) as i64),
                        "PaintFade": enum_name(PAINT_FADE, b.u32(o + 36) as i64), "unpainted_flag": b.u8(o + 40),
                    })
                })
                .collect(),
        ),
    );
    v.insert("m_vInteriorCharacterNeonLightPos".into(), vec3(b, 0x800));
    v.insert("m_vInteriorCharacterGaugeLightPos".into(), vec3(b, 0x810));
    v.insert("m_vInteriorCharacterGaugeConeDir".into(), vec3(b, 0x820));
    v.insert("Camber".into(), json!([fv(b.f32(0x830)), fv(b.f32(0x834))]));
    for (name, o) in [("TireOffset", 0x838), ("RideHeight", 0x83a), ("TireWidth", 0x83c), ("RimSize", 0x83e), ("TireProfile", 0x840)] {
        v.insert(name.into(), json!([b.u8(o), b.u8(o + 1)]));
    }
    let (rim, tire, brake) = (b.u8(0x844) as usize, b.u8(0x845) as usize, b.u8(0x846) as usize);
    v.insert("RimMdlIdx".into(), json!(rim));
    v.insert("rim_model".into(), if rim < RIMS.len() { json!(RIMS[rim]) } else { Value::Null });
    v.insert("TireTexIdx".into(), json!(tire));
    v.insert("tire_model".into(), if tire < TIRES.len() { json!(TIRES[tire]) } else { Value::Null });
    v.insert("BrakeMdlIdx".into(), json!(brake));
    v.insert("brake_model".into(), if brake < BRAKES.len() { json!(BRAKES[brake]) } else { Value::Null });
    let letters = b.slice(0x847, 7);
    v.insert("plate_letters".into(), json!(plate(letters)));
    v.insert("plate_letters_raw".into(), json!(letters.to_vec()));
    v.insert("LicenseBG".into(), json!(b.u8(0x84f)));
    v.insert("m_iCarPaintOverride".into(), json!(b.i32(0x854)));
    v.insert("vinyl_preset_group".into(), json!(b.i32(0x858)));
    v.insert("vinyl_preset_index".into(), json!(b.i32(0x85c)));
    v.insert("parts".into(), Value::Object(PARTS.iter().zip(b.slice(0x860, 62)).map(|(k, x)| (k.to_string(), json!(*x))).collect()));
    // (the vinyl layers are not kept except the flagged ones, below)
    let mut flagged = Vec::new();
    for z in 0..5 {
        for k in 0..ZONE_CAP[z] {
            let o = 0x904 + 20 * (ZONE_START[z] + k);
            let raw = b.slice(o, 20);
            if raw.len() == 20 && raw != UNUSED_LAYER {
                let mut lay = layer(raw);
                lay.insert("layer".into(), json!(k));
                let shape = lay["m_iShapeIdx"].as_i64().unwrap_or(-1);
                lay.insert("used".into(), json!(shape >= 0));
                if raw[19] != 0 {
                    flagged.push(Value::Object(lay));
                }
            }
        }
    }
    v.insert("vinyl_is_user_slot".into(), json!(b.u8(8068)));
    let tint_a = rounded(b.f32(0x160 + 12));
    out.insert("visual".into(), Value::Object(v));

    // ---- CarCfgInt and the record's tail ----
    out.insert("unk_8096".into(), json!(b.u32(8096)));
    out.insert("in_garage".into(), json!(b.u32(8100)));
    let h = b.u32(8104);
    out.insert("source_config_hash".into(), json!(hex32(h)));
    let mut src: Option<String> = None;
    if h != 0 {
        for p in config_prefixes() {
            let n = if p.is_empty() { type_name.clone() } else { format!("{p}_{type_name}") };
            if joaat_lower(&n) == h {
                src = Some(n);
                break;
            }
        }
    }
    out.insert("source_config".into(), json!(src));
    let ab = b.i8(8108) as i64;
    out.insert("ability".into(), if ab >= 0 { enum_name(ABILITY, ab) } else { json!(ab) });

    // ---- derived, for the importer's convenience ----
    let tint_level = tint_a.map(|a| round_int((a - 0.372) / (0.9 - 0.372) * 10.0).clamp(0, 9));
    let origin = match &src {
        Some(s) => {
            if s.to_lowercase() != type_name.to_lowercase() {
                let keep = s.chars().count().saturating_sub(type_name.chars().count() + 1);
                json!(s.chars().take(keep).collect::<String>())
            } else {
                json!("stock")
            }
        }
        None => Value::Null,
    };
    out.insert(
        "derived".into(),
        json!({
            "neon_exterior_on": rounded(ext_neon).map(|x| x > 0.0).unwrap_or(false), "neon_interior_on": rounded(int_neon).map(|x| x > 0.0).unwrap_or(false),
            "window_tint_level": tint_level, "origin": origin,
        }),
    );
    Slot { json: Value::Object(out), type_name, empty, brand_raw: brand, flagged }
}

/// The 30 garage records. `owned` = the number of vehicles owned (LocalOptions + 1340), `current` = the slot in use (+ 1344).
pub fn decode(local_options: &[u8]) -> Vec<Slot> {
    let b = B(local_options);
    let owned = b.i32(1340);
    let current = b.i32(1344);
    (0..crate::options::SLOTS)
        .map(|i| {
            let mut s = decode_record(b.slice(GARAGE_AT + i * SLOT, SLOT), i, owned);
            let cur = i as i64 == current as i64 && !s.empty;
            if let Value::Object(m) = &mut s.json {
                m.insert("current".into(), json!(cur));
            }
            s
        })
        .collect()
}
