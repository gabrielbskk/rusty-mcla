//! Verify (and prepare repair): re-check the built data folder against the file list written when it was stamped.

use crate::error::{ErrorKind, InstallError, Result};
use crate::layout::{parse_list, Layout, State};
use crate::progress::{Ctx, Stage, Unit};
use rayon::prelude::*;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VerifyReport {
    pub checked: usize,
    pub missing: Vec<String>,
    pub wrong_size: Vec<String>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.missing.is_empty() && self.wrong_size.is_empty()
    }
    pub fn damaged(&self) -> usize {
        self.missing.len() + self.wrong_size.len()
    }
    /// Stage ids (`extract`, `audio`, `video`) that have damaged files.
    pub fn damaged_stages(&self) -> Vec<&'static str> {
        let mut v = Vec::new();
        for (prefix, stage) in [("cache/", "extract"), ("audio_pc/", "audio"), ("video_pc/", "video")] {
            if self.missing.iter().chain(self.wrong_size.iter()).any(|p| p.starts_with(prefix)) {
                v.push(stage);
            }
        }
        v
    }
}

/// Compare every listed file's presence and size. Files in `data/` are listed relative to `data/`.
pub fn verify(l: &Layout, ctx: &Ctx) -> Result<VerifyReport> {
    let text = std::fs::read_to_string(l.files_list()).map_err(|_| InstallError::new(ErrorKind::DataDamaged, "the file list data/.install/files.tsv is missing: the data was not built here"))?;
    let list = parse_list(&text);
    let counter = ctx.counter(Stage::Verify, Unit::Items, list.len() as u64);
    counter.set_message("Checking the built files");
    let data = l.data();
    let bad: std::sync::Mutex<(Vec<String>, Vec<String>)> = std::sync::Mutex::new((Vec::new(), Vec::new()));
    list.par_chunks(512).try_for_each(|chunk| {
        for f in chunk {
            match std::fs::metadata(data.join(&f.path)) {
                Ok(m) if m.len() == f.size => {}
                Ok(_) => bad.lock().unwrap_or_else(|p| p.into_inner()).1.push(f.path.clone()),
                Err(_) => bad.lock().unwrap_or_else(|p| p.into_inner()).0.push(f.path.clone()),
            }
        }
        counter.add(chunk.len() as u64)
    })?;
    counter.finish();
    let (mut missing, mut wrong_size) = bad.into_inner().unwrap_or_default();
    missing.sort();
    wrong_size.sort();
    Ok(VerifyReport { checked: list.len(), missing, wrong_size })
}

/// Prepare a repair: delete files of the wrong size and clear the `done` flag of every damaged stage, so the next install run rebuilds
/// only what is missing.
pub fn prepare_repair(l: &Layout, report: &VerifyReport) -> Result<()> {
    for p in &report.wrong_size {
        let _ = std::fs::remove_file(l.data().join(p));
    }
    let mut st = State::load(l);
    for s in report.damaged_stages() {
        st.done.insert(s.to_string(), false);
    }
    st.save(l)
}
