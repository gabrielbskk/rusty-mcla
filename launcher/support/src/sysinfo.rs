//! The system information text of an export, and the short summary the Support tab shows.
//!
//! Everything is read without administrator rights and without sending anything anywhere: the registry through `reg query`, one
//! PowerShell call for memory and graphics (WMI), and a look at which graphics DLLs exist. Each part that cannot be read says so instead of
//! failing. The result is redacted by the caller, like every other text of the export.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

/// What only the running launcher knows.
#[derive(Clone, Debug, Default)]
pub struct Context {
    pub launcher_version: String,
    pub game_version: Option<String>,
    /// From the web view (`tauri::webview_version`).
    pub webview: Option<String>,
    /// One line per display, from the window's monitor list.
    pub displays: Vec<String>,
    pub root: PathBuf,
    pub user_dir: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct Hardware {
    pub os: String,
    pub cpu: String,
    pub logical_cores: usize,
    pub ram_bytes: Option<u64>,
    /// (name, driver version, driver date)
    pub gpus: Vec<(String, String, String)>,
    pub vulkan_loader: Option<String>,
    pub graphics_dlls: Vec<(String, bool)>,
    pub notes: Vec<String>,
}

/// The short lines of the Support tab's "Your system" card.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Summary {
    pub launcher: String,
    pub game: Option<String>,
    pub windows: String,
    pub graphics: String,
    pub webview: String,
}

#[cfg(windows)]
fn reg_values(key: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(o) = crate::quiet_command("reg").args(["query", key]).output() else { return out };
    for line in String::from_utf8_lossy(&o.stdout).lines() {
        // "    ProductName    REG_SZ    Windows 11 Pro"
        let mut it = line.trim().splitn(3, "    ");
        if let (Some(name), Some(kind), Some(value)) = (it.next(), it.next(), it.next()) {
            if kind.starts_with("REG_") {
                out.insert(name.trim().to_string(), value.trim().to_string());
            }
        }
    }
    out
}

#[cfg(windows)]
fn read_os() -> String {
    let v = reg_values(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    let get = |k: &str| v.get(k).cloned().unwrap_or_default();
    let ubr = v.get("UBR").and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).ok());
    let mut s = get("ProductName");
    // "ProductName" still says Windows 10 on Windows 11; the build number tells them apart
    let build: u32 = get("CurrentBuildNumber").parse().unwrap_or(0);
    if build >= 22000 && s.contains("Windows 10") {
        s = s.replace("Windows 10", "Windows 11");
    }
    let disp = get("DisplayVersion");
    if !disp.is_empty() {
        s.push_str(&format!(" {disp}"));
    }
    s.push_str(&format!(" (build {}{})", build, ubr.map(|u| format!(".{u}")).unwrap_or_default()));
    s.push_str(&format!(", {}", std::env::consts::ARCH));
    s
}

#[cfg(not(windows))]
fn read_os() -> String {
    let t = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let name = t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=")).map(|v| v.trim_matches('"').to_string()).unwrap_or_else(|| std::env::consts::OS.to_string());
    format!("{name}, {}", std::env::consts::ARCH)
}

