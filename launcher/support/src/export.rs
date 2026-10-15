//! Export Logs: one zip with what is needed to look into a problem, and nothing else.
//!
//! The zip is built from **roles**, never from a folder walk: each role names a fixed place and a fixed file-name pattern (the launcher's,
//! build, game and crash logs; the settings; the manifest...), and [`allowed`] checks every entry name again just before it is written.
//! The ISO, the game files, the data folder, the update cache and anything else can therefore not end up in the zip. The save and the
//! screenshots are only added when the user ticked them. Every text goes through the [`Redactor`] (user and machine name), and so does every
//! entry name.

use crate::redact::Redactor;
use crate::{sha256_file, Error, Result};
use serde::Deserialize;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// What the user ticked. The defaults are the design's: the save and the screenshots are off.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Include {
    pub launcher_logs: bool,
    pub build_logs: bool,
    pub game_logs: bool,
    pub system: bool,
    pub settings: bool,
    pub save: bool,
    pub screenshots: bool,
}

impl Default for Include {
    fn default() -> Self {
        Include { launcher_logs: true, build_logs: true, game_logs: true, system: true, settings: true, save: false, screenshots: false }
    }
}

pub struct Request<'a> {
    pub user_dir: &'a Path,
    pub root: &'a Path,
    pub screenshots_dir: &'a Path,
    pub launcher_exe: &'a Path,
    pub launcher_version: &'a str,
    /// The text of `sysinfo::render` (used when `include.system`).
    pub system_text: &'a str,
    pub include: &'a Include,
    pub out_dir: &'a Path,
    pub redactor: &'a Redactor,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Done {
    pub path: String,
    pub size: u64,
    pub entries: Vec<(String, u64)>,
    pub redactions: usize,
    /// Parts that were left out and why (a file that could not be read, a limit).
    pub notes: Vec<String>,
}

const TEXT_CAP: u64 = 8 << 20;
const SHOT_COUNT: usize = 20;
const SHOT_BYTES: u64 = 40 << 20;
const GAME_LOGS: usize = 3;

const FIXED: &[&str] = &[
    "README.txt",
    "system-info.txt",
    "versions.txt",
    "iso-identity.txt",
    "config/settings.toml",
    "config/launcher.json",
    "config/manifest.local.json",
    "config/package.json",
    "save/mcla_save.json",
];

fn plain(n: &str) -> bool {
    !n.is_empty() && n.len() <= 120 && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '~' | ' ' | '(' | ')'))
}

/// The allow-list: the only entry names a support zip may contain.
pub fn allowed(name: &str) -> bool {
    if FIXED.contains(&name) {
        return true;
    }
    let Some((dir, file)) = name.split_once('/') else { return false };
    if file.contains('/') || !plain(file) {
        return false;
    }
    let lower = file.to_ascii_lowercase();
    match dir {
        "logs" => {
            (lower.starts_with("launcher-") && lower.ends_with(".log"))
                || (lower.starts_with("build-") && lower.ends_with(".log"))
                || (lower.starts_with("game-") && lower.ends_with(".log"))
                || (lower.starts_with("crash-") && lower.ends_with(".txt"))
        }
        "screenshots" => lower.ends_with(".png") || lower.ends_with(".jpg") || lower.ends_with(".jpeg"),
        _ => false,
    }
}

struct Item {
    name: String,
    data: Vec<u8>,
}

/// The last `cap` bytes of a file, and whether the start was cut off.
fn read_capped(path: &Path, cap: u64) -> std::io::Result<(Vec<u8>, bool)> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let cut = len > cap;
    if cut {
        f.seek(SeekFrom::Start(len - cap))?;
    }
    let mut v = Vec::with_capacity(len.min(cap) as usize);
    f.take(cap).read_to_end(&mut v)?;
    Ok((v, cut))
}

fn files_in(dir: &Path, keep: impl Fn(&str) -> bool) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<(String, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| e.file_name().to_str().map(|n| (n.to_string(), e.path())))
        .filter(|(n, _)| keep(&n.to_ascii_lowercase()))
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

