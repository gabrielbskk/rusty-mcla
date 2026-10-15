//! Progress and cancel API. No UI code: a caller passes a closure that receives [`Event`]s and may flip the [`Cancel`] flag from any thread.

use crate::error::{InstallError, Result};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Preflight,
    Validate,
    Checksum,
    Extract,
    Ffmpeg,
    Audio,
    Video,
    Stamp,
    Verify,
}

impl Stage {
    pub fn id(self) -> &'static str {
        match self {
            Stage::Preflight => "preflight",
            Stage::Validate => "validate",
            Stage::Checksum => "checksum",
            Stage::Extract => "extract",
            Stage::Ffmpeg => "ffmpeg",
            Stage::Audio => "audio",
            Stage::Video => "video",
            Stage::Stamp => "stamp",
            Stage::Verify => "verify",
        }
    }
    /// The stages of an install, in order (Checksum runs beside the others).
    pub const INSTALL: [Stage; 8] = [Stage::Preflight, Stage::Validate, Stage::Extract, Stage::Ffmpeg, Stage::Audio, Stage::Video, Stage::Stamp, Stage::Verify];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unit {
    Bytes,
    Items,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pending,
    Pass,
    Warn,
    Fail,
}

/// One line of the validation checklist (`id` is a stable key, `text` an English detail).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Check {
    pub id: String,
    pub status: CheckStatus,
    pub text: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub stage: Stage,
    pub done: u64,
    pub total: u64,
    pub unit: Unit,
    pub message: String,
    /// Set by the validation stage for each checklist line as it completes.
    pub check: Option<Check>,
    /// True for the last event of a stage.
    pub finished: bool,
}

/// Cooperative cancel flag, cheap to clone, flip from any thread.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub fn reset(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(InstallError::cancelled())
        } else {
            Ok(())
        }
    }
}

pub type Sink = Arc<dyn Fn(&Event) + Send + Sync>;

/// What every long function receives: where events go and the cancel flag.
#[derive(Clone)]
pub struct Ctx {
    sink: Sink,
    pub cancel: Cancel,
}

impl Ctx {
    pub fn new(sink: Sink, cancel: Cancel) -> Self {
        Ctx { sink, cancel }
    }
    /// No listener (tests, fire and forget).
    pub fn silent() -> Self {
        Ctx { sink: Arc::new(|_| {}), cancel: Cancel::new() }
    }
    pub fn emit(&self, e: Event) {
        (self.sink)(&e);
    }
    pub fn check(&self) -> Result<()> {
        self.cancel.check()
    }
    pub fn message(&self, stage: Stage, text: impl Into<String>) {
        self.emit(Event { stage, done: 0, total: 0, unit: Unit::Items, message: text.into(), check: None, finished: false });
    }
    pub fn check_line(&self, stage: Stage, id: &str, status: CheckStatus, text: impl Into<String>) {
        let text = text.into();
        self.emit(Event { stage, done: 0, total: 0, unit: Unit::Items, message: text.clone(), check: Some(Check { id: id.to_string(), status, text }), finished: false });
    }
    /// A throttled counter for one stage (at most ~10 events a second, plus the final one).
    pub fn counter(&self, stage: Stage, unit: Unit, total: u64) -> Counter {
        Counter { ctx: self.clone(), stage, unit, total: AtomicU64::new(total), done: AtomicU64::new(0), last: Mutex::new(Instant::now() - Duration::from_secs(1)), message: Mutex::new(String::new()) }
    }
}

pub struct Counter {
    ctx: Ctx,
    stage: Stage,
    unit: Unit,
    total: AtomicU64,
    done: AtomicU64,
    last: Mutex<Instant>,
    message: Mutex<String>,
}

impl Counter {
    pub fn set_total(&self, total: u64) {
        self.total.store(total, Ordering::Relaxed);
    }
    pub fn add_total(&self, n: u64) {
        self.total.fetch_add(n, Ordering::Relaxed);
    }
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }
    pub fn set_message(&self, m: impl Into<String>) {
        if let Ok(mut g) = self.message.lock() {
            *g = m.into();
        }
    }
    /// Add `n` to the done count and maybe emit; returns Err when cancelled.
    pub fn add(&self, n: u64) -> Result<()> {
        self.done.fetch_add(n, Ordering::Relaxed);
        self.emit_throttled();
        self.ctx.check()
    }
    fn emit_throttled(&self) {
        let Ok(mut last) = self.last.lock() else { return };
        if last.elapsed() < Duration::from_millis(100) {
            return;
        }
        *last = Instant::now();
        drop(last);
        self.emit(false);
    }
    fn emit(&self, finished: bool) {
        let message = self.message.lock().map(|m| m.clone()).unwrap_or_default();
        self.ctx.emit(Event { stage: self.stage, done: self.done(), total: self.total.load(Ordering::Relaxed), unit: self.unit, message, check: None, finished });
    }
    pub fn finish(&self) {
        self.emit(true);
    }
}
