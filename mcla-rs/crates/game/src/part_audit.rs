//! Disc-backed audit of garage rows against the parts the renderer can draw.
//! Run with `MCLA_PART_AUDIT=1 mcla --data <extracted cache>`.

use crate::{assets::Assets, game::Game, vehicle_model::{self, CarConfig, Slot}};
use anyhow::{ensure, Result};
use std::path::Path;

const VISUAL_SLOTS: [Slot; 8] = [
    Slot::Body, Slot::Hood, Slot::Spoiler, Slot::BumperF,
    Slot::BumperR, Slot::Skirts, Slot::Exhaust, Slot::Seats,
];

pub fn run(data_dir: &str) -> Result<()> {
    let root = Path::new(data_dir).join("resources/vehicle");
    let mut dirs: Vec<_> = std::fs::read_dir(&root)?
        .flatten().map(|e| e.path())
        .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| {
            let n = n.to_string_lossy();
            n.starts_with("vp_") || n.starts_with("vpd_")
        }))
        .collect();
    dirs.sort();
    let mut game = Game::new(dirs.clone(), 0);
    game.load_perf_prices(data_dir);
    let assets = Assets::new(data_dir);
    let mut buyer = Game::new(dirs.clone(), 0);
    buyer.load_perf_prices(data_dir);
    let mut checked = 0usize;
    let mut findings = 0usize;
    let mut purchased = 0usize;
    let mut issues = Vec::new();
    for (ci, dir) in dirs.iter().enumerate() {
        let name = dir.file_name().unwrap().to_string_lossy();
        let car = match vehicle_model::load_car(&assets, dir) {
            Ok(c) => c,
            Err(e) => { issues.push(format!("{name}\tLOAD\t{e:#}")); continue; }
        };
        checked += 1;
        buyer.current_car = ci;
        buyer.selected_car = ci;
        if std::env::var("MCLA_PART_AUDIT_SPATIAL").is_ok() {
            let mut lo = glam::Vec3::splat(f32::INFINITY);
            let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
            for p in car.stock_parts() {
                // The base fragment is only the center shell on some cars; stock
                // bumpers and lamps define the actual exterior envelope.
                for x in [p.min.x, p.max.x] { for y in [p.min.y, p.max.y] { for z in [p.min.z, p.max.z] {
                    let v = p.transform.transform_point3(glam::Vec3::new(x, y, z));
                    lo = lo.min(v); hi = hi.max(v);
                }}}
            }
            lo -= glam::Vec3::new(0.5, 0.5, 0.5);
            hi += glam::Vec3::new(0.5, 0.5, 0.5);
            for p in car.parts.iter().filter(|p| p.placed && !p.is_base && p.tris > 0) {
                // Exhaust pipes are instanced at the fitted bumper outlets later.
                if p.primary.starts_with("exh_pipe") { continue; }
                let c = p.transform.transform_point3((p.min + p.max) * 0.5);
                if c.cmplt(lo).any() || c.cmpgt(hi).any() {
                    println!("SPATIAL\t{name}\t{}\tcenter={c:?}\tstock={lo:?}..{hi:?}", p.primary);
                }
            }
        }
        if std::env::var("MCLA_PART_AUDIT_DETAIL").ok().is_some_and(|v| name.contains(&v)) {
            if let Some(sk) = car.skeleton.as_ref() {
                let tag = sk.bone_by_tag(18166).map(|i| sk.translation(i));
                println!("DETAIL\t{name}\tSTEER_BONES\ttag={tag:?} candidates={:?}", vehicle_model::slot_candidates(Slot::Steer, sk));
            }
            for p in &car.parts {
                println!("DETAIL\t{name}\t{:?}\t{}\tstock={} placed={} parent={} tris={}", vehicle_model::classify_slot(&p.primary, &p.names), p.primary, p.stock, p.placed, p.from_parent, p.tris);
                if p.primary == "steer_whl0" { println!("DETAIL\t{name}\tSTEER_BOX\t{:?}..{:?}", p.min, p.max); }
            }
        }
        for slot in VISUAL_SLOTS {
            let variants = car.variants(slot);
            let Some(rows) = game.part_entries(ci, slot) else {
                if !variants.is_empty() {
                    issues.push(format!("{name}\t{slot:?}\tno garage price list; {} drawable variants", variants.len()));
                }
                // Non-DUB cars expose their model variants even without a GTUN
                // section. Exercise that fallback path as well (the 2007 Golf
                // has eight such sections in this cache).
                if !name.ends_with("_dub") {
                    for (index, primary) in variants.iter().enumerate().skip(1) {
                        if primary.is_empty() && !(slot == Slot::Spoiler && index == 5) {
                            issues.push(format!("{name}\t{slot:?}\tfallback index {index} has no drawable variant"));
                            continue;
                        }
                        let mut cfg = CarConfig::default();
                        cfg.set(slot, index);
                        let selected = car.placed_parts_for(&cfg);
                        if !primary.is_empty() && !selected.iter().any(|(p, _)| p.primary == *primary) {
                            issues.push(format!("{name}\t{slot:?}\tfallback index {index} {primary} is not drawn"));
                        }
                        buyer.cfg = cfg;
                        buyer.parts_owned.clear();
                        buyer.money = 1_000_000;
                        let price = buyer.part_price(ci, slot, index);
                        ensure!(buyer.try_buy_parts() && buyer.money == 1_000_000 - price,
                            "{name} {slot:?} fallback index {index} first purchase failed");
                        ensure!(buyer.try_buy_parts() && buyer.money == 1_000_000 - price,
                            "{name} {slot:?} fallback index {index} reinstall charged again");
                        purchased += 1;
                    }
                }
                continue;
            };
            if rows.is_empty() { continue; }
            if variants.is_empty() {
                issues.push(format!("{name}\t{slot:?}\t{} garage rows; no drawable variants", rows.len()));
            }
            for &(brand, price, index) in rows {
                let Some(primary) = variants.get(index) else {
                    issues.push(format!("{name}\t{slot:?}\tgarage index {index} price {price} brand {brand} out of {} variants", variants.len()));
                    continue;
                };
                if index > 0 && primary.is_empty() && !(slot == Slot::Spoiler && index == 5) {
                    issues.push(format!("{name}\t{slot:?}\tgarage index {index} price {price} brand {brand} has no drawable variant"));
                    continue;
                }
                let mut cfg = CarConfig::default();
                cfg.set(slot, index);
                let selected = car.placed_parts_for(&cfg);
                if slot == Slot::Spoiler && index == 5 && primary.is_empty()
                    && selected.iter().any(|(p, _)| vehicle_model::classify_slot(&p.primary, &p.names) == Slot::Spoiler) {
                    issues.push(format!("{name}\tSpoiler\tgarage spoiler-delete row still draws a spoiler"));
                }
                if !primary.is_empty() && !selected.iter().any(|(p, _)| p.primary == *primary) {
                    issues.push(format!("{name}\t{slot:?}\tgarage index {index} {primary} is not drawn"));
                }
                if index == 0 && !primary.is_empty() && !car.parts.iter().any(|p| p.primary == *primary && p.stock) {
                    issues.push(format!("{name}\t{slot:?}\tgarage stock maps to nonstock {primary}"));
                }
                if index > 0 && (!primary.is_empty() || (slot == Slot::Spoiler && index == 5)) {
                    buyer.cfg = CarConfig::default();
                    buyer.cfg.set(slot, index);
                    buyer.parts_owned.clear();
                    buyer.money = 1_000_000;
                    ensure!(buyer.part_price(ci, slot, index) == price, "{name} {slot:?} index {index} price differs from GTUN");
                    ensure!(buyer.try_buy_parts() && buyer.money == 1_000_000 - price,
                        "{name} {slot:?} index {index} first purchase failed");
                    ensure!(buyer.try_buy_parts() && buyer.money == 1_000_000 - price,
                        "{name} {slot:?} index {index} reinstall charged again");
                    purchased += 1;
                }
            }
        }
        for &(section, _) in &vehicle_model::EXTRA_PARTS {
            let Some(rows) = game.part_entries_sec(ci, section) else { continue };
            for &(_, _, index) in rows {
                let matching: Vec<_> = car.parts.iter().filter(|p| p.placed && vehicle_model::extra_part(&p.primary) == Some((section, index))).collect();
                if matching.is_empty() {
                    // A stock row may have no separate mesh (blower/gauge/side exhaust).
                    if index != 0 { issues.push(format!("{name}\t{section}\tgarage index {index} has no drawable model part")); }
                    continue;
                }
                let mut cfg = CarConfig::default();
                cfg.extra.insert(section.to_string(), index);
                if !car.placed_parts_for(&cfg).iter().any(|(p, _)| matching.iter().any(|m| m.primary == p.primary)) {
                    issues.push(format!("{name}\t{section}\tgarage index {index} is not drawn"));
                }
                if index > 0 {
                    let Some(k) = crate::game::XPARTS.iter().position(|x| x.1 == section) else { continue };
                    let Some(row) = buyer.xpart_rows(k).iter().position(|r| r.3 == index) else { continue };
                    buyer.parts_owned.clear();
                    buyer.xparts.clear();
                    buyer.money = 1_000_000;
                    let price = buyer.xpart_rows(k)[row].2;
                    ensure!(buyer.try_buy_xpart(k, row) && buyer.money == 1_000_000 - price,
                        "{name} {section} index {index} first purchase failed");
                    ensure!(buyer.try_buy_xpart(k, row) && buyer.money == 1_000_000 - price,
                        "{name} {section} index {index} reinstall charged again");
                    purchased += 1;
                }
            }
        }
        if car.variants(Slot::Seats).len() > 1 && car.parts.iter().any(|p| p.primary == "seats_p1" && p.placed) {
            let mut cfg = CarConfig::default();
            cfg.seats = 1;
            let selected = car.parts_for(&cfg);
            if !selected.iter().any(|p| p.primary == "seats_p1") || selected.iter().any(|p| p.primary == "seats_p0") {
                issues.push(format!("{name}\tSeats\tpassenger seat does not follow fitted front seats"));
            }
        }
        if car.variants(Slot::Body).get(1).is_some_and(|p| p.starts_with("widebody")) {
            let mut cfg = CarConfig::default();
            cfg.body = 1;
            let selected = car.parts_for(&cfg);
            for companion in ["hood_wide0", "spoiler_wide0", "door_l_widebody", "door_r_widebody"] {
                if car.parts.iter().any(|p| p.primary == companion && p.placed)
                    && !selected.iter().any(|p| p.primary == companion) {
                    issues.push(format!("{name}\tBody\twidebody does not include {companion}"));
                }
            }
        }
        for p in &car.parts {
            if !p.placed && !p.is_base && !p.primary.contains("anim") && !p.primary.starts_with("occluder") {
                issues.push(format!("{name}\tMODEL\tunplaced {} ({})", p.primary, p.file.display()));
            }
            let shaders = if p.from_parent && !car.shaders_parent.is_empty() { &car.shaders_parent } else { &car.shaders };
            for (_, si) in &p.meshes {
                if *si as usize >= shaders.len() {
                    issues.push(format!("{name}\tMODEL\t{} shader {} out of {}", p.primary, si, shaders.len()));
                    break;
                }
            }
        }
    }
    issues.sort();
    issues.dedup();
    for issue in &issues { println!("{issue}"); findings += 1; }
    if let Some(i) = dirs.iter().position(|p| p.file_name().is_some_and(|n| n == "vpd_pon_gto_66")) {
        let mut buyer = Game::new(dirs.clone(), i);
        buyer.load_perf_prices(data_dir);
        buyer.money = 10_000;
        buyer.cfg.body = 1;
        ensure!(buyer.part_price(i, Slot::Body, 1) == 3_320, "GTO body price does not match its garage file");
        ensure!(buyer.try_buy_parts() && buyer.money == 6_680, "body purchase did not charge once");
        ensure!(buyer.try_buy_parts() && buyer.money == 6_680, "owned body was charged twice");
        let k = crate::game::XPARTS.iter().position(|x| x.1 == "headlight").unwrap();
        if buyer.xpart_rows(k).len() > 1 {
            let price = buyer.xpart_rows(k)[1].2;
            ensure!(buyer.try_buy_xpart(k, 1) && buyer.money == 6_680 - price, "headlight purchase failed");
            ensure!(buyer.xpart_rows(k)[1].2 == 0, "owned headlight still has a price");
            ensure!(buyer.try_buy_xpart(k, 1) && buyer.money == 6_680 - price, "owned headlight was charged twice");
        }
        println!("PURCHASE\tGTO body and headlight first purchase/reinstall passed");
        let mut wheel_rows = 0usize;
        for section in ["tires", "brakes"] {
            let k = crate::game::XPARTS.iter().position(|x| x.1 == section).unwrap();
            let rows = buyer.xpart_rows(k);
            for (row, (_, _, price, index)) in rows.iter().enumerate() {
                if *index == 0 { continue; }
                buyer.parts_owned.clear();
                buyer.xparts.clear();
                buyer.money = 1_000_000;
                ensure!(buyer.try_buy_xpart(k, row) && buyer.money == 1_000_000 - *price,
                    "GTO {section} index {index} first purchase failed");
                ensure!(buyer.try_buy_xpart(k, row) && buyer.money == 1_000_000 - *price,
                    "GTO {section} index {index} reinstall charged again");
                wheel_rows += 1;
            }
        }
        println!("PURCHASE\t{wheel_rows} shared tire and brake rows purchased/reinstalled");
    }
    println!("PURCHASE\t{purchased} model-backed visual and extra rows purchased/reinstalled");
    println!("AUDIT\t{checked} vehicles\t{findings} findings");
    Ok(())
}
