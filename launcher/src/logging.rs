//! `%LOCALAPPDATA%\RustyMCLA\logs\launcher-<time>.log`: one file per start, the newest `KEEP` are kept.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const KEEP: usize = 10;

struct Log {
    file: Mutex<Option<File>>,
    path: PathBuf,
}

static LOG: OnceLock<Log> = OnceLock::new();

/// Creates the log file for this run and removes old ones. A failure to open it is not fatal: logging then does nothing.
pub fn init(logs_dir: &Path) -> PathBuf {
    let _ = fs::create_dir_all(logs_dir);
    prune(logs_dir);
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let path = logs_dir.join(format!("launcher-{stamp}.log"));
    let file = OpenOptions::new().create(true).append(true).open(&path).ok();
    let _ = LOG.set(Log { file: Mutex::new(file), path: path.clone() });
    path
}

pub fn path() -> Option<PathBuf> {
    LOG.get().map(|l| l.path.clone())
}

pub fn write(level: &str, msg: &str) {
    let Some(log) = LOG.get() else { return };
    let Ok(mut guard) = log.file.lock() else { return };
    if let Some(f) = guard.as_mut() {
        let t = chrono::Local::now().format("%H:%M:%S%.3f");
        let _ = writeln!(f, "[{t}] {level:<5} {}", msg.replace('\n', " | "));
    }
}

/// Lets the `log` crate users (the update library, warnings of the install library) write into the launcher log.
struct Bridge;

impl log::Log for Bridge {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn || (m.level() <= log::Level::Info && m.target().starts_with("mcla_updater"))
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            let level = match r.level() {
                log::Level::Error => "ERROR",
                log::Level::Warn => "WARN",
                _ => "INFO",
            };
            write(level, &format!("{}", r.args()));
        }
    }
    fn flush(&self) {}
}

static BRIDGE: Bridge = Bridge;

pub fn install_bridge() {
    if log::set_logger(&BRIDGE).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

pub fn info(msg: &str) {
    write("INFO", msg)
}
pub fn warn(msg: &str) {
    write("WARN", msg)
}
pub fn error(msg: &str) {
    write("ERROR", msg)
}

fn prune(dir: &Path) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    let mut logs: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            n.starts_with("launcher-") && n.ends_with(".log")
        })
        .collect();
    logs.sort(); // the timestamp in the name sorts oldest first
    while logs.len() >= KEEP {
        let _ = fs::remove_file(logs.remove(0));
    }
}