fn stamp(p: &Path) -> String {
    p.metadata().and_then(|m| m.modified()).ok().map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_else(|| "unknown".into())
}

fn hash_line(label: &str, p: &Path) -> String {
    match (p.metadata(), sha256_file(p)) {
        (Ok(m), Ok(h)) => format!("{label:<22} {} bytes, modified {}, sha256 {h}\n", m.len(), stamp(p)),
        _ => format!("{label:<22} not present\n"),
    }
}

fn iso_identity(root: &Path) -> String {
    let mut s = String::from("Disc image identity (the disc image itself is never part of an export)\n\n");
    let text = std::fs::read_to_string(root.join("manifest.local.json")).ok();
    let v: Option<serde_json::Value> = text.and_then(|t| serde_json::from_str(&t).ok());
    let Some(iso) = v.as_ref().and_then(|v| v.get("iso")) else {
        s.push_str("No install found: manifest.local.json is missing or has no disc entry.\n");
        return s;
    };
    let g = |k: &str| iso.get(k).map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string())).unwrap_or_else(|| "unknown".into());
    s.push_str(&format!("file name      {}\n", g("file_name")));
    s.push_str(&format!("size           {} bytes\n", g("size")));
    s.push_str(&format!("fast check     passed (an install is only recorded after the fast checks: Xbox disc layout, default.xex title id {}, archive key, {} archive entries, partition at {})\n", g("title_id"), g("rpf_entries"), g("partition_base")));
    s.push_str(&format!("region word    {}\n", g("region_mask")));
    match iso.get("sha256").and_then(|x| x.as_str()) {
        Some(h) => s.push_str(&format!("sha-256        {h}\n")),
        None => s.push_str("sha-256        not computed\n"),
    }
    s.push_str(&format!("dump           {}\n", g("dump")));
    if let Some(d) = v.as_ref().and_then(|v| v.get("data_version")) {
        s.push_str(&format!("data version   {d}\n"));
    }
    s
}

fn versions(req: &Request) -> String {
    let root = req.root;
    let user = req.user_dir;
    let mut s = String::from("Versions and file hashes\n\n");
    s.push_str(&format!("created                {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S %z")));
    s.push_str(&format!("launcher version       v{}\n", req.launcher_version));
    s.push_str(&hash_line("launcher exe", req.launcher_exe));
    let pkg = std::fs::read_to_string(root.join("game").join("package.json")).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    match &pkg {
        Some(p) => s.push_str(&format!("game version           v{} (data version {}, source {})\n", p.get("version").and_then(|x| x.as_str()).unwrap_or("?"), p.get("data_version").map(|x| x.to_string()).unwrap_or_else(|| "?".into()), p.get("source").and_then(|x| x.as_str()).unwrap_or("?"))),
        None => s.push_str("game version           not installed (game/package.json is missing)\n"),
    }
    s.push_str(&hash_line("game exe", &root.join("game").join("mcla.exe")));
    s.push_str(&hash_line("manifest.local.json", &root.join("manifest.local.json")));
    s.push_str(&hash_line("settings.toml", &user.join("settings.toml")));
    s.push_str(&hash_line("launcher.json", &user.join("launcher.json")));
    s.push_str(&format!("previous game folder   {}\n", if root.join("game.old").is_dir() { "game.old/ is present" } else { "none" }));
    s.push_str("update cache           not included\n");
    s
}

fn readme(req: &Request, files: &[String]) -> String {
    let mut s = String::from("Rusty MCLA support bundle\n=========================\n\n");
    s.push_str("This zip was made by the launcher's Export Logs. Nothing was sent anywhere: attach it to a GitHub issue yourself.\n");
    s.push_str("Your Windows user name and your computer's name were replaced by <user> and <machine> in every text and file name.\n");
    s.push_str("The game, its data, the disc image and the update cache are never included");
    s.push_str(if req.include.save { ".\n" } else { ", and neither is your save.\n" });
    if req.include.save {
        s.push_str("This zip DOES contain the save file (you ticked it). It holds your progress.\n");
    }
    if req.include.screenshots {
        s.push_str("This zip contains your newest screenshots (you ticked it).\n");
    }
    s.push_str("\nContents:\n");
    for f in files {
        s.push_str(&format!("  {f}\n"));
    }
    s
}

