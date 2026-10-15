//! Starting the game, noticing that it died at once, and remembering a crash for the next launcher start.
//!
//! The launcher never asks the game exe anything (`--help` and unknown flags boot the game): it only starts it with the flags the game
//! documents (`--data`, and `--save-dir` / `--settings` when the launcher's user folder is not the default), and watches the process.

use crate::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

/// How long a start has to survive before it counts as started.
pub const WATCH: Duration = Duration::from_secs(4);

#[derive(Debug, Clone, serde::Serialize)]
pub struct Plan {
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub data: PathBuf,
}

pub fn game_exe(root: &Path) -> PathBuf {
    root.join("game").join(if cfg!(windows) { "mcla.exe" } else { "mcla" })
}

/// True when the user folder is not `%LOCALAPPDATA%\RustyMCLA` (a test run or a moved folder): then the game gets the flags to find the same one.
fn user_dir_is_custom(user_dir: &Path) -> bool {
    match dirs::data_local_dir() {
        Some(d) => d.join("RustyMCLA") != user_dir,
        None => true,
    }
}

/// The data folder the game would pick itself (`MCLA_DATA`, `paths.data_dir` of `settings.toml`, else beside the game as `mcla_config::paths` decides).
pub fn data_folder(root: &Path, user_dir: &Path) -> PathBuf {
    let cwd = game_exe(root).parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
    let settings = mcla_config::Settings::load(&user_dir.join("settings.toml")).settings;
    let env_data = std::env::var("MCLA_DATA").ok();
    mcla_config::paths::resolve_data_dir(&mcla_config::paths::DataInputs { flag: None, env: env_data.as_deref(), settings: &settings.paths.data_dir, exe_dir: &cwd, dev_fallback: None }).path
}

/// The command line for a normal start. `Err("missing_exe")` / `Err("missing_data")` when the game or its data are not there.
pub fn plan(root: &Path, user_dir: &Path) -> Result<Plan> {
    let exe = game_exe(root);
    if !exe.is_file() {
        return Err(Error::new("missing_exe", format!("{} does not exist", exe.display())));
    }
    let cwd = exe.parent().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
    // the data folder the game would pick itself, passed explicitly so the game never has to guess where it was started from
    let data = data_folder(root, user_dir);
    if !data.is_dir() {
        return Err(Error::new("missing_data", format!("the data folder {} does not exist", data.display())));
    }
    let mut args = vec!["--data".to_string(), data.to_string_lossy().into_owned()];
    if user_dir_is_custom(user_dir) {
        args.push("--save-dir".into());
        args.push(user_dir.join("saves").to_string_lossy().into_owned());
        args.push("--settings".into());
        args.push(user_dir.join("settings.toml").to_string_lossy().into_owned());
    }
    Ok(Plan { exe, args, cwd, data })
}

pub fn spawn(plan: &Plan) -> Result<Child> {
    let mut c = std::process::Command::new(&plan.exe);
    c.args(&plan.args).current_dir(&plan.cwd).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP: it outlives the launcher
    }
    c.spawn().map_err(|e| Error::io(format!("start {}", plan.exe.display()), &e))
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Outcome {
    /// Still running when the watch ended.
    Running,
    /// Ended inside the watch. `code` is `None` when the system gave none (killed).
    Exited { code: Option<i32>, millis: u128 },
}

/// Polls the child for `window`; returns early when it ends.
pub fn watch(child: &mut Child, window: Duration) -> Outcome {
    let t0 = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return Outcome::Exited { code: st.code(), millis: t0.elapsed().as_millis() },
            Ok(None) if t0.elapsed() < window => std::thread::sleep(Duration::from_millis(40)),
            Ok(None) => return Outcome::Running,
            Err(_) => return Outcome::Running, // cannot tell: do not call it a failure
        }
    }
}

// ---------- the record of the last start ----------

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct LastRun {
    /// Unix seconds of the last start from the launcher.
    pub started: i64,
    pub pid: u32,
    /// Exit code, when the launcher stayed open and saw the game end.
    pub code: Option<i32>,
    /// Crash reports at or before this time (unix seconds) have been shown and dismissed.
    pub acknowledged_until: i64,
}

fn file(user_dir: &Path) -> PathBuf {
    user_dir.join("last_run.json")
}

