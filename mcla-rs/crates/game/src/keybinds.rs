//! The keyboard's bindings - the port's own addition (the original is a console game and has
//! no keyboard). The model - the 32 actions, their defaults, the reserved keys, conflicts and
//! the save field - lives in `mcla_config::keybinds`, shared with the launcher's Controls tab
//! (docs/LAUNCHER.md decision 8); this file is the game's thin adapter between that model's
//! keys (named by their W3C `code` strings) and winit's `KeyCode`, plus the page's state
//! (`KbdUi`) and the test hook. The page that edits it is OPTIONS > CONTROLLER > KEYBOARD
//! (`mc4::PAGE_KEYBOARD`), made after the pad's custom mapping editor; the table is kept in the
//! port's save (`SaveData::keyboard`). The rules are at the top of `mcla_config::keybinds`.

use mcla_config::keybinds as model;
use std::collections::BTreeMap;
use winit::keyboard::KeyCode;

pub use model::{rows, ACTS, COUNT};
pub use model::{ABILITY_NEXT, BRAKE, CAM_DOWN, CAM_UP, CONVERTIBLE_UP_DN, GARAGE, GAS, GPS_MAP, HANDBRAKE, HEADLIGHTS, HORN, HYDRAULICS, IN_ZONE, LEAN_BACK, LEAN_FORWARD, LOOKBACK, MAP};
pub use model::{MUSIC_DOWN, MUSIC_PAUSE, MUSIC_UP, NITRO, PAUSE, RESET_CAR, SHIFT_DOWN, SHIFT_UP, STEER_LEFT, STEER_RIGHT, WEIGHT_XFER, WHEEL_BACK_LEFT, WHEEL_BACK_RIGHT};
pub use model::{WHEEL_FRONT_LEFT, WHEEL_FRONT_RIGHT};

use KeyCode as K;

/// winit's key for each of the model's keys, in the order of `model::KEYS` (a test checks
/// that every variant's name is the model's `code`).
const CODES: [KeyCode; 98] = [
    K::KeyA, K::KeyB, K::KeyC, K::KeyD, K::KeyE, K::KeyF, K::KeyG, K::KeyH, K::KeyI, K::KeyJ, K::KeyK, K::KeyL, K::KeyM, K::KeyN, K::KeyO, K::KeyP, K::KeyQ, K::KeyR,
    K::KeyS, K::KeyT, K::KeyU, K::KeyV, K::KeyW, K::KeyX, K::KeyY, K::KeyZ,
    K::Digit0, K::Digit1, K::Digit2, K::Digit3, K::Digit4, K::Digit5, K::Digit6, K::Digit7, K::Digit8, K::Digit9,
    K::ArrowUp, K::ArrowDown, K::ArrowLeft, K::ArrowRight, K::Space, K::Tab, K::CapsLock,
    K::ShiftLeft, K::ShiftRight, K::ControlLeft, K::ControlRight, K::AltLeft, K::AltRight,
    K::Insert, K::Home, K::End, K::PageUp, K::PageDown,
    K::Minus, K::Equal, K::BracketLeft, K::BracketRight, K::Backslash, K::Semicolon, K::Quote, K::Comma, K::Period, K::Slash, K::Backquote, K::IntlBackslash,
    K::Numpad0, K::Numpad1, K::Numpad2, K::Numpad3, K::Numpad4, K::Numpad5, K::Numpad6, K::Numpad7, K::Numpad8, K::Numpad9,
    K::NumpadAdd, K::NumpadSubtract, K::NumpadMultiply, K::NumpadDivide, K::NumpadDecimal,
    K::Escape, K::Enter, K::NumpadEnter, K::Backspace, K::Delete,
    K::F1, K::F2, K::F3, K::F4, K::F5, K::F6, K::F7, K::F8, K::F9, K::F10, K::F11, K::F12,
];

/// The keys that have a name, with the name each has on the page and in the save.
pub const KEYS: [(KeyCode, &str); 98] = {
    let mut out = [(K::KeyA, ""); 98];
    let mut i = 0;
    while i < 98 {
        out[i] = (CODES[i], model::KEYS[i].name);
        i += 1;
    }
    out
};