/// Gathers the entries the request asks for. Anything that cannot be read is left out and listed in `notes`.
fn gather(req: &Request, notes: &mut Vec<String>) -> Vec<Item> {
    let inc = req.include;
    let logs = req.user_dir.join("logs");
    let mut items: Vec<Item> = Vec::new();
    let add_file = |items: &mut Vec<Item>, name: String, path: &Path, cap: u64, notes: &mut Vec<String>| match read_capped(path, cap) {
        Ok((mut data, cut)) => {
            if cut {
                let mut v = format!("[the first part of this file was left out: only the last {cap} bytes are included]\n").into_bytes();
                v.append(&mut data);
                data = v;
            }
            items.push(Item { name, data });
        }
        Err(e) => notes.push(format!("{name}: {e}")),
    };
    if inc.launcher_logs {
        for (n, p) in files_in(&logs, |n| n.starts_with("launcher-") && n.ends_with(".log")) {
            add_file(&mut items, format!("logs/{n}"), &p, TEXT_CAP, notes);
        }
    }
    if inc.build_logs {
        for (n, p) in files_in(&logs, |n| n.starts_with("build-") && n.ends_with(".log")) {
            add_file(&mut items, format!("logs/{n}"), &p, TEXT_CAP, notes);
        }
    }
    if inc.game_logs {
        let g = files_in(&logs, |n| n.starts_with("game-") && n.ends_with(".log"));
        for (n, p) in g.into_iter().rev().take(GAME_LOGS).collect::<Vec<_>>().into_iter().rev() {
            add_file(&mut items, format!("logs/{n}"), &p, TEXT_CAP, notes);
        }
        for (n, p) in files_in(&logs, |n| n.starts_with("crash-") && n.ends_with(".txt")) {
            add_file(&mut items, format!("logs/{n}"), &p, TEXT_CAP, notes);
        }
    }
    if inc.system {
        items.push(Item { name: "system-info.txt".into(), data: req.system_text.as_bytes().to_vec() });
    }
    if inc.settings {
        items.push(Item { name: "versions.txt".into(), data: versions(req).into_bytes() });
        items.push(Item { name: "iso-identity.txt".into(), data: iso_identity(req.root).into_bytes() });
        for (name, path) in [
            ("config/settings.toml", req.user_dir.join("settings.toml")),
            ("config/launcher.json", req.user_dir.join("launcher.json")),
            ("config/manifest.local.json", req.root.join("manifest.local.json")),
            ("config/package.json", req.root.join("game").join("package.json")),
        ] {
            if path.is_file() {
                add_file(&mut items, name.into(), &path, TEXT_CAP, notes);
            }
        }
    }
    if inc.save {
        let p = req.user_dir.join("saves").join(mcla_updater::backup::SAVE_FILE);
        if p.is_file() {
            add_file(&mut items, "save/mcla_save.json".into(), &p, 64 << 20, notes);
        } else {
            notes.push("save/mcla_save.json: there is no save yet".into());
        }
    }
    if inc.screenshots {
        let shots = files_in(req.screenshots_dir, |n| n.ends_with(".png") || n.ends_with(".jpg") || n.ends_with(".jpeg"));
        let mut total = 0u64;
        for (n, p) in shots.into_iter().rev().take(SHOT_COUNT) {
            let len = p.metadata().map(|m| m.len()).unwrap_or(0);
            if total + len > SHOT_BYTES {
                notes.push(format!("screenshots/{n}: left out (the screenshots would pass {} MB)", SHOT_BYTES >> 20));
                continue;
            }
            total += len;
            add_file(&mut items, format!("screenshots/{n}"), &p, SHOT_BYTES, notes);
        }
    }
    items
}