pub fn read_last(user_dir: &Path) -> LastRun {
    std::fs::read_to_string(file(user_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write_last(user_dir: &Path, r: &LastRun) {
    if let Ok(t) = serde_json::to_vec_pretty(r) {
        if let Err(e) = mcla_updater::write_atomic(&file(user_dir), &t) {
            log::warn!("could not write last_run.json: {e}");
        }
    }
}

pub fn record_start(user_dir: &Path, pid: u32) {
    let mut r = read_last(user_dir);
    r.started = mcla_updater::now_unix();
    r.pid = pid;
    r.code = None;
    write_last(user_dir, &r);
}

pub fn record_exit(user_dir: &Path, pid: u32, code: Option<i32>) {
    let mut r = read_last(user_dir);
    if r.pid == pid {
        r.code = Some(code.unwrap_or(-1));
        write_last(user_dir, &r);
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CrashNotice {
    /// File name of the newest crash report, if there is one.
    pub report: Option<String>,
    pub code: Option<i32>,
    pub when: i64,
}

fn mtime_unix(p: &Path) -> i64 {
    p.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The crash the user has not been told about: the last observed exit code was an error, or a crash report newer than the last start
/// (minus a little slack) and than the last dismissal exists.
pub fn pending_crash(user_dir: &Path) -> Option<CrashNotice> {
    let last = read_last(user_dir);
    let floor = last.acknowledged_until.max(last.started - 2);
    let newest = std::fs::read_dir(user_dir.join("logs"))
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            n.starts_with("crash-") && n.ends_with(".txt")
        })
        .map(|e| (mtime_unix(&e.path()), e.file_name().to_string_lossy().into_owned()))
        .filter(|(t, _)| *t > floor)
        .max();
    let code_bad = last.code.map(|c| c != 0).unwrap_or(false) && last.started > last.acknowledged_until;
    if newest.is_none() && !code_bad {
        return None;
    }
    Some(CrashNotice { when: newest.as_ref().map(|n| n.0).unwrap_or(last.started), report: newest.map(|n| n.1), code: last.code })
}

/// Dismiss: remembers that everything up to now has been seen.
pub fn dismiss_crash(user_dir: &Path) {
    let mut r = read_last(user_dir);
    r.acknowledged_until = mcla_updater::now_unix().max(r.started);
    write_last(user_dir, &r);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stage() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let (root, user) = (d.path().join("app"), d.path().join("user"));
        std::fs::create_dir_all(root.join("game")).unwrap();
        std::fs::create_dir_all(root.join("data").join("cache")).unwrap();
        std::fs::create_dir_all(user.join("logs")).unwrap();
        std::fs::write(game_exe(&root), "MZ").unwrap();
        (d, root, user)
    }

    #[test]
    fn plan_passes_the_data_folder_and_custom_user_folders() {
        let (_d, root, user) = stage();
        let p = plan(&root, &user).unwrap();
        assert_eq!(p.args[0], "--data");
        assert!(PathBuf::from(&p.args[1]).ends_with(Path::new("data").join("cache")), "{:?}", p.args);
        // a scratch user folder is not %LOCALAPPDATA%\RustyMCLA, so the game is told where the save and the settings are
        assert!(p.args.iter().any(|a| a == "--save-dir") && p.args.iter().any(|a| a == "--settings"));
        assert_eq!(p.cwd, root.join("game"));
        assert!(!p.args.iter().any(|a| a == "--help" || a == "--version"));
    }

    #[test]
    fn plan_reports_what_is_missing() {
        let (_d, root, user) = stage();
        std::fs::remove_file(game_exe(&root)).unwrap();
        assert_eq!(plan(&root, &user).unwrap_err().code, "missing_exe");
        std::fs::write(game_exe(&root), "MZ").unwrap();
        std::fs::remove_dir_all(root.join("data")).unwrap();
        assert_eq!(plan(&root, &user).unwrap_err().code, "missing_data");
    }

    #[test]
    fn settings_data_dir_is_respected() {
        let (d, root, user) = stage();
        let other = d.path().join("elsewhere");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(user.join("settings.toml"), format!("schema = 1\n[paths]\ndata_dir = '{}'\n", other.display())).unwrap();
        let p = plan(&root, &user).unwrap();
        assert_eq!(PathBuf::from(&p.args[1]), other);
    }

    #[test]
    #[cfg(windows)]
    fn watch_tells_a_quick_failure_from_a_running_game() {
        let mut quick = std::process::Command::new("cmd").args(["/c", "exit 3"]).spawn().unwrap();
        match watch(&mut quick, Duration::from_secs(5)) {
            Outcome::Exited { code, millis } => assert!(code == Some(3) && millis < 4000, "{code:?} {millis}"),
            o => panic!("{o:?}"),
        }
        let mut slow = std::process::Command::new("ping").args(["-n", "8", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        assert_eq!(watch(&mut slow, Duration::from_millis(600)), Outcome::Running);
        let _ = slow.kill();
        let _ = slow.wait();
    }

    #[test]
    fn a_crash_report_after_the_last_start_is_shown_once() {
        let (_d, _root, user) = stage();
        assert!(pending_crash(&user).is_none());
        record_start(&user, 111);
        assert!(pending_crash(&user).is_none(), "a start alone is not a crash");
        std::fs::write(user.join("logs").join("crash-20261015-130000.txt"), "panic").unwrap();
        let n = pending_crash(&user).expect("the report is newer than the start");
        assert_eq!(n.report.as_deref(), Some("crash-20261015-130000.txt"));
        dismiss_crash(&user);
        assert!(pending_crash(&user).is_none(), "dismissed");
        // a later report shows again
        std::thread::sleep(Duration::from_millis(1100));
        std::fs::write(user.join("logs").join("crash-20261015-140000.txt"), "panic").unwrap();
        // (file times have whole-second precision here; make the second report clearly newer than the dismissal)
        let f = std::fs::File::options().write(true).open(user.join("logs").join("crash-20261015-140000.txt")).unwrap();
        f.set_modified(std::time::SystemTime::now() + Duration::from_secs(5)).unwrap();
        assert_eq!(pending_crash(&user).unwrap().report.as_deref(), Some("crash-20261015-140000.txt"));
    }

    #[test]
    fn an_observed_error_exit_counts_and_a_clean_one_does_not() {
        let (_d, _root, user) = stage();
        record_start(&user, 5);
        record_exit(&user, 5, Some(0));
        assert!(pending_crash(&user).is_none());
        record_start(&user, 6);
        record_exit(&user, 99, Some(101)); // another process id: ignored
        assert!(pending_crash(&user).is_none());
        record_exit(&user, 6, Some(101));
        assert_eq!(pending_crash(&user).unwrap().code, Some(101));
        dismiss_crash(&user);
        assert!(pending_crash(&user).is_none());
    }

    #[test]
    fn an_old_report_before_the_first_start_is_not_a_crash_of_the_last_run() {
        let (_d, _root, user) = stage();
        std::fs::write(user.join("logs").join("crash-old.txt"), "old").unwrap();
        let f = std::fs::File::options().write(true).open(user.join("logs").join("crash-old.txt")).unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(3600)).unwrap();
        record_start(&user, 7);
        assert!(pending_crash(&user).is_none());
    }
}
