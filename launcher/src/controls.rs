//! The Controls tab's keyboard table (Tauri commands): the game's own bindings (`mcla_config::keybinds`, the model the
//! game's OPTIONS > CONTROLLER > KEYBOARD page uses), read from and written into the game's save,
//! `<user dir>/saves/mcla_save.json`, member `keyboard` (docs/LAUNCHER.md decision 8: no second binding system).
//!
//! Every change is written at once (only the `keyboard` member; atomic; the first write of a launcher session first
//! copies the save to `mcla_save.json.bak`). With no save yet the page shows the defaults read-only: the launcher never
//! creates the save, because the game's first start copies an older save from the data folder only when there is none
//! (DECISIONS.md D16), and a save made here would hide that one.

use crate::logging;
use mcla_config::keybinds::{self as kb, save as ks, Bindings, Key};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Controls {
    path: PathBuf,
    writer: Mutex<ks::SaveWriter>,
}

#[derive(Serialize, Clone)]
pub struct KeyView {
    code: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
pub struct RowView {
    id: &'static str,
    keys: [Option<KeyView>; 2],
    must: bool,
}

#[derive(Serialize)]
pub struct ErrView {
    code: &'static str,
    detail: String,
}

#[derive(Serialize)]
pub struct KbdView {
    /// `ok`, `no_save` (first run: defaults shown read-only) or `error` (the save could not be read)
    state: &'static str,
    path: String,
    error: Option<ErrView>,
    /// the save has a `keyboard` member (false = the defaults)
    present: bool,
    warning: Option<String>,
    defaults: bool,
    unmapped: bool,
    rows: Vec<RowView>,
    /// the keys the game keeps for itself: (locale id, keys)
    fixed: Vec<(&'static str, &'static str)>,
}

#[derive(Serialize)]
pub struct EditResult {
    /// `saved`, `unchanged`, `refused` (reason), `conflict` (asks before moving the key), `failed` (error)
    outcome: &'static str,
    /// `reserved`, `alt_enter`, `unknown` for a refused key
    reason: Option<&'static str>,
    key: Option<KeyView>,
    /// where the key is now: (action id, slot)
    conflict: Option<(&'static str, usize)>,
    error: Option<ErrView>,
    backup: Option<String>,
    view: KbdView,
}

fn key_view(k: Key) -> KeyView {
    KeyView { code: k.code(), name: k.name() }
}

fn err_view(e: &ks::SaveError) -> ErrView {
    ErrView { code: e.code(), detail: e.to_string() }
}

impl Controls {
    pub fn new(user_dir: &std::path::Path) -> Controls {
        Controls { path: user_dir.join("saves").join("mcla_save.json"), writer: Mutex::new(ks::SaveWriter::new()) }
    }

    fn view_of(&self, b: &Bindings, state: &'static str, error: Option<ErrView>, present: bool, warning: Option<String>) -> KbdView {
        let rows = kb::ACTS
            .iter()
            .enumerate()
            .map(|(i, a)| RowView { id: a.name, keys: [b.keys[i][0].map(key_view), b.keys[i][1].map(key_view)], must: b.must(i) })
            .collect();
        KbdView {
            state,
            path: self.path.to_string_lossy().into_owned(),
            error,
            present,
            warning,
            defaults: *b == Bindings::default(),
            unmapped: b.unmapped(),
            rows,
            fixed: kb::FIXED.to_vec(),
        }
    }

    fn read(&self) -> Result<(Bindings, KbdView), KbdView> {
        match ks::read(&self.path) {
            Ok(r) => {
                let v = self.view_of(&r.bindings, "ok", None, r.present, r.warning.clone());
                Ok((r.bindings, v))
            }
            Err(e @ ks::SaveError::NotFound(_)) => Err(self.view_of(&Bindings::default(), "no_save", Some(err_view(&e)), false, None)),
            Err(e) => {
                logging::warn(&format!("controls: cannot read the save: {e}"));
                Err(self.view_of(&Bindings::default(), "error", Some(err_view(&e)), false, None))
            }
        }
    }