#[cfg(windows)]
fn read_cpu() -> String {
    reg_values(r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0").get("ProcessorNameString").cloned().unwrap_or_default()
}

#[cfg(not(windows))]
fn read_cpu() -> String {
    let t = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    t.lines().find_map(|l| l.strip_prefix("model name").and_then(|r| r.split_once(':')).map(|(_, v)| v.trim().to_string())).unwrap_or_default()
}

/// Runs PowerShell with a time limit. `None` when it is missing, fails or takes too long.
#[cfg(windows)]
fn powershell(script: &str, limit: Duration) -> Option<String> {
    use std::io::Read;
    let mut child = crate::quiet_command("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = stdout.read_to_end(&mut s);
        s
    });
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => {
                let bytes = reader.join().ok()?;
                return st.success().then(|| String::from_utf8_lossy(&bytes).into_owned());
            }
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(40)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(windows)]
const PS_SCRIPT: &str = r#"$ErrorActionPreference='SilentlyContinue'
$o=@{}
$o.ram=(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory
$o.gpus=@(Get-CimInstance Win32_VideoController | ForEach-Object { @{ name=$_.Name; driver=$_.DriverVersion; date=("{0:yyyy-MM-dd}" -f $_.DriverDate) } })
$o.vulkan=(Get-Item "$env:windir\System32\vulkan-1.dll").VersionInfo.ProductVersion
$o | ConvertTo-Json -Compress -Depth 4"#;

#[cfg(windows)]
fn read_graphics(h: &mut Hardware) {
    let Some(text) = powershell(PS_SCRIPT, Duration::from_secs(20)) else {
        h.notes.push("memory and graphics could not be read (PowerShell was not available or took too long)".into());
        return;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text.trim()) else {
        h.notes.push("memory and graphics: the answer could not be read".into());
        return;
    };
    h.ram_bytes = v.get("ram").and_then(|r| r.as_u64().or_else(|| r.as_f64().map(|f| f as u64)));
    let list: Vec<serde_json::Value> = match v.get("gpus") {
        Some(serde_json::Value::Array(a)) => a.clone(),
        Some(o @ serde_json::Value::Object(_)) => vec![o.clone()],
        _ => Vec::new(),
    };
    for g in list {
        let s = |k: &str| g.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        h.gpus.push((s("name"), s("driver"), s("date")));
    }
    h.vulkan_loader = v.get("vulkan").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(String::from);
}

#[cfg(not(windows))]
fn read_graphics(h: &mut Hardware) {
    h.notes.push("graphics are not read on this system".into());
}

fn graphics_dlls() -> Vec<(String, bool)> {
    #[cfg(windows)]
    {
        let dir = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("System32");
        ["vulkan-1.dll", "d3d12.dll", "d3d11.dll", "dxgi.dll"].iter().map(|n| (n.to_string(), dir.join(n).is_file())).collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Reads the machine once. Takes about a second on Windows (the WMI call).
pub fn hardware() -> Hardware {
    let mut h = Hardware { os: read_os(), cpu: read_cpu(), logical_cores: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0), graphics_dlls: graphics_dlls(), ..Default::default() };
    read_graphics(&mut h);
    h
}

fn gb(b: u64) -> String {
    format!("{:.1} GB", b as f64 / 1_073_741_824.0)
}

pub fn summary(h: &Hardware, ctx: &Context) -> Summary {
    let graphics = match h.gpus.first() {
        Some((n, d, _)) if !d.is_empty() => format!("{n}, driver {d}"),
        Some((n, _, _)) => n.clone(),
        None => String::new(),
    };
    Summary { launcher: format!("v{}", ctx.launcher_version), game: ctx.game_version.as_ref().map(|v| format!("v{v}")), windows: h.os.clone(), graphics, webview: ctx.webview.clone().unwrap_or_default() }
}

/// The text file of the export.
pub fn render(h: &Hardware, ctx: &Context) -> String {
    let mut s = String::new();
    s.push_str("System information (read without administrator rights; nothing here is sent anywhere)\n\n");
    s.push_str(&format!("Launcher        v{}\n", ctx.launcher_version));
    s.push_str(&format!("Game            {}\n", ctx.game_version.as_ref().map(|v| format!("v{v}")).unwrap_or_else(|| "not installed".into())));
    s.push_str(&format!("Web view        {}\n", ctx.webview.as_deref().unwrap_or("unknown")));
    s.push_str(&format!("Windows         {}\n", h.os));
    s.push_str(&format!("CPU             {} ({} logical cores)\n", if h.cpu.is_empty() { "unknown" } else { &h.cpu }, h.logical_cores));
    s.push_str(&format!("Memory          {}\n", h.ram_bytes.map(gb).unwrap_or_else(|| "unknown".into())));
    if h.gpus.is_empty() {
        s.push_str("Graphics        unknown\n");
    }
    for (i, (name, driver, date)) in h.gpus.iter().enumerate() {
        s.push_str(&format!("Graphics {}      {} | driver {} {}\n", i + 1, name, if driver.is_empty() { "unknown" } else { driver }, date));
    }
    s.push_str("\nGraphics interfaces (files present in the system folder)\n");
    for (n, present) in &h.graphics_dlls {
        let extra = if n == "vulkan-1.dll" { h.vulkan_loader.as_ref().map(|v| format!(" (loader {v})")).unwrap_or_default() } else { String::new() };
        s.push_str(&format!("  {:<14} {}{}\n", n, if *present { "present" } else { "missing" }, extra));
    }
    s.push_str("  (the game's own log, game-*.log, names the adapter and backend it actually used)\n");
    s.push_str("\nDisplays\n");
    if ctx.displays.is_empty() {
        s.push_str("  unknown\n");
    }
    for d in &ctx.displays {
        s.push_str(&format!("  {d}\n"));
    }
    s.push_str("\nFolders\n");
    s.push_str(&format!("  launcher folder  {}\n  user folder      {}\n", ctx.root.display(), ctx.user_dir.display()));
    for n in &h.notes {
        s.push_str(&format!("\nNote: {n}\n"));
    }
    s
}

/// Small helper for tests and the page: a key/value view of a Summary.
pub fn as_map(s: &Summary) -> BTreeMap<&'static str, String> {
    let mut m = BTreeMap::new();
    m.insert("launcher", s.launcher.clone());
    m.insert("game", s.game.clone().unwrap_or_default());
    m.insert("windows", s.windows.clone());
    m.insert("graphics", s.graphics.clone());
    m.insert("webview", s.webview.clone());
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_names_every_part_and_survives_missing_data() {
        let h = Hardware { os: "Windows 11 Pro 25H2 (build 26200.1)".into(), cpu: "Test CPU".into(), logical_cores: 8, ram_bytes: Some(17_179_869_184), gpus: vec![("GPU A".into(), "1.2.3".into(), "2026-01-01".into())], vulkan_loader: Some("1.4.0.0".into()), graphics_dlls: vec![("vulkan-1.dll".into(), true), ("d3d12.dll".into(), false)], notes: vec![] };
        let ctx = Context { launcher_version: "0.1.0".into(), game_version: Some("0.2.0".into()), webview: Some("155.0".into()), displays: vec!["1. DISPLAY1 1920x1080 (main)".into()], ..Default::default() };
        let t = render(&h, &ctx);
        for want in ["Windows 11 Pro", "Test CPU (8 logical cores)", "16.0 GB", "GPU A | driver 1.2.3", "vulkan-1.dll", "(loader 1.4.0.0)", "d3d12.dll", "missing", "1. DISPLAY1 1920x1080", "155.0", "v0.2.0"] {
            assert!(t.contains(want), "missing {want:?} in\n{t}");
        }
        let empty = render(&Hardware::default(), &Context::default());
        assert!(empty.contains("Graphics        unknown") && empty.contains("not installed"));
        let s = summary(&h, &ctx);
        assert_eq!(s.graphics, "GPU A, driver 1.2.3");
        assert_eq!(as_map(&s)["webview"], "155.0");
    }

    #[test]
    #[cfg(windows)]
    fn this_machine_reports_an_os_and_a_cpu() {
        let h = hardware();
        assert!(h.os.contains("Windows") && h.os.contains("build"), "{}", h.os);
        assert!(!h.cpu.is_empty());
        assert!(h.logical_cores > 0);
        eprintln!("{}", render(&h, &Context::default()));
    }
}