fn unique_path(dir: &Path, date: &str) -> PathBuf {
    let mut p = dir.join(format!("RustyMCLA-logs-{date}.zip"));
    let mut n = 1;
    while p.exists() {
        n += 1;
        p = dir.join(format!("RustyMCLA-logs-{date}-{n}.zip"));
    }
    p
}

/// Makes the zip. Redaction is applied to every text and to every entry name; the allow-list is checked on the redacted names.
pub fn export(req: &Request) -> Result<Done> {
    std::fs::create_dir_all(req.out_dir).map_err(|e| Error::io(format!("the export folder {}", req.out_dir.display()), &e))?;
    let mut notes = Vec::new();
    let raw = gather(req, &mut notes);
    let is_text = |name: &str| !name.starts_with("screenshots/");
    let mut redactions = 0usize;
    let mut items: Vec<Item> = Vec::new();
    for it in raw {
        let name = req.redactor.entry_name(&it.name);
        if !allowed(&name) {
            return Err(Error::new("not_allowed", format!("{name:?} is not on the list of files an export may contain")));
        }
        let data = if is_text(&it.name) {
            let (t, n) = req.redactor.text_counted(&String::from_utf8_lossy(&it.data));
            redactions += n;
            t.into_bytes()
        } else {
            it.data
        };
        items.push(Item { name, data });
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    let mut names: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
    names.push("README.txt".into());
    names.sort();
    let (readme_text, n) = req.redactor.text_counted(&readme(req, &names));
    redactions += n;
    items.insert(0, Item { name: "README.txt".into(), data: readme_text.into_bytes() });

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let dest = unique_path(req.out_dir, &date);
    let part = dest.with_extension("zip.part");
    let entries = write_zip(&part, &items).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        e
    })?;
    std::fs::rename(&part, &dest).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        Error::io(format!("finish {}", dest.display()), &e)
    })?;
    let size = dest.metadata().map(|m| m.len()).unwrap_or(0);
    log::info!("export written: {} ({size} bytes, {} files, {redactions} names redacted)", dest.display(), entries.len());
    Ok(Done { path: dest.to_string_lossy().into_owned(), size, entries, redactions, notes })
}

fn write_zip(path: &Path, items: &[Item]) -> Result<Vec<(String, u64)>> {
    let f = std::fs::File::create(path).map_err(|e| Error::io(path.display(), &e))?;
    let mut z = zip::ZipWriter::new(std::io::BufWriter::new(f));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut out = Vec::new();
    for it in items {
        if !allowed(&it.name) {
            return Err(Error::new("not_allowed", format!("{:?} is not on the list of files an export may contain", it.name)));
        }
        z.start_file(it.name.as_str(), opts).map_err(|e| Error::new("io", format!("zip: {e}")))?;
        z.write_all(&it.data).map_err(|e| Error::io("zip", &e))?;
        out.push((it.name.clone(), it.data.len() as u64));
    }
    let mut w = z.finish().map_err(|e| Error::new("io", format!("zip: {e}")))?;
    w.flush().map_err(|e| Error::io("zip", &e))?;
    Ok(out)
}

/// Where an export goes by default: the Desktop, else the home folder, else the user folder.
pub fn default_dir(user_dir: &Path) -> PathBuf {
    dirs::desktop_dir().filter(|p| p.is_dir()).or_else(dirs::home_dir).unwrap_or_else(|| user_dir.to_path_buf())
}

