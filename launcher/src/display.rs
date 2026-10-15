//! Settings > Display and game: the rows the game reads from `settings.toml` (`mcla_config`). Only `display.monitor`
//! is live here; the other rows stay greyed until the launcher wires them (the key table says what the game acts on).
//!
//! Monitor numbering is the game's (crates/game/src/hooks.rs `monitor_list`): 0 = the primary monitor, then the
//! others in the order the system lists them (Tauri and winit both ask Windows' `EnumDisplayMonitors`).

use crate::logging;
use serde::Serialize;
use std::path::PathBuf;
use tauri::WebviewWindow;

#[derive(Serialize)]
pub struct MonitorView {
    index: u32,
    /// the system's name (`\\.\DISPLAY1` on Windows, shown without the prefix)
    name: String,
    width: u32,
    height: u32,
    primary: bool,
}

#[derive(Serialize)]
pub struct DisplayView {
    monitors: Vec<MonitorView>,
    /// `display.monitor` as the game will read it (0 when the file is missing or the key is bad)
    monitor: u32,
    /// the key table says the game acts on the key
    implemented: bool,
    settings_path: String,
    /// `ok`, `missing` (defaults), `broken` (not TOML: the launcher will not overwrite it)
    file: &'static str,
}

fn settings_path() -> PathBuf {
    mcla_config::paths::settings_path(None)
}

fn short_name(n: Option<&String>) -> String {
    let n = n.map(|s| s.as_str()).unwrap_or("");
    n.trim_start_matches(r"\\.\").to_string()
}

/// The monitors in the game's order: the primary first, then the rest as listed.
pub fn monitors(window: &WebviewWindow) -> Vec<MonitorView> {
    let all = window.available_monitors().unwrap_or_default();
    let primary = window.primary_monitor().ok().flatten();
    let same = |a: &tauri::Monitor, b: &tauri::Monitor| a.name() == b.name() && a.position() == b.position() && a.size() == b.size();
    let pi = primary.as_ref().and_then(|p| all.iter().position(|m| same(m, p))).unwrap_or(0);
    let mut order: Vec<usize> = vec![pi];
    order.extend((0..all.len()).filter(|i| *i != pi));
    order
        .into_iter()
        .filter(|i| *i < all.len())
        .enumerate()
        .map(|(n, i)| {
            let m = &all[i];
            MonitorView { index: n as u32, name: short_name(m.name()), width: m.size().width, height: m.size().height, primary: n == 0 }
        })
        .collect()
}

pub fn view(window: &WebviewWindow) -> DisplayView {
    let path = settings_path();
    let l = mcla_config::Settings::load(&path);
    let file = match l.status {
        mcla_config::Status::Ok => "ok",
        mcla_config::Status::Missing => "missing",
        mcla_config::Status::Broken => "broken",
    };
    DisplayView {
        monitors: monitors(window),
        monitor: l.settings.display.monitor,
        implemented: mcla_config::schema().iter().any(|k| k.key == "display.monitor" && k.implemented),
        settings_path: path.to_string_lossy().into_owned(),
        file,
    }
}

/// Writes `display.monitor` (only that key; atomic). A broken file is not overwritten.
pub fn set_monitor(window: &WebviewWindow, index: u32) -> Result<DisplayView, String> {
    if index > 15 {
        return Err("out_of_range".into());
    }
    let path = settings_path();
    mcla_config::set_key(&path, "display", "monitor", mcla_config::toml::Value::Integer(index as i64)).map_err(|e| {
        logging::error(&format!("settings.toml: display.monitor not written: {e}"));
        if e.kind() == std::io::ErrorKind::InvalidData { "broken".to_string() } else { "io".to_string() }
    })?;
    logging::info(&format!("settings.toml: display.monitor = {index} ({})", path.display()));
    Ok(view(window))
}
