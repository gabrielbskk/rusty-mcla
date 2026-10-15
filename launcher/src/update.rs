//! Update system glue: the Tauri commands behind the Home banner, the release notes, Settings > Updates and the first-run game package.
//! All the work is in the `mcla-updater` library; this file owns the run state (one job at a time), the progress the page polls
//! (`update_status`, same idea as the install page: no Tauri events, so no capability file) and the log lines.
//!
//! Nothing here runs without the user: the launcher checks on start (if `check_updates` is on) but only the buttons install anything.

use mcla_updater::config::ReleaseConfig;
use mcla_updater::http::Http;
use mcla_updater::manifest::ReleaseManifest;
use mcla_updater::package::{self, Options};
use mcla_updater::plan::{plan, Installed, Plan};
use mcla_updater::releases::{CheckState, Checked, Release, Releases};
use mcla_updater::{notes, selfupdate, version, Cancel, Kind, Phase, Progress, UpdateError};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::logging;
use crate::settings::Settings;

/// An error as the page shows it: `code` selects the translated text (`dyn.upd.err.<code>`).
#[derive(Serialize, Clone, Debug)]
pub struct ErrorView {
    pub code: String,
    pub detail: String,
    /// Which job failed: `update`, `setup`, `version`, `package`.
    pub job: String,
    pub reset_at: Option<i64>,
}