/// The model's key for a winit key (None: a key without a name - the Windows key, media
/// keys, PrintScreen ...).
pub fn to_key(k: KeyCode) -> Option<model::Key> {
    CODES.iter().position(|c| *c == k).and_then(model::Key::at)
}

pub fn to_code(k: model::Key) -> KeyCode {
    CODES[k.index()]
}

/// A key no action may have (see the rules at the top of `mcla_config::keybinds`).
pub fn reserved(k: KeyCode) -> bool {
    to_key(k).is_some_and(|k| k.reserved())
}

pub fn key_name(k: KeyCode) -> Option<&'static str> {
    to_key(k).map(|k| k.name())
}

pub fn key_from_name(name: &str) -> Option<KeyCode> {
    model::Key::from_name(name).map(to_code)
}

/// A key that can be given to an action.
pub fn bindable(k: KeyCode) -> bool {
    to_key(k).is_some_and(|k| k.bindable())
}

/// The bindings: per action a key and an alternate one, as winit keys (the model's
/// `Bindings`, translated). Kept in order - an action with one key has it in the first place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeyBinds {
    pub keys: [[Option<KeyCode>; 2]; COUNT],
}

impl Default for KeyBinds {
    fn default() -> Self {
        KeyBinds::from_model(&model::Bindings::default())
    }
}

impl KeyBinds {
    pub fn from_model(b: &model::Bindings) -> KeyBinds {
        KeyBinds { keys: b.keys.map(|a| a.map(|k| k.map(to_code))) }
    }

    /// The model's bindings (a key here always has a name: only named keys get in).
    pub fn model(&self) -> model::Bindings {
        model::Bindings { keys: self.keys.map(|a| a.map(|k| k.and_then(to_key))) }
    }

    /// One of the action's keys is down.
    pub fn down(&self, act: usize, is_down: impl Fn(KeyCode) -> bool) -> bool {
        self.keys[act].iter().flatten().any(|k| is_down(*k))
    }

    /// The action a key belongs to.
    pub fn action_of(&self, k: KeyCode) -> Option<usize> {
        self.keys.iter().position(|a| a.contains(&Some(k)))
    }

    /// A key for an action's first (`slot` 0) or alternate place. False = refused (a
    /// reserved key or one without a name), nothing changed. Whatever else had the key
    /// loses it.
    pub fn assign(&mut self, act: usize, slot: usize, k: KeyCode) -> bool {
        let Some(key) = to_key(k) else { return false };
        let mut m = self.model();
        if !m.assign(act, slot, key) {
            return false;
        }
        *self = KeyBinds::from_model(&m);
        true
    }

    pub fn clear(&mut self, act: usize, slot: usize) {
        let mut m = self.model();
        m.clear(act, slot);
        *self = KeyBinds::from_model(&m);
    }

    /// Manual gears: both shifts have a key.
    pub fn manual(&self) -> bool {
        self.model().manual()
    }

    /// An action without a key that should have one: the steering and the pedals - and a
    /// shift while the other one is bound (one shift alone shifts nothing).
    pub fn must(&self, act: usize) -> bool {
        self.model().must(act)
    }

    pub fn unmapped(&self) -> bool {
        self.model().unmapped()
    }

    /// For the save: nothing while the table is the default one; else every action by its
    /// name with its keys' names.
    pub fn to_save(&self) -> Option<BTreeMap<String, Vec<String>>> {
        self.model().to_save()
    }

    /// From the save: no field = the defaults (the model's rules for a field by hand).
    pub fn from_save(saved: Option<&BTreeMap<String, Vec<String>>>) -> KeyBinds {
        KeyBinds::from_model(&model::Bindings::from_save(saved))
    }
}

/// The keyboard page's state, beside the pad editor's (`mc4::CtrlUi`).
#[derive(Clone, Debug, Default)]
pub struct KbdUi {
    /// the bindings the game plays with, and the page's working copy (committed with the
    /// pad's by "SAVE OPTIONS")
    pub binds: KeyBinds,
    pub work: KeyBinds,
    /// "PRESS A KEY": the next key is the row's
    pub polling: bool,
    /// the keys pressed since the last frame while the page is up (the window's loop fills it)
    pub queue: Vec<KeyCode>,
    /// a shift asked for by a key (+1 up, -1 down), and the last one with its time
    pub shift: i8,
    pub shift_last: Option<(i8, std::time::Instant)>,
}

