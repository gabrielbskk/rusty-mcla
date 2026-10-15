//! The `Career` entry (port of `tools/mc4save/career.py`; layout version 11, 200000 bytes, written by the career manager's save routine
//! sub_826BF7B0). Only what the importer reads or keeps is produced: the Python decoder's layout table, residue and unknown-byte reports are
//! dropped by the importer and are not built here.

use crate::util::*;
use crate::{Error, Result};
use serde_json::{json, Map, Value};

const ADDON_OFF: usize = 150000;
const DIFFICULTIES: &[&str] = &["Easy", "Medium", "Hard", "Impossible"];
const STATES: &[&str] = &["INACTIVE", "CHECKING_PREREQUISITES", "READY_TO_START", "CHECKING_OBJECTIVES", "READY_TO_COMPLETE", "COMPLETE"];
const DISTRICTS: &[&str] = &["Hollywood", "Beaches", "Hills", "Downtown", "SouthCentral"];
const VEHICLE_CLASSES: &[&str] = &["Tuner", "Muscle", "Exotic", "SportBike", "Luxury", "Cop", "SUV", "LowRider", "Misc1"];
const ABILITIES: &[&str] = &["Zone", "Agro", "Roar", "Pulse", "Grip"];
const PART_TYPES: &[&str] = &[
    "Roof", "Hood", "RearBumper", "TruckSwap", "FrontBumper", "Fenders", "Skirts", "Spoiler", "SpoilerBody", "Headlight", "Taillight", "TaillightBody",
    "FrontGrill", "Wheeliebar", "Blower", "Intercooler", "BikeTail", "BikeTank", "BikeCowl", "DoorLeft", "DoorRight", "Exhaust", "ExhaustSide", "WideBody",
    "SpoilerWide", "HoodWide", "DoorLeftWide", "DoorRightWide", "TrunkWide", "Interior", "FrontSeat", "IntDoorLeft", "IntDoorRight", "Stereo",
    "StereoLeft", "StereoRight", "SteeringWheel", "PostGauge",
];