    pub fn get(&self) -> KbdView {
        match self.read() {
            Ok((_, v)) | Err(v) => v,
        }
    }

    fn result(outcome: &'static str, view: KbdView) -> EditResult {
        EditResult { outcome, reason: None, key: None, conflict: None, error: None, backup: None, view }
    }

    /// Write `b` and say how it went.
    fn commit(&self, b: &Bindings, what: &str) -> EditResult {
        let mut w = match self.writer.lock() {
            Ok(w) => w,
            Err(p) => p.into_inner(),
        };
        match w.write(&self.path, b) {
            Ok(done) => {
                let backup = done.backup.map(|p| p.to_string_lossy().into_owned());
                if let Some(bak) = &backup {
                    logging::info(&format!("controls: save backed up to {bak}"));
                }
                logging::info(&format!("controls: {what} -> {}", if done.changed { "saved" } else { "unchanged" }));
                let mut r = Self::result(if done.changed { "saved" } else { "unchanged" }, self.get());
                r.backup = backup;
                r
            }
            Err(e) => {
                logging::error(&format!("controls: {what} not saved: {e}"));
                let mut r = Self::result("failed", self.get());
                r.error = Some(err_view(&e));
                r
            }
        }
    }

    fn start(&self) -> Result<Bindings, EditResult> {
        self.read().map(|(b, _)| b).map_err(|v| {
            let mut r = Self::result("failed", v);
            r.error = r.view.error.as_ref().map(|e| ErrView { code: e.code, detail: e.detail.clone() });
            r
        })
    }

    /// The key (`code` = `KeyboardEvent.code`, `alt` = Alt held) for an action's slot. A key that another place has is
    /// only moved with `force` (the page asks first).
    pub fn set(&self, act: &str, slot: usize, code: &str, alt: bool, force: bool) -> EditResult {
        let mut b = match self.start() {
            Ok(b) => b,
            Err(r) => return r,
        };
        let Some(a) = kb::action_by_name(act).filter(|_| slot < 2) else {
            let mut r = Self::result("refused", self.get());
            r.reason = Some("unknown");
            return r;
        };
        let refuse = |reason: &'static str, key: Option<Key>| {
            let mut r = Self::result("refused", self.get());
            r.reason = Some(reason);
            r.key = key.map(key_view);
            r
        };
        let Some(key) = Key::from_code(code) else { return refuse("unknown", None) };
        if kb::alt_enter(alt, key) {
            return refuse("alt_enter", Some(key));
        }
        if key.reserved() {
            return refuse("reserved", Some(key));
        }
        if let Some((ca, cs)) = b.conflict(a, slot, key) {
            if !force {
                let mut r = Self::result("conflict", self.get());
                r.key = Some(key_view(key));
                r.conflict = Some((kb::ACTS[ca].name, cs));
                return r;
            }
        }
        b.assign(a, slot, key);
        let mut r = self.commit(&b, &format!("{act} key {} = {}", slot + 1, key.name()));
        r.key = Some(key_view(key));
        r
    }

    pub fn clear(&self, act: &str, slot: usize) -> EditResult {
        let mut b = match self.start() {
            Ok(b) => b,
            Err(r) => return r,
        };
        let Some(a) = kb::action_by_name(act).filter(|_| slot < 2) else {
            let mut r = Self::result("refused", self.get());
            r.reason = Some("unknown");
            return r;
        };
        b.clear(a, slot);
        self.commit(&b, &format!("{act} key {} cleared", slot + 1))
    }

    pub fn reset(&self) -> EditResult {
        if let Err(r) = self.start() {
            return r;
        }
        self.commit(&Bindings::default(), "reset to defaults")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rusty-mcla-controls-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("saves")).unwrap();
        d
    }