/// The "new issue" link of the repository with a title and a body that carry the version numbers. The zip is attached by the user
/// (GitHub cannot take an upload through a link). Kept well under the 2048 characters `open_url` accepts.
pub fn issue_url(issues_base: &str, launcher: &str, game: Option<&str>, os: &str, data_version: Option<u32>, redactor: &Redactor) -> Result<String> {
    let mut u = url::Url::parse(issues_base).map_err(|e| Error::new("url", format!("{issues_base}: {e}")))?;
    let game_s = game.map(|g| format!("v{g}")).unwrap_or_else(|| "not installed".into());
    let title = format!("[launcher v{launcher}, game {game_s}] ");
    let os = redactor.text(os);
    let body = format!(
        "**What happened**\n(Describe what you did and what you expected.)\n\n**Versions**\n- Launcher: v{launcher}\n- Game: {game_s}\n- Data version: {}\n- Windows: {os}\n\n**Logs**\nDrag the zip from the launcher's Export Logs into this issue.\nPlease do not post or ask for game files or disc images.\n",
        data_version.map(|d| d.to_string()).unwrap_or_else(|| "unknown".into())
    );
    u.query_pairs_mut().append_pair("title", &title).append_pair("body", &body);
    let s = u.to_string();
    if s.len() > 1900 {
        return Err(Error::new("url", "the issue link would be too long"));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    struct Fix {
        _d: tempfile::TempDir,
        user: PathBuf,
        root: PathBuf,
        shots: PathBuf,
        out: PathBuf,
    }

    fn w(p: &Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    /// A fake user profile "Zelda Quinn" on "QUINN-PC" with real-looking files and a lot of things that must stay out.
    fn fixture() -> Fix {
        let d = tempfile::tempdir().unwrap();
        let (user, root) = (d.path().join("user"), d.path().join("app"));
        let logs = user.join("logs");
        for t in ["2026-10-15_09-00-00", "2026-10-15_10-00-00"] {
            w(&logs.join(format!("launcher-{t}.log")), &format!("[{t}] INFO portable root C:\\Users\\Zelda Quinn\\Desktop\\Rusty MCLA\n[x] INFO zelda quinn opened QUINN-PC\n"));
        }
        w(&logs.join("build-20261015-091000.log"), "build in C:\\Users\\Zelda Quinn\\Desktop\\Rusty MCLA\\data\n");
        for i in 1..=5 {
            w(&logs.join(format!("game-2026101{i}-120000.log")), &format!("game log {i}: save at C:\\Users\\Zelda Quinn\\AppData\\Local\\RustyMCLA\\saves\n"));
        }
        w(&logs.join("crash-20261015-130000.txt"), "panic at C:\\Users\\Zelda Quinn\\x\\src\\main.rs\n");
        // decoys inside the logs folder
        w(&logs.join("secret.iso"), "ISO");
        w(&logs.join("notes.txt"), "private notes");
        w(&logs.join("passwords.txt"), "x");
        w(&logs.join("launcher-evil.exe"), "MZ");
        w(&user.join("cache").join("latest.json"), "{\"etag\":\"x\"}");
        w(&user.join("settings.toml"), "schema = 1\n[paths]\ndata_dir = \"C:/Users/Zelda Quinn/data\"\n");
        w(&user.join("launcher.json"), "{\"schema\":1,\"theme\":\"steel\",\"export_dir\":\"C:\\\\Users\\\\Zelda Quinn\\\\Desktop\"}");
        w(&user.join("saves").join("mcla_save.json"), "{\"car\":\"x\",\"money\":5,\"name\":\"Zelda Quinn\"}");
        w(&user.join("backups").join("mcla_save-20261015-131029-manual.json"), "{}");
        w(&root.join("manifest.local.json"), "{\"schema\":1,\"data_version\":1,\"iso\":{\"file_name\":\"Zelda Quinn's disc.iso\",\"size\":7838695424,\"partition_base\":265330688,\"title_id\":\"545407F8\",\"region_mask\":\"FFFFFFFF\",\"rpf_entries\":23931,\"sha256\":\"877a9415\",\"dump\":\"verified\"}}");
        w(&root.join("game").join("package.json"), "{\"version\":\"0.2.0\",\"data_version\":1,\"source\":\"download\"}");
        w(&root.join("game").join("mcla.exe"), "MZ game");
        w(&root.join("data").join("cache").join("big.bin"), "DATA");
        w(&root.join("disc.iso"), "ISO");
        let shots = root.join("screenshots");
        w(&shots.join("game-20261015-120000.png"), "PNG1");
        w(&shots.join("game-20261015-120100.png"), "PNG2");
        w(&shots.join("notes.txt"), "no");
        let launcher = root.join("RustyMCLA.exe");
        w(&launcher, "MZ launcher");
        Fix { out: d.path().join("out"), _d: d, user, root, shots }
    }

    fn run(f: &Fix, inc: &Include) -> Done {
        let red = Redactor::new(&["Zelda Quinn"], &["QUINN-PC"]);
        let exe = f.root.join("RustyMCLA.exe");
        export(&Request { user_dir: &f.user, root: &f.root, screenshots_dir: &f.shots, launcher_exe: &exe, launcher_version: "0.1.0", system_text: "Windows on QUINN-PC, user Zelda Quinn\n", include: inc, out_dir: &f.out, redactor: &red }).unwrap()
    }

    fn open(path: &str) -> zip::ZipArchive<std::fs::File> {
        zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap()
    }

    fn read(z: &mut zip::ZipArchive<std::fs::File>, name: &str) -> String {
        let mut s = String::new();
        z.by_name(name).unwrap().read_to_string(&mut s).unwrap();
        s
    }

    #[test]
    fn default_export_contains_only_the_allowed_roles() {
        let f = fixture();
        let done = run(&f, &Include::default());
        let mut z = open(&done.path);
        let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
        assert!(done.path.contains("RustyMCLA-logs-") && done.path.ends_with(".zip"), "{}", done.path);
        for n in &names {
            assert!(allowed(n), "{n} is not allowed");
        }
        let want = [
            "README.txt", "system-info.txt", "versions.txt", "iso-identity.txt", "config/settings.toml", "config/launcher.json", "config/manifest.local.json", "config/package.json",
            "logs/launcher-2026-10-15_09-00-00.log", "logs/launcher-2026-10-15_10-00-00.log", "logs/build-20261015-091000.log", "logs/crash-20261015-130000.txt",
            "logs/game-20261013-120000.log", "logs/game-20261014-120000.log", "logs/game-20261015-120000.log",
        ];
        let mut sorted = names.clone();
        sorted.sort();
        let mut want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
        want.sort();
        assert_eq!(sorted, want);
        // the last three game logs only (11 and 12 are older), no decoys, no save, no screenshots, no cache
        assert!(!names.iter().any(|n| n.contains("secret") || n.contains("notes") || n.contains("passwords") || n.contains("evil") || n.contains("latest") || n.starts_with("save/") || n.starts_with("screenshots/") || n.contains(".iso") || n.contains("mcla.exe") || n.contains("big.bin")));
        assert!(done.size > 0 && done.size < 20_000);
    }

    #[test]
    fn nothing_of_the_profile_survives_in_any_file() {
        let f = fixture();
        let inc = Include { save: true, screenshots: true, ..Include::default() };
        let done = run(&f, &inc);
        let mut z = open(&done.path);
        for i in 0..z.len() {
            let name = z.by_index(i).unwrap().name().to_string();
            assert!(!name.to_lowercase().contains("zelda") && !name.to_lowercase().contains("quinn"), "{name}");
            if name.ends_with(".png") {
                continue;
            }
            let t = read(&mut z, &name);
            let l = t.to_lowercase();
            assert!(!l.contains("zelda") && !l.contains("quinn"), "{name} still has the profile:\n{t}");
        }
        assert!(read(&mut z, "logs/launcher-2026-10-15_09-00-00.log").contains(r"C:\Users\<user>\Desktop"));
        assert!(read(&mut z, "config/launcher.json").contains("Users\\\\<user>\\\\Desktop"));
        assert!(read(&mut z, "config/settings.toml").contains("C:/Users/<user>/data"));
        assert!(read(&mut z, "iso-identity.txt").contains("<user>'s disc.iso"));
        assert!(done.redactions >= 10, "{}", done.redactions);
    }

    #[test]
    fn save_and_screenshots_only_when_ticked() {
        let f = fixture();
        let off = run(&f, &Include::default());
        assert!(!off.entries.iter().any(|(n, _)| n.starts_with("save/") || n.starts_with("screenshots/")));
        let on = run(&f, &Include { save: true, screenshots: true, ..Include::default() });
        let names: Vec<&str> = on.entries.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"save/mcla_save.json"));
        assert!(names.contains(&"screenshots/game-20261015-120000.png") && names.contains(&"screenshots/game-20261015-120100.png"));
        assert!(!names.iter().any(|n| n.contains("notes")));
        let mut z = open(&on.path);
        assert!(read(&mut z, "README.txt").contains("DOES contain the save"));
        // a second export the same day never overwrites the first
        assert_ne!(off.path, on.path);
        assert!(Path::new(&off.path).is_file() && Path::new(&on.path).is_file());
    }

    #[test]
    fn unticked_parts_are_left_out() {
        let f = fixture();
        let inc = Include { launcher_logs: false, build_logs: false, game_logs: false, system: false, settings: false, save: false, screenshots: false };
        let done = run(&f, &inc);
        let names: Vec<&str> = done.entries.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["README.txt"]);
    }

    #[test]
    fn the_allow_list_refuses_everything_else() {
        for ok in ["logs/game-1.log", "logs/crash-1.txt", "save/mcla_save.json", "screenshots/a.png", "system-info.txt"] {
            assert!(allowed(ok), "{ok}");
        }
        for bad in ["", "disc.iso", "game/mcla.exe", "data/cache/x", "logs/secret.iso", "logs/notes.txt", "logs/../x", "logs/a/b.log", "../logs/game-1.log", "/logs/game-1.log", "logs\\game-1.log", "cache/latest.json", "save/other.json", "screenshots/a.txt", "config/secret.toml", "logs/game-<user>.log", "RustyMCLA.exe", "config/../../x", "logs/launcher-1.exe"] {
            assert!(!allowed(bad), "{bad:?} must not be allowed");
        }
        let items = [Item { name: "game/mcla.exe".into(), data: vec![1] }];
        let d = tempfile::tempdir().unwrap();
        let e = write_zip(&d.path().join("a.zip"), &items).unwrap_err();
        assert_eq!(e.code, "not_allowed");
    }

    #[test]
    fn a_huge_log_is_cut_from_the_front_and_says_so() {
        let f = fixture();
        let big = "x".repeat(9 << 20);
        w(&f.user.join("logs").join("launcher-2026-10-16_00-00-00.log"), &format!("START{big}END"));
        let done = run(&f, &Include::default());
        let mut z = open(&done.path);
        let t = read(&mut z, "logs/launcher-2026-10-16_00-00-00.log");
        assert!(t.starts_with("[the first part") && t.ends_with("END") && !t.contains("START"));
        assert!(t.len() < (8 << 20) + 200);
    }

    #[test]
    fn issue_link_carries_versions_and_stays_short() {
        let red = Redactor::new(&["Zelda Quinn"], &[""] as &[&str]);
        let u = issue_url("https://github.com/OWNER/rusty-mcla/issues/new", "0.1.0", Some("0.2.0"), "Windows 11 (user Zelda Quinn)", Some(1), &red).unwrap();
        assert!(u.starts_with("https://github.com/OWNER/rusty-mcla/issues/new?title="), "{u}");
        let parsed = url::Url::parse(&u).unwrap();
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert!(q["title"].contains("launcher v0.1.0") && q["title"].contains("game v0.2.0"));
        assert!(q["body"].contains("- Launcher: v0.1.0") && q["body"].contains("- Game: v0.2.0") && q["body"].contains("<user>") && !q["body"].contains("Zelda"));
        assert!(u.len() < 1900, "{}", u.len());
        assert!(issue_url("not a url", "0", None, "", None, &red).is_err());
    }
}