impl KbdUi {
    /// A shift key was pressed: one the other way within 0.4 s of the last is passed over,
    /// as the pad's is (sub_822A2988 at 0x822A352C).
    pub fn shift_press(&mut self, way: i8) {
        if !self.binds.manual() {
            return;
        }
        if !self.shift_last.map_or(false, |(w, t)| w != way && t.elapsed().as_secs_f32() < 0.4) {
            self.shift = way;
            self.shift_last = Some((way, std::time::Instant::now()));
        }
    }

    /// The keyboard's part of "manual" and "shift" of a frame's driving input: (manual, shift).
    pub fn take_shift(&mut self) -> (bool, i8) {
        let s = std::mem::take(&mut self.shift);
        (self.binds.manual(), if self.binds.manual() { s } else { 0 })
    }
}

/// The keys the test hook presses this frame of the page: the steps `frame:key=NAME` of
/// MCLA_CTRL_TEST (the pad editor's hook; NAME as on the page, with `_` for a space).
pub fn hook_keys(frame: u32) -> Vec<KeyCode> {
    let Ok(script) = std::env::var("MCLA_CTRL_TEST") else { return Vec::new() };
    script.split(',').filter_map(|s| s.trim().split_once(':')).filter(|(at, _)| at.trim().parse::<u32>().ok() == Some(frame)).filter_map(|(_, w)| w.trim().strip_prefix("key=")).filter_map(|n| key_from_name(&n.replace('_', " "))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A save edited by the launcher (`mcla_config::keybinds::save`) is byte for byte what the game
    /// writes itself (`write_save`: pretty JSON of `SaveData`, `keyboard` last) and reads back.
    #[test]
    fn keyboard_field_written_by_the_launcher_is_the_games() {
        use crate::game::SaveData;
        let d = std::env::temp_dir().join(format!("mcla-kbd-launcher-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("mcla_save.json");
        let plain = serde_json::to_string_pretty(&SaveData::default()).unwrap();
        std::fs::write(&p, &plain).unwrap();
        let mut b = KeyBinds::default();
        assert!(b.assign(NITRO, 0, K::ShiftRight) && b.assign(HORN, 0, K::KeyE));
        model::save::SaveWriter::new().write(&p, &b.model()).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        let game = serde_json::to_string_pretty(&SaveData { keyboard: b.to_save(), ..Default::default() }).unwrap();
        assert_eq!(text, game);
        let back: SaveData = serde_json::from_str(&text).unwrap();
        assert_eq!(KeyBinds::from_save(back.keyboard.as_ref()), b);
        // (back at the defaults the launcher removes the member, as the game does)
        model::save::SaveWriter::new().write(&p, &KeyBinds::default().model()).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), plain);
        let _ = std::fs::remove_dir_all(d);
    }

    /// winit's names are the model's codes, key for key.
    #[test]
    fn keyboard_adapter_codes_match_the_model() {
        for (i, c) in CODES.iter().enumerate() {
            assert_eq!(format!("{c:?}"), model::KEYS[i].code);
            assert_eq!(to_key(*c).map(|k| k.index()), Some(i));
        }
        assert_eq!((to_key(K::PrintScreen), to_key(K::SuperLeft), bindable(K::AltLeft), reserved(K::Enter)), (None, None, true, true));
    }

    /// The defaults are the keys the port had hard-coded.
    #[test]
    fn keyboard_defaults_are_the_old_keys() {
        let b = KeyBinds::default();
        let want: [(usize, &[KeyCode]); COUNT] = [
            (STEER_LEFT, &[K::KeyA, K::ArrowLeft]), (STEER_RIGHT, &[K::KeyD, K::ArrowRight]), (GAS, &[K::KeyW, K::ArrowUp]), (BRAKE, &[K::KeyS, K::ArrowDown]),
            (SHIFT_UP, &[]), (SHIFT_DOWN, &[]), (HANDBRAKE, &[K::Space]), (WEIGHT_XFER, &[K::KeyT]), (NITRO, &[K::ShiftLeft]), (HEADLIGHTS, &[K::KeyF]),
            (HORN, &[K::KeyH]), (IN_ZONE, &[K::KeyE]), (ABILITY_NEXT, &[K::KeyQ]), (CAM_UP, &[K::KeyV]), (CAM_DOWN, &[]), (LOOKBACK, &[K::KeyC]),
            (MAP, &[K::KeyO]), (GPS_MAP, &[K::KeyM, K::Tab]), (MUSIC_UP, &[K::KeyN]), (MUSIC_DOWN, &[]), (MUSIC_PAUSE, &[K::KeyB]), (HYDRAULICS, &[K::KeyL]),
            (WHEEL_FRONT_LEFT, &[K::KeyU]), (WHEEL_FRONT_RIGHT, &[K::KeyI]), (WHEEL_BACK_LEFT, &[K::KeyJ]), (WHEEL_BACK_RIGHT, &[K::KeyK]),
            (CONVERTIBLE_UP_DN, &[]), (LEAN_BACK, &[K::KeyX]), (LEAN_FORWARD, &[K::KeyY]), (PAUSE, &[]), (GARAGE, &[K::KeyG]), (RESET_CAR, &[K::KeyR]),
        ];
        for (i, (act, keys)) in want.iter().enumerate() {
            assert_eq!(i, *act);
            let have: Vec<KeyCode> = b.keys[*act].iter().flatten().copied().collect();
            assert_eq!(&have, keys, "{}", ACTS[*act].name);
        }
        // (no key twice, none reserved, every one has a name; names are unique and found again)
        let all: Vec<KeyCode> = b.keys.iter().flatten().flatten().copied().collect();
        for (i, k) in all.iter().enumerate() {
            assert!(bindable(*k) && !all[..i].contains(k), "{:?}", k);
        }
        for (i, (k, n)) in KEYS.iter().enumerate() {
            assert_eq!(key_from_name(n), Some(*k));
            assert!(!KEYS[..i].iter().any(|e| e.0 == *k || e.1 == *n), "{n}");
        }
        for (i, a) in ACTS.iter().enumerate() {
            assert!(!ACTS[..i].iter().any(|x| x.name == a.name));
        }
        // (automatic gears, nothing missing, nothing for the save)
        assert!(!b.manual() && !b.unmapped() && b.to_save().is_none());
        // (reading: either key; the key's action)
        assert!(b.down(GAS, |k| k == K::ArrowUp) && b.down(GAS, |k| k == K::KeyW) && !b.down(GAS, |k| k == K::KeyS));
        assert_eq!((b.action_of(K::Tab), b.action_of(K::KeyE), b.action_of(K::KeyP)), (Some(GPS_MAP), Some(IN_ZONE), None));
        assert_eq!(rows().len(), COUNT + 5);
    }

    #[test]
    fn keyboard_rebinding_conflicts_reserved() {
        let mut b = KeyBinds::default();
        // (a free key)
        assert!(b.assign(NITRO, 0, K::ControlLeft));
        assert_eq!((b.keys[NITRO], b.action_of(K::ShiftLeft)), ([Some(K::ControlLeft), None], None));
        // (a key another action has: that one loses it - no swap)
        assert!(b.assign(HORN, 0, K::KeyE));
        assert_eq!((b.keys[HORN][0], b.keys[IN_ZONE], b.action_of(K::KeyH)), (Some(K::KeyE), [None, None], None));
        // (the first key of an action with two goes: the alternate one moves up)
        assert!(b.assign(HANDBRAKE, 0, K::KeyW));
        assert_eq!((b.keys[GAS], b.keys[HANDBRAKE][0]), ([Some(K::ArrowUp), None], Some(K::KeyW)));
        // (the action's own other key into the alternate place: one key, once)
        assert!(b.assign(BRAKE, 1, K::KeyS));
        assert_eq!(b.keys[BRAKE], [Some(K::KeyS), None]);
        // (reserved keys and keys without a name: refused, nothing changes)
        let was = b;
        for k in [K::Escape, K::Enter, K::NumpadEnter, K::Backspace, K::Delete, K::F1, K::F2, K::F5, K::F7, K::F12, K::PrintScreen, K::SuperLeft] {
            assert!(!b.assign(HORN, 0, k), "{:?}", k);
        }
        assert_eq!(b, was);
        // (must be mapped: the steering and the pedals; one shift alone asks for the other)
        b.clear(GAS, 0);
        assert!(b.must(GAS) && b.unmapped() && !b.must(HANDBRAKE));
        assert!(b.assign(GAS, 0, K::KeyW) && !b.unmapped());
        b.clear(HANDBRAKE, 0);
        assert!(!b.unmapped());
        assert!(b.assign(SHIFT_UP, 0, K::KeyP));
        assert!(!b.manual() && b.must(SHIFT_DOWN) && b.unmapped());
        assert!(b.assign(SHIFT_DOWN, 0, K::Semicolon));
        assert!(b.manual() && !b.unmapped());
        // (the shifts: only when manual; the other way within 0.4 s is passed over)
        let mut ui = KbdUi::default();
        ui.shift_press(1);
        assert_eq!(ui.take_shift(), (false, 0));
        ui.binds = b;
        ui.shift_press(1);
        assert_eq!(ui.take_shift(), (true, 1));
        ui.shift_press(-1);
        assert_eq!(ui.take_shift(), (true, 0));
        ui.shift_press(1);
        assert_eq!(ui.take_shift(), (true, 1));
        ui.shift_last = Some((1, std::time::Instant::now() - std::time::Duration::from_millis(500)));
        ui.shift_press(-1);
        assert_eq!(ui.take_shift(), (true, -1));
    }

    #[test]
    fn keyboard_save_round_trip() {
        use crate::game::SaveData;
        // (an old save: no field - the defaults; and the defaults write no field)
        let old = serde_json::to_string(&SaveData::default()).unwrap();
        assert!(!old.contains("keyboard"));
        let sd: SaveData = serde_json::from_str(&old).unwrap();
        assert!(sd.keyboard.is_none());
        assert_eq!(KeyBinds::from_save(sd.keyboard.as_ref()), KeyBinds::default());
        // (changed: every action by name, there and back)
        let mut b = KeyBinds::default();
        assert!(b.assign(NITRO, 0, K::ControlLeft) && b.assign(SHIFT_UP, 0, K::KeyP) && b.assign(SHIFT_DOWN, 0, K::Semicolon) && b.assign(HORN, 0, K::KeyE));
        b.clear(GPS_MAP, 0);
        let sd = SaveData { keyboard: b.to_save(), ..Default::default() };
        let t = serde_json::to_string(&sd).unwrap();
        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
        assert_eq!(v["keyboard"].as_object().unwrap().len(), COUNT);
        assert_eq!(v["keyboard"]["NITRO"], serde_json::json!(["LEFT CTRL"]));
        assert_eq!(v["keyboard"]["GAS"], serde_json::json!(["W", "UP"]));
        assert_eq!(v["keyboard"]["IN_ZONE"], serde_json::json!([]));
        assert_eq!(v["keyboard"]["GPS_MAP"], serde_json::json!(["TAB"]));
        let back: SaveData = serde_json::from_str(&t).unwrap();
        assert_eq!(KeyBinds::from_save(back.keyboard.as_ref()), b);
        // (a field by hand: an action left out keeps its default unless a named one has the
        // key; unknown, reserved and repeated keys are dropped - a key given twice stays with the
        // action that comes first in the table - and no more than two keys are kept)
        let hand: BTreeMap<String, Vec<String>> = serde_json::from_value(serde_json::json!({
            "HORN": ["w", "ESCAPE", "NO SUCH KEY"], "NITRO": ["W", "LEFT CTRL", "Z"], "IN_ZONE": [], "NOT_AN_ACTION": ["P"]
        })).unwrap();
        let h = KeyBinds::from_save(Some(&hand));
        assert_eq!((h.keys[HORN], h.keys[NITRO], h.keys[IN_ZONE]), ([None, None], [Some(K::KeyW), Some(K::ControlLeft)], [None, None]));
        assert_eq!((h.keys[GAS], h.keys[BRAKE], h.action_of(K::KeyP), h.action_of(K::KeyH)), ([Some(K::ArrowUp), None], [Some(K::KeyS), Some(K::ArrowDown)], None, None));
    }
}