#[derive(Serialize, Clone, Debug)]
pub struct DoneView {
    pub job: String,
    pub game_version: Option<String>,
    pub needs_rebuild: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct ProgressView {
    pub job: String,
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct ReleaseView {
    pub tag: String,
    pub version: String,
    pub name: String,
    pub published_at: Option<String>,
    pub html_url: String,
    pub notes_html: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct CheckView {
    #[serde(flatten)]
    pub state: CheckState,
    /// Unix seconds of the last answer from GitHub, 0 = never.
    pub checked_at: i64,
    /// `check_updates` is switched off: nothing is checked on start.
    pub enabled: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct InstalledView {
    pub launcher: String,
    pub game_version: Option<String>,
    pub game_data_version: Option<u32>,
    pub built_data_version: Option<u32>,
}

#[derive(Serialize, Clone, Debug)]
pub struct StatusView {
    /// `idle`, `checking` or `updating`
    pub busy: &'static str,
    pub check: CheckView,
    pub release: Option<ReleaseView>,
    pub manifest_error: Option<String>,
    pub plan: Option<Plan>,
    pub installed: InstalledView,
    pub progress: Option<ProgressView>,
    pub error: Option<ErrorView>,
    pub done: Option<DoneView>,
    pub skipped_version: Option<String>,
    pub game_bytes: u64,
    pub launcher_bytes: u64,
    pub key_set: bool,
    /// Web pages of the repository (project, all releases, new issue), for the About tab.
    pub links: Links,
}

#[derive(Serialize, Clone, Debug)]
pub struct ReleaseRow {
    pub tag: String,
    pub version: String,
    pub name: String,
    pub published_at: Option<String>,
    pub installed: bool,
}

struct Shared {
    busy: &'static str,
    cancel: Option<Cancel>,
    last: Option<Checked>,
    release_view: Option<ReleaseView>,
    check_state: CheckState,
    progress: Option<ProgressView>,
    error: Option<ErrorView>,
    done: Option<DoneView>,
}

#[derive(Clone)]
pub struct Updater {
    inner: Arc<Mutex<Shared>>,
    cfg: Arc<ReleaseConfig>,
    releases: Option<Releases>,
    root: PathBuf,
    user_dir: PathBuf,
    exe: PathBuf,
    /// how many save backups to keep (`backups_keep` of launcher.json; the save manager changes it)
    keep: Arc<std::sync::atomic::AtomicUsize>,
}

fn lock(u: &Updater) -> std::sync::MutexGuard<'_, Shared> {
    u.inner.lock().unwrap_or_else(|p| p.into_inner())
}

fn err_view(job: &str, e: &UpdateError) -> ErrorView {
    ErrorView { code: e.kind.code().to_string(), detail: e.detail.clone(), job: job.to_string(), reset_at: e.reset_at }
}

fn release_view(r: &Release) -> ReleaseView {
    ReleaseView {
        tag: r.tag.clone(),
        version: r.version().unwrap_or_else(|| r.tag.clone()),
        name: r.name.clone(),
        published_at: r.published_at.clone(),
        html_url: r.html_url.clone(),
        notes_html: notes::render(&r.body),
    }
}

impl Updater {
    pub fn new(user_dir: &Path, root: &Path, exe: &Path) -> Updater {
        let cfg = Arc::new(ReleaseConfig::from_env(env!("CARGO_PKG_VERSION")));
        logging::info(&format!("update source: {} (api {})", cfg.repo, cfg.api_base));
        let releases = match Http::new(cfg.clone()) {
            Ok(http) => Some(Releases::new(http, user_dir.join("cache"))),
            Err(e) => {
                logging::error(&format!("update system disabled: {e}"));
                None
            }
        };
        let cached = releases.as_ref().map(|r| r.cached());
        let u = Updater {
            inner: Arc::new(Mutex::new(Shared { busy: "idle", cancel: None, last: None, release_view: None, check_state: CheckState::NotChecked, progress: None, error: None, done: None })),
            cfg,
            releases,
            root: root.to_path_buf(),
            user_dir: user_dir.to_path_buf(),
            exe: exe.to_path_buf(),
            keep: Arc::new(std::sync::atomic::AtomicUsize::new(mcla_updater::backup::KEEP_DEFAULT)),
        };
        if let Some(c) = cached {
            u.set_checked(c);
        }
        u
    }

    pub fn set_keep(&self, n: usize) {
        self.keep.store(n.clamp(1, 100), std::sync::atomic::Ordering::Relaxed);
    }

    /// An update, setup or version job is running (the game folder may be swapped any moment).
    pub fn is_updating(&self) -> bool {
        lock(self).busy == "updating"
    }

    /// The repository's "new issue" page.
    pub fn issues_base(&self) -> String {
        format!("{}/{}/issues/new", self.cfg.web_base.trim_end_matches('/'), self.cfg.repo)
    }

    /// The version this launcher reports (a test hook can change it when the mock server is configured).
    pub fn running_version(&self) -> String {
        if self.cfg.mock_authority.is_some() {
            if let Ok(v) = std::env::var("RUSTY_MCLA_TEST_VERSION") {
                if version::parse(&v).is_some() {
                    return v;
                }
            }
        }
        env!("CARGO_PKG_VERSION").to_string()
    }

    /// The install pipeline's manifest: the installed game's own copy of its release manifest overrides the compiled-in one.
    pub fn install_manifest(&self) -> mcla_install::Manifest {
        match package::read_manifest_copy(&self.root, &self.cfg) {
            Some(m) => m.to_install_manifest(),
            None => mcla_install::Manifest::embedded(),
        }
    }

    fn installed(&self) -> Installed {
        let rec = package::read_record(&self.root);
        let built = mcla_install::LocalManifest::load(&mcla_install::Layout::new(&self.root)).map(|m| m.data_version);
        Installed { launcher: self.running_version(), game_version: rec.as_ref().map(|r| r.version.clone()), game_data_version: rec.as_ref().map(|r| r.data_version), built_data_version: built }
    }

    fn set_checked(&self, c: Checked) {
        let view = c.release.as_ref().map(release_view);
        let mut s = lock(self);
        s.check_state = c.state.clone();
        s.release_view = view;
        s.last = Some(c);
    }

    pub fn status(&self, settings: &Settings) -> StatusView {
        let inst = self.installed();
        let s = lock(self);
        let manifest = s.last.as_ref().and_then(|c| c.manifest.clone());
        let pl = manifest.as_ref().map(|m| plan(m, &inst, settings.skipped_version.as_deref()));
        let mut plan_v = pl;
        if plan_v.is_none() {
            // no manifest at all (never checked, or the release has none): still say whether the installed pair is consistent
            if let (Some(g), Some(b)) = (inst.game_data_version, inst.built_data_version) {
                if g != b {
                    plan_v = Some(Plan { version: String::new(), launcher_update: false, game_update: false, first_install: false, launcher_first: false, update_needs_rebuild: false, rebuild_pending: true, skipped: false, offer: false });
                }
            }
        }
        StatusView {
            busy: s.busy,
            check: CheckView { state: s.check_state.clone(), checked_at: s.last.as_ref().map(|c| c.checked_at).unwrap_or(0), enabled: settings.check_updates },
            release: s.release_view.clone(),
            manifest_error: s.last.as_ref().and_then(|c| c.manifest_error.clone()),
            plan: plan_v,
            installed: InstalledView { launcher: inst.launcher.clone(), game_version: inst.game_version.clone(), game_data_version: inst.game_data_version, built_data_version: inst.built_data_version },
            progress: s.progress.clone(),
            error: s.error.clone(),
            done: s.done.clone(),
            skipped_version: settings.skipped_version.clone(),
            game_bytes: manifest.as_ref().map(|m| m.game.size).unwrap_or(0),
            launcher_bytes: manifest.as_ref().map(|m| m.launcher.size).unwrap_or(0),
            key_set: selfupdate::public_key(&self.cfg).is_ok(),
            links: Links { repo: format!("{}/{}", self.cfg.web_base.trim_end_matches('/'), self.cfg.repo), releases: format!("{}/{}/releases", self.cfg.web_base.trim_end_matches('/'), self.cfg.repo), issues: format!("{}/{}/issues/new", self.cfg.web_base.trim_end_matches('/'), self.cfg.repo) },
        }
    }

    // ---------- check ----------

    /// Starts a check (conditional request). `auto` = the start-up check: it does nothing when `check_updates` is off.
    pub fn check(&self, auto: bool, enabled: bool) {
        if auto && !enabled {
            logging::info("update check on start is switched off");
            return;
        }
        let Some(releases) = self.releases.clone() else { return };
        {
            let mut s = lock(self);
            if s.busy != "idle" {
                return;
            }
            s.busy = "checking";
        }
        let me = self.clone();
        tauri::async_runtime::spawn(async move {
            let c = releases.check_latest().await;
            match &c.state {
                CheckState::Fresh | CheckState::NotModified => logging::info(&format!("update check: {:?}, latest {}", c.state, c.release.as_ref().map(|r| r.tag.as_str()).unwrap_or("-"))),
                other => logging::warn(&format!("update check: {other:?}")),
            }
            me.set_checked(c);
            lock(&me).busy = "idle";
        });
    }

    pub async fn list(&self) -> Result<Vec<ReleaseRow>, ErrorView> {
        let Some(r) = &self.releases else { return Err(ErrorView { code: "io".into(), detail: "update system unavailable".into(), job: "list".into(), reset_at: None }) };
        let have = package::read_record(&self.root).map(|x| x.version);
        match r.list(20).await {
            Ok(list) => Ok(list.into_iter().map(|x| ReleaseRow { version: x.version().unwrap_or_default(), installed: x.version() == have, tag: x.tag, name: x.name, published_at: x.published_at }).collect()),
            Err(e) => {
                logging::warn(&format!("release list: {e}"));
                Err(err_view("list", &e))
            }
        }
    }

    // ---------- jobs ----------

    pub fn cancel(&self) {
        if let Some(c) = &lock(self).cancel {
            c.cancel();
        }
    }

    fn begin(&self, job: &str) -> Result<Cancel, ()> {
        let mut s = lock(self);
        if s.busy != "idle" {
            return Err(());
        }
        let c = Cancel::new();
        s.busy = "updating";
        s.cancel = Some(c.clone());
        s.error = None;
        s.done = None;
        s.progress = Some(ProgressView { job: job.to_string(), phase: Phase::Check, done: 0, total: 0 });
        Ok(c)
    }

    fn end(&self, job: &str, r: Result<DoneView, UpdateError>) {
        let mut s = lock(self);
        s.busy = "idle";
        s.cancel = None;
        s.progress = None;
        match r {
            Ok(d) => {
                logging::info(&format!("update job {job} finished: game {:?}, rebuild needed {}", d.game_version, d.needs_rebuild));
                if d.game_version.is_some() {
                    logging::info(&format!("install manifest in effect now: data_version {}", self.install_manifest().data_version));
                }
                s.done = Some(d);
            }
            Err(e) if e.is_cancelled() => logging::info(&format!("update job {job} cancelled")),
            Err(e) => {
                logging::error(&format!("update job {job} failed: {e}"));
                s.error = Some(err_view(job, &e));
            }
        }
    }

    fn progress_fn(&self, job: &str) -> mcla_updater::ProgressFn {
        let me = self.clone();
        let job = job.to_string();
        Arc::new(move |p: Progress| {
            lock(&me).progress = Some(ProgressView { job: job.clone(), phase: p.phase, done: p.done, total: p.total });
        })
    }

    fn options(&self, backup: bool) -> Options {
        Options { backup_save: backup, keep_backups: self.keep.load(std::sync::atomic::Ordering::Relaxed).max(1), ..Options::default() }
    }

    fn latest(&self) -> Result<(Release, ReleaseManifest), UpdateError> {
        let s = lock(self);
        let c = s.last.as_ref().ok_or_else(|| UpdateError::new(Kind::Missing, "no release information yet"))?;
        match (&c.release, &c.manifest) {
            (Some(r), Some(m)) => Ok((r.clone(), m.clone())),
            _ => Err(UpdateError::new(Kind::Missing, c.manifest_error.clone().unwrap_or_else(|| "the latest release has no usable manifest".into()))),
        }
    }

    /// UPDATE NOW: game first (the launcher can be restarted only once), then the launcher; when the running launcher is older than the
    /// release's `min_launcher_version` the launcher alone comes first and the game follows on the next check.
    pub fn update_now(&self, app: tauri::AppHandle, backup: bool, skipped: Option<String>) {
        let Ok(cancel) = self.begin("update") else { return };
        let me = self.clone();
        logging::info("UPDATE NOW pressed");
        tauri::async_runtime::spawn(async move {
            let r = me.run_update(&cancel, backup, skipped, &app).await;
            me.end("update", r);
        });
    }

    async fn run_update(&self, cancel: &Cancel, backup: bool, skipped: Option<String>, app: &tauri::AppHandle) -> Result<DoneView, UpdateError> {
        let (rel, m) = self.latest()?;
        let http = self.http()?;
        let p = plan(&m, &self.installed(), skipped.as_deref());
        logging::info(&format!("update plan: {p:?}"));
        let progress = self.progress_fn("update");
        let mut done = DoneView { job: "update".into(), game_version: None, needs_rebuild: false };
        if p.game_update && !p.launcher_first {
            let rec = package::download_and_install(&http, &rel, &m, &self.root, &self.user_dir, &self.options(backup), cancel, &progress).await?;
            logging::info(&format!("game package {} installed (sha256 {}, {} bytes)", rec.version, rec.sha256, rec.size));
            done.game_version = Some(rec.version);
            done.needs_rebuild = p.update_needs_rebuild;
        }
        if p.launcher_update {
            let staged = selfupdate::download_and_stage(&http, &rel, &m, &self.exe, cancel, &progress).await?;
            logging::info(&format!("launcher {} verified (size {}, sha256 {}, signature ok)", m.launcher.version, m.launcher.size, m.launcher.sha256));
            progress(Progress { phase: Phase::ApplyLauncher, done: 0, total: 1 });
            let old = selfupdate::apply(&self.exe, &staged)?;
            match selfupdate::restart(&self.exe) {
                Ok(child) => {
                    logging::info(&format!("launcher replaced ({} -> {}); new process {} started, this one exits", old.display(), self.exe.display(), child.id()));
                    progress(Progress { phase: Phase::ApplyLauncher, done: 1, total: 1 });
                    tokio::time::sleep(std::time::Duration::from_millis(900)).await;
                    app.exit(0);
                }
                Err(e) => {
                    let _ = selfupdate::rollback(&self.exe, &old);
                    return Err(UpdateError::io("the new launcher could not be started, the old one was restored", &e));
                }
            }
        }
        Ok(done)
    }

    fn http(&self) -> Result<Http, UpdateError> {
        self.releases.as_ref().map(|r| r.http().clone()).ok_or_else(|| UpdateError::new(Kind::Io, "update system unavailable"))
    }

    /// First run, step 1: the game package of the latest release (no launcher self-update: the user just got the latest launcher).
    pub fn setup_game(&self, check_first: bool) {
        let Ok(cancel) = self.begin("setup") else { return };
        let me = self.clone();
        logging::info("game package requested (first run)");
        tauri::async_runtime::spawn(async move {
            let r = me.run_setup(&cancel, check_first).await;
            me.end("setup", r);
        });
    }

    async fn run_setup(&self, cancel: &Cancel, check_first: bool) -> Result<DoneView, UpdateError> {
        let releases = self.releases.clone().ok_or_else(|| UpdateError::new(Kind::Io, "update system unavailable"))?;
        if check_first || self.latest().is_err() {
            let c = releases.check_latest().await;
            self.set_checked(c);
        }
        let (rel, m) = match self.latest() {
            Ok(x) => x,
            Err(e) => {
                // a check that failed for the network's sake is "offline", not "missing"
                let st = lock(self).check_state.clone();
                return Err(match st {
                    CheckState::Offline => UpdateError::new(Kind::Offline, "no connection to the release server"),
                    CheckState::RateLimited { reset_at } => UpdateError { kind: Kind::RateLimited, detail: "rate limited".into(), reset_at },
                    CheckState::Failed { code, detail } if code == Kind::Manifest.code() => UpdateError::new(Kind::Manifest, detail),
                    _ => e,
                });
            }
        };
        let progress = self.progress_fn("setup");
        let rec = package::download_and_install(&releases.http().clone(), &rel, &m, &self.root, &self.user_dir, &self.options(false), cancel, &progress).await?;
        logging::info(&format!("game package {} installed (sha256 {}, {} bytes)", rec.version, rec.sha256, rec.size));
        Ok(DoneView { job: "setup".into(), game_version: Some(rec.version), needs_rebuild: false })
    }

    /// "Install a specific version": the game package of that release (the launcher stays as it is).
    pub fn install_tag(&self, tag: String, backup: bool) {
        let Ok(cancel) = self.begin("version") else { return };
        let me = self.clone();
        logging::info(&format!("install a specific version: {tag}"));
        tauri::async_runtime::spawn(async move {
            let r = me.run_tag(&cancel, &tag, backup).await;
            me.end("version", r);
        });
    }

    async fn run_tag(&self, cancel: &Cancel, tag: &str, backup: bool) -> Result<DoneView, UpdateError> {
        let releases = self.releases.clone().ok_or_else(|| UpdateError::new(Kind::Io, "update system unavailable"))?;
        let list = releases.list(20).await?;
        let rel = list.into_iter().find(|r| r.tag == tag).ok_or_else(|| UpdateError::new(Kind::Missing, format!("release {tag} is not in the list")))?;
        let m = releases.manifest_for(&rel).await?;
        if version::is_newer(&m.min_launcher_version, &self.running_version()) {
            return Err(UpdateError::new(Kind::Manifest, format!("that release needs launcher {} or newer", m.min_launcher_version)));
        }
        let before = self.installed();
        let progress = self.progress_fn("version");
        let rec = package::download_and_install(releases.http(), &rel, &m, &self.root, &self.user_dir, &self.options(backup), cancel, &progress).await?;
        let needs_rebuild = before.built_data_version.map(|d| d != rec.data_version).unwrap_or(false);
        Ok(DoneView { job: "version".into(), game_version: Some(rec.version), needs_rebuild })
    }

    /// A game package the user already has (a zip chosen in the file dialog), verified exactly like a download.
    pub fn use_local_package(&self, path: String, backup: bool) {
        let Ok(cancel) = self.begin("package") else { return };
        let me = self.clone();
        logging::info(&format!("local game package chosen: {path}"));
        tauri::async_runtime::spawn(async move {
            let r = me.run_local(&cancel, PathBuf::from(path), backup).await;
            me.end("package", r);
        });
    }

    async fn run_local(&self, cancel: &Cancel, zip: PathBuf, backup: bool) -> Result<DoneView, UpdateError> {
        let before = self.installed();
        let (root, user, cfg, cancel2, progress) = (self.root.clone(), self.user_dir.clone(), self.cfg.clone(), cancel.clone(), self.progress_fn("package"));
        let cached = lock(self).last.as_ref().and_then(|c| c.manifest.clone());
        let opts = self.options(backup);
        let rec = tokio::task::spawn_blocking(move || -> Result<package::PackageRecord, UpdateError> {
            let mut candidates: Vec<ReleaseManifest> = Vec::new();
            if let Some(dir) = zip.parent() {
                if let Ok(t) = std::fs::read_to_string(dir.join("manifest.json")) {
                    match ReleaseManifest::parse(&t, &cfg) {
                        Ok(m) => candidates.push(m),
                        Err(e) => logging::warn(&format!("manifest.json beside the package is not usable: {e}")),
                    }
                }
            }
            candidates.extend(cached);
            candidates.extend(package::read_manifest_copy(&root, &cfg));
            let m = package::identify_local(&zip, &candidates, &cancel2)?;
            logging::info(&format!("local package matches release {} (game {})", m.version, m.game.version));
            package::install_from_zip(&root, &user, &zip, &m, "local", &opts, &cancel2, &progress)
        })
        .await
        .unwrap_or_else(|e| Err(UpdateError::new(Kind::Io, format!("installer thread failed: {e}"))))?;
        let needs_rebuild = before.built_data_version.map(|d| d != rec.data_version).unwrap_or(false);
        Ok(DoneView { job: "package".into(), game_version: Some(rec.version), needs_rebuild })
    }

    /// Dismisses the last result or error (the page shows each until it is dismissed or a new job starts).
    pub fn clear_result(&self) {
        let mut s = lock(self);
        s.error = None;
        s.done = None;
    }

    /// Start-up housekeeping: the `.old` exe of a finished self-update, partial downloads, a half-finished game swap.
    pub fn startup_cleanup(&self, updated: bool) {
        let (exe, root) = (self.exe.clone(), self.root.clone());
        std::thread::spawn(move || {
            if selfupdate::cleanup_after_start(&exe) {
                logging::info(&format!("removed the previous launcher file ({})", if updated { "after a self-update" } else { "left from an earlier update" }));
            }
            package::clean_stale(&root);
        });
    }
}

/// Opens an http(s) link in the system browser. Anything else is refused.
pub fn open_url(url: &str) -> Result<(), String> {
    let u = url::Url::parse(url).map_err(|_| "not a url".to_string())?;
    if !matches!(u.scheme(), "http" | "https") || url.len() > 2048 || url.chars().any(|c| c.is_control()) || u.host_str().is_none() {
        return Err("only http and https links are opened".into());
    }
    logging::info(&format!("open link {}", u.host_str().unwrap_or("")));
    crate::platform::open_url(u.as_str())
}

pub fn pick_package() -> Option<String> {
    rfd::FileDialog::new().set_title("Choose a game package").add_filter("Game package", &["zip"]).pick_file().map(|p| p.to_string_lossy().into_owned())
}

#[derive(Serialize, Clone, Debug)]
pub struct Links {
    pub repo: String,
    pub releases: String,
    pub issues: String,
}