    fn keys(v: &KbdView, id: &str) -> Vec<Option<&'static str>> {
        v.rows.iter().find(|r| r.id == id).unwrap().keys.iter().map(|k| k.as_ref().map(|k| k.code)).collect()
    }

    #[test]
    fn edits_go_through_the_model_into_the_save() {
        let d = dir("edit");
        let save = d.join("saves").join("mcla_save.json");
        let text = "{\n  \"car\": \"vp_aud_r8_08\",\n  \"money\": 1341764\n}";
        std::fs::write(&save, text).unwrap();
        let c = Controls::new(&d);
        let v = c.get();
        assert_eq!((v.state, v.rows.len(), v.present, v.defaults, v.fixed.iter().any(|f| f.1 == "Alt+Enter")), ("ok", 32, false, true, true));
        // (a free key: saved, backed up once)
        let r = c.set("NITRO", 0, "ControlLeft", false, false);
        assert_eq!((r.outcome, r.backup.is_some()), ("saved", true));
        assert_eq!(keys(&r.view, "NITRO"), vec![Some("ControlLeft"), None]);
        // (a used key: asked first, nothing written; then moved with force)
        let r = c.set("HORN", 0, "KeyE", false, false);
        assert_eq!((r.outcome, r.conflict), ("conflict", Some(("IN_ZONE", 0))));
        assert_eq!(keys(&c.get(), "IN_ZONE"), vec![Some("KeyE"), None]);
        let r = c.set("HORN", 0, "KeyE", false, true);
        assert_eq!((r.outcome, r.backup.is_none()), ("saved", true));
        assert_eq!((keys(&r.view, "HORN"), keys(&r.view, "IN_ZONE")), (vec![Some("KeyE"), None], vec![None, None]));
        // (refusals: reserved, Alt+Enter, a key without a name, an unknown action)
        for (code, alt, why) in [("F5", false, "reserved"), ("Escape", false, "reserved"), ("Enter", true, "alt_enter"), ("NumpadEnter", true, "alt_enter"), ("MetaLeft", false, "unknown")] {
            let r = c.set("HORN", 1, code, alt, false);
            assert_eq!((r.outcome, r.reason), ("refused", Some(why)), "{code}");
        }
        assert_eq!(c.set("NO_SUCH", 0, "KeyP", false, false).reason, Some("unknown"));
        // (Alt alone is an ordinary key)
        assert_eq!(c.set("HORN", 1, "AltLeft", true, false).outcome, "saved");
        // (clear down to "must be mapped")
        c.clear("GAS", 0);
        let r = c.clear("GAS", 0);
        assert!(r.view.rows.iter().find(|x| x.id == "GAS").unwrap().must && r.view.unmapped);
        // (the save: other members as they were; the game's names)
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&save).unwrap()).unwrap();
        assert_eq!((v["car"].as_str(), v["money"].as_i64()), (Some("vp_aud_r8_08"), Some(1341764)));
        assert_eq!(v["keyboard"]["HORN"], serde_json::json!(["E", "LEFT ALT"]));
        assert_eq!(std::fs::read_to_string(ks::bak_path(&save)).unwrap(), text);
        // (reset: the member goes, the file is the original again)
        assert_eq!(c.reset().outcome, "saved");
        assert_eq!(std::fs::read_to_string(&save).unwrap(), text);
        assert_eq!(c.reset().outcome, "unchanged");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn no_save_and_bad_save_are_read_only() {
        let d = dir("none");
        let c = Controls::new(&d);
        let v = c.get();
        assert_eq!((v.state, v.defaults, v.rows.len()), ("no_save", true, 32));
        let r = c.set("NITRO", 0, "ControlLeft", false, false);
        assert_eq!((r.outcome, r.error.as_ref().map(|e| e.code)), ("failed", Some("no_save")));
        assert_eq!((c.reset().outcome, c.clear("GAS", 0).outcome), ("failed", "failed"));
        assert!(!d.join("saves").join("mcla_save.json").exists(), "the launcher never creates the save");
        std::fs::write(d.join("saves").join("mcla_save.json"), "[1,2").unwrap();
        assert_eq!(c.get().state, "error");
        assert_eq!(c.set("NITRO", 0, "ControlLeft", false, false).outcome, "failed");
        assert_eq!(std::fs::read_to_string(d.join("saves").join("mcla_save.json")).unwrap(), "[1,2");
        let _ = std::fs::remove_dir_all(d);
    }
}