/// Condition type -> (class, names of the u32s saved after the `met` byte).
fn condition(t: u32) -> Option<(&'static str, &'static [&'static str])> {
    Some(match t {
        0 => ("Experience", &["start_amount"][..]),
        1 => ("String", &[][..]),
        2 => ("Mission", &[][..]),
        3 => ("Race", &[][..]),
        4 => ("Tally", &["count"][..]),
        5 => ("TallyRace", &["count"][..]),
        6 => ("TallyMission", &["count"][..]),
        7 => ("Stat", &["start_value", "value"][..]),
        _ => return None,
    })
}

fn perf_level(i: usize) -> String {
    let c = ['D', 'C', 'B', 'A'];
    if i < 20 {
        format!("{}{}", c[i / 5], i % 5)
    } else {
        i.to_string()
    }
}

/// Iteration order of the game's hash map (sub_82389998: bucket = hash % 11, new nodes go to the head of their chain).
fn map_order(names: &[&'static str]) -> Vec<&'static str> {
    let mut buckets: Vec<Vec<&'static str>> = vec![Vec::new(); 11];
    for n in names {
        buckets[(joaat_career(n) % 11) as usize].insert(0, n);
    }
    buckets.into_iter().flatten().collect()
}

struct Reader<'a> {
    d: &'a [u8],
    o: usize,
}

impl<'a> Reader<'a> {
    fn need(&self, n: usize) -> Result<()> {
        if self.o.checked_add(n).map(|e| e > self.d.len()).unwrap_or(true) {
            return Err(Error::new("career", format!("the Career entry ends inside a record (offset {:#x})", self.o)));
        }
        Ok(())
    }
    fn u32(&mut self) -> Result<u32> {
        self.need(4)?;
        let v = B(self.d).u32(self.o);
        self.o += 4;
        Ok(v)
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn u8(&mut self) -> Result<u8> {
        self.need(1)?;
        let v = self.d[self.o];
        self.o += 1;
        Ok(v)
    }
    fn raw(&mut self, n: usize) -> Result<&'a [u8]> {
        self.need(n)?;
        let v = &self.d[self.o..self.o + n];
        self.o += n;
        Ok(v)
    }
}

pub struct Career {
    /// The decoded entry as the `mc4` block keeps it (the importer's `strip` is applied by the caller; `money` and `reputation` are not in it).
    pub json: Map<String, Value>,
    pub money: u32,
    /// The Progression system's amount, if there is one.
    pub reputation: Option<u32>,
    pub missions: Vec<Mission>,
    pub active_order: Vec<String>,
}

pub struct Mission {
    pub name: String,
    pub state_id: u32,
    /// Per objective: the first condition's `count` or `value` (and `start_value`), the only ones the importer reads.
    pub objectives: Vec<Option<ObjectiveTally>>,
}

#[derive(Clone, Copy)]
pub struct ObjectiveTally {
    pub count: Option<u32>,
    pub value: Option<u32>,
    pub start_value: u32,
}

fn requirement(r: &mut Reader, first: &mut Option<ObjectiveTally>) -> Result<Value> {
    let hidden = r.u8()? != 0;
    let satisfied = r.u8()? != 0;
    let k = r.u32()?;
    let mut conds = Vec::new();
    for i in 0..k {
        let t = r.u32()?;
        let Some((cname, fields)) = condition(t) else { return Err(Error::new("career", format!("unknown condition type {t} at {:#x}", r.o - 4))) };
        let mut c = Map::new();
        c.insert("type".into(), json!(cname));
        c.insert("met".into(), json!(r.u8()? != 0));
        let mut tally = ObjectiveTally { count: None, value: None, start_value: 0 };
        for f in fields {
            let v = r.u32()?;
            c.insert((*f).into(), json!(v));
            match *f {
                "count" => tally.count = Some(v),
                "value" => tally.value = Some(v),
                "start_value" => tally.start_value = v,
                _ => {}
            }
        }
        if i == 0 {
            *first = Some(tally);
        }
        conds.push(Value::Object(c));
    }
    Ok(json!({"hidden": hidden, "satisfied": satisfied, "conditions": conds}))
}

fn missions(r: &mut Reader, addon: bool, list: &mut Vec<Value>, typed: &mut Vec<Mission>) -> Result<()> {
    let k = r.u32()?;
    for _ in 0..k {
        let nraw = r.raw(128)?;
        let name = cstr(nraw);
        let idx = r.i32()?;
        let log_slot = r.i32()?;
        let done_slot = r.i32()?;
        let state = r.u32()?;
        let np = r.u32()?;
        let mut prereq = Vec::new();
        for _ in 0..np {
            prereq.push(requirement(r, &mut None)?);
        }
        let no = r.u32()?;
        let mut objectives = Vec::new();
        let mut tallies = Vec::new();
        for _ in 0..no {
            let mut first = None;
            objectives.push(requirement(r, &mut first)?);
            tallies.push(first);
        }
        list.push(json!({
            "name": name, "index": idx, "active_slot": if log_slot < 0 { Value::Null } else { json!(log_slot) },
            "completed_slot": if done_slot < 0 { Value::Null } else { json!(done_slot) },
            "state": enum_name(STATES, state as i64), "state_id": state, "addon": addon, "prerequisites": prereq, "objectives": objectives,
        }));
        typed.push(Mission { name, state_id: state, objectives: tallies });
    }
    Ok(())
}

fn scripts(r: &mut Reader) -> Result<Value> {
    let k = r.u32()?;
    let mut res = Vec::new();
    for _ in 0..k {
        let stack = r.u32()?;
        let ln = r.u32()? as usize;
        let name = latin1(r.raw(ln)?);
        let size = r.u32()? as usize;
        let data = r.raw(size)?;
        let mut e = Map::new();
        e.insert("name".into(), json!(name));
        e.insert("stack_size".into(), json!(stack));
        e.insert("saved_bytes".into(), json!(size));
        if size % 4 == 0 {
            e.insert("saved_words".into(), json!((0..size / 4).map(|i| B(data).u32(4 * i)).collect::<Vec<_>>()));
        } else {
            e.insert("saved_hex".into(), json!(hex(data)));
        }
        res.push(Value::Object(e));
    }
    Ok(Value::Array(res))
}

/// `races`: the game's race table (`tune/race/0x0f1f4d8d`), to name the race hashes; see `convert::race_table`.
pub fn decode(d: &[u8], container_version: u32, races: &[String]) -> Result<Career> {
    let _ = container_version;
    let mut r = Reader { d, o: 0 };
    let mut out = Map::new();
    let mut by_hash = std::collections::HashMap::new();
    for x in races {
        by_hash.insert(joaat_career(x), x.as_str());
    }

    // ---- header ----
    let flags = r.u32()?;
    let version = r.u8()?;
    if version != 11 {
        return Err(Error::new("career", format!("Career layout version {version}, expected 11")));
    }
    out.insert("flags".into(), json!({"raw": flags, "career_startup_done": flags & 1 != 0, "police_unlocked": flags & 4 != 0}));

    // ---- event completion counters ----
    let n = r.u32()?;
    let mut events = Vec::new();
    for _ in 0..n {
        let name = cstr(r.raw(32)?);
        let mut c = Map::new();
        for dname in DIFFICULTIES {
            c.insert((*dname).into(), json!(r.u32()?));
        }
        events.push(json!({"name": name, "completions": Value::Object(c)}));
    }
    out.insert("events".into(), Value::Array(events));

    // ---- experience manager ----
    let money = r.u32()?;
    fn system(r: &mut Reader) -> Result<Value> {
        let amount = r.u32()?;
        let k = r.u32()?;
        let mut th = Vec::new();
        for _ in 0..k {
            let tid = cstr(r.raw(64)?);
            th.push(json!({"id": tid, "reached": r.u8()? != 0}));
        }
        Ok(json!({"amount": amount, "thresholds": th}))
    }
    fn systems(r: &mut Reader, names_for: impl Fn(usize) -> Vec<&'static str>) -> Result<Map<String, Value>> {
        let k = r.u32()? as usize;
        let names = names_for(k);
        let mut res = Map::new();
        for i in 0..k {
            let s = system(r)?;
            res.insert(names.get(i).map(|n| n.to_string()).unwrap_or_else(|| format!("system_{i}")), s);
        }
        Ok(res)
    }
    let main = systems(&mut r, |k| if k == 2 { vec!["Progression", "Unlocking"] } else { vec![] })?;
    let classes = systems(&mut r, |k| if k == VEHICLE_CLASSES.len() { map_order(VEHICLE_CLASSES) } else { vec![] })?;
    let abilities = systems(&mut r, |k| if k == ABILITIES.len() { map_order(ABILITIES) } else { vec![] })?;
    let reputation = main.get("Progression").and_then(|p| p["amount"].as_u64()).map(|v| v as u32);
    out.insert("experience".into(), json!({"main": main, "classes": classes, "abilities": abilities}));

    // ---- missions ----
    let mut mlist = Vec::new();
    let mut typed = Vec::new();
    missions(&mut r, false, &mut mlist, &mut typed)?;

    // ---- cruising hookman slots ----
    let nd = r.u32()? as usize;
    let mut slots: Vec<Vec<bool>> = Vec::new();
    for _ in 0..nd {
        let k = r.u32()?;
        let mut v = Vec::new();
        for _ in 0..k {
            v.push(r.u8()? != 0);
        }
        slots.push(v);
    }
    let mut locked = Vec::new();
    for _ in 0..nd {
        locked.push(r.u8()? != 0);
    }
    out.insert(
        "cruising_slots".into(),
        Value::Array(
            (0..nd)
                .map(|i| json!({"district": if i < DISTRICTS.len() { json!(DISTRICTS[i]) } else { json!(i) }, "district_locked": locked[i], "slots_hidden": slots[i]}))
                .collect(),
        ),
    );

    // ---- special hookmen visibility ----
    let n = r.u32()?;
    let mut alt = Vec::new();
    for _ in 0..n {
        let uid = r.u32()?;
        alt.push(json!({"uid": uid, "visible": r.u8()? != 0}));
    }
    out.insert("alt_hookmen".into(), Value::Array(alt));

    // ---- unlock flags ----
    let mut tw = Map::new();
    for dn in &DISTRICTS[..4] {
        tw.insert((*dn).into(), json!(r.u8()? != 0));
    }
    out.insert("tournaments_won".into(), Value::Object(tw));
    let n = r.u32()?;
    let mut lv = Map::new();
    for i in 0..n as usize {
        lv.insert(perf_level(i), json!(r.u8()? != 0));
    }
    out.insert("performance_levels_unlocked".into(), Value::Object(lv));
    let rows = r.u32()? as usize;
    let cols = r.u32()? as usize;
    if rows > 64 || cols > 256 {
        return Err(Error::new("career", format!("body part grid of {rows} x {cols}")));
    }
    let (mut unlocked, mut fresh) = (Map::new(), Map::new());
    for a in 0..rows {
        let cl = if a < VEHICLE_CLASSES.len() { VEHICLE_CLASSES[a].to_string() } else { a.to_string() };
        let cell = r.raw(2 * cols)?;
        let name = |c: usize| if c < PART_TYPES.len() { PART_TYPES[c].to_string() } else { c.to_string() };
        unlocked.insert(cl.clone(), json!((0..cols).filter(|c| cell[2 * c] != 0).map(name).collect::<Vec<_>>()));
        fresh.insert(cl, json!((0..cols).filter(|c| cell[2 * c + 1] != 0).map(name).collect::<Vec<_>>()));
    }
    out.insert("body_parts_unlocked".into(), Value::Object(unlocked));
    out.insert("body_parts_new".into(), Value::Object(fresh));
    out.insert("abilities_unlocked".into(), json!(r.u8()? != 0));

    let mut hashes = Vec::new();
    for _ in 0..250 {
        hashes.push(r.u32()?);
    }
    out.insert(
        "races_unlocked".into(),
        Value::Array(hashes.iter().filter(|h| **h != 0).map(|h| json!(by_hash.get(h).map(|s| s.to_string()).unwrap_or_else(|| hex32(*h)))).collect()),
    );

    let mut tt = Vec::new();
    for _ in 0..5 {
        tt.push(r.u8()? != 0);
    }
    let mut times = Vec::new();
    for _ in 0..20 {
        times.push(r.u32()?);
    }
    out.insert(
        "time_trials".into(),
        Value::Array((0..5).map(|i| json!({"district": DISTRICTS[i], "unlocked": tt[i], "target_ms": times[4 * i..4 * i + 4].to_vec()})).collect()),
    );

    let mut ring = Vec::new();
    for _ in 0..12 {
        ring.push(r.u32()?);
    }
    let (tail, head, count) = (r.u32()?, r.u32()?, r.u32()?);
    let recent: Vec<u32> = if count <= 12 && head < 12 { (0..count as usize).map(|i| ring[(head as usize + 1 + i) % 12]).collect() } else { vec![] };
    out.insert(
        "race_history".into(),
        json!({
            "oldest_first": recent.iter().map(|x| json!({"race_index": x, "race": races.get(*x as usize)})).collect::<Vec<_>>(),
            "write_index": tail, "head": head, "count": count,
        }),
    );

    // ---- pink slip profiles ----
    let uid_counter = r.u32()?;
    let mut ps = Vec::new();
    for i in 0..5 {
        let s = r.raw(8272)?;
        let sb = B(s);
        let kind = sb.i32(0);
        let mut e = Map::new();
        e.insert("slot".into(), json!(i));
        e.insert("type".into(), json!(kind));
        e.insert("empty".into(), json!(kind == -1));
        e.insert("name".into(), json!(cstr(&s[4..68])));
        e.insert("uid".into(), json!(sb.i32(8256)));
        if kind != -1 {
            e.insert("vehicle_record_hex".into(), json!(hex(&s[80..8256])));
            e.insert("vehicle_name".into(), json!(cstr(&s[80 + 8109..80 + 8109 + 64])));
        }
        ps.push(Value::Object(e));
    }
    out.insert("pink_slips".into(), json!({"uid_counter": uid_counter, "slots": ps}));

    // ---- scripts ----
    out.insert("scripts".into(), scripts(&mut r)?);

    // ---- South Central block ----
    r.o = ADDON_OFF;
    let present = r.u32()?;
    let mut addon = Map::new();
    addon.insert("present".into(), json!(present));
    if present == 1 {
        let mut am = Vec::new();
        missions(&mut r, true, &mut am, &mut typed)?;
        addon.insert("missions".into(), json!(typed[typed.len() - am.len()..].iter().map(|m| m.name.clone()).collect::<Vec<_>>()));
        mlist.extend(am);
        addon.insert("scripts".into(), scripts(&mut r)?);
    }
    out.insert("addon".into(), Value::Object(addon));

    // ---- views ----
    let slot_of = |m: &Value, key: &str| m[key].as_i64();
    let mut by_slot: Vec<&Value> = mlist.iter().filter(|m| slot_of(m, "active_slot").is_some()).collect();
    by_slot.sort_by_key(|m| slot_of(m, "active_slot"));
    let active_order: Vec<String> = by_slot.iter().map(|m| m["name"].as_str().unwrap_or("").to_string()).collect();
    let mut by_done: Vec<&Value> = mlist.iter().filter(|m| slot_of(m, "completed_slot").is_some()).collect();
    by_done.sort_by_key(|m| slot_of(m, "completed_slot"));
    let completed_order: Vec<String> = by_done.iter().map(|m| m["name"].as_str().unwrap_or("").to_string()).collect();
    out.insert("active_order".into(), json!(active_order));
    out.insert("completed_order".into(), json!(completed_order));
    out.insert("missions".into(), Value::Array(mlist));

    Ok(Career { json: out, money, reputation, missions: typed, active_order })
}
