//! The keyboard's bindings - the port's own addition (DECISIONS.md D1; the original is a console game and has no
//! keyboard). Shared by the game (its OPTIONS > CONTROLLER > KEYBOARD page and its driving code, through the thin
//! winit adapter in `crates/game/src/keybinds.rs`) and the launcher's Controls tab, so both edit the same data
//! (docs/LAUNCHER.md decision 8). No winit here: a key is a [`Key`], named by its W3C `KeyboardEvent.code` string
//! ("KeyW", "ArrowUp", "Space" - the same words as winit's `KeyCode` variants) and, in the save and on the game's
//! page, by its page name ("W", "UP", "SPACE").
//!
//! The rules:
//! - reserved keys can never be bound: Escape, Enter, the numpad's Enter (the menus' back and accept), Backspace and
//!   Delete (CLEAR on the page; Backspace is also a key of the map and of the replay), and F1..F12 (the developer keys
//!   F1, F2, F7, F12 = screenshot, and F5 = DEFAULTS on the page). Alt+Enter (window mode) is fixed too: Enter is
//!   reserved, so the combination can never reach an action. A key without a name in [`KEYS`] cannot be bound either;
//! - a key does one thing: binding a key takes it off whatever had it (as the pad's editor empties the other row -
//!   no swap);
//! - manual gears: SHIFT_UP and SHIFT_DOWN are unbound by default (automatic); with both bound the keyboard is
//!   manual - the pad's rule (both shifts in its map).
//!
//! In the save (`SaveData::keyboard`, outside the `mc4` block): every action by name with its keys' page names,
//! written only while the table differs from the defaults. [`save`] reads and edits that member of a save file
//! for the launcher without touching anything else.

pub mod save;

use std::collections::BTreeMap;

/// The actions, by their place in [`ACTS`] (stable: the save names them by [`ActDef::name`]).
pub const STEER_LEFT: usize = 0;
pub const STEER_RIGHT: usize = 1;
pub const GAS: usize = 2;
pub const BRAKE: usize = 3;
pub const SHIFT_UP: usize = 4;
pub const SHIFT_DOWN: usize = 5;
pub const HANDBRAKE: usize = 6;
pub const WEIGHT_XFER: usize = 7;
pub const NITRO: usize = 8;
pub const HEADLIGHTS: usize = 9;
pub const HORN: usize = 10;
pub const IN_ZONE: usize = 11;
pub const ABILITY_NEXT: usize = 12;
pub const CAM_UP: usize = 13;
pub const CAM_DOWN: usize = 14;
pub const LOOKBACK: usize = 15;
pub const MAP: usize = 16;
pub const GPS_MAP: usize = 17;
pub const MUSIC_UP: usize = 18;
pub const MUSIC_DOWN: usize = 19;
pub const MUSIC_PAUSE: usize = 20;
pub const HYDRAULICS: usize = 21;
pub const WHEEL_FRONT_LEFT: usize = 22;
pub const WHEEL_FRONT_RIGHT: usize = 23;
pub const WHEEL_BACK_LEFT: usize = 24;
pub const WHEEL_BACK_RIGHT: usize = 25;
pub const CONVERTIBLE_UP_DN: usize = 26;
pub const LEAN_BACK: usize = 27;
pub const LEAN_FORWARD: usize = 28;
pub const PAUSE: usize = 29;
pub const GARAGE: usize = 30;
pub const RESET_CAR: usize = 31;
pub const COUNT: usize = 32;

/// A bindable or reserved key: its place in [`KEYS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key(u8);

/// A key's two names: the W3C `KeyboardEvent.code` (= winit's `KeyCode` variant) and the name on the game's page and in
/// the save.
#[derive(Clone, Copy, Debug)]
pub struct KeyDef {
    pub code: &'static str,
    pub name: &'static str,
}

const fn kd(code: &'static str, name: &'static str) -> KeyDef {
    KeyDef { code, name }
}

/// Every key that has a name, the reserved ones included (they have names for the hooks and for telling "reserved"
/// from "unknown"). The order is fixed: a [`Key`] is an index into it.
pub const KEYS: [KeyDef; 98] = [
    kd("KeyA", "A"), kd("KeyB", "B"), kd("KeyC", "C"), kd("KeyD", "D"), kd("KeyE", "E"), kd("KeyF", "F"), kd("KeyG", "G"), kd("KeyH", "H"), kd("KeyI", "I"),
    kd("KeyJ", "J"), kd("KeyK", "K"), kd("KeyL", "L"), kd("KeyM", "M"), kd("KeyN", "N"), kd("KeyO", "O"), kd("KeyP", "P"), kd("KeyQ", "Q"), kd("KeyR", "R"),
    kd("KeyS", "S"), kd("KeyT", "T"), kd("KeyU", "U"), kd("KeyV", "V"), kd("KeyW", "W"), kd("KeyX", "X"), kd("KeyY", "Y"), kd("KeyZ", "Z"),
    kd("Digit0", "0"), kd("Digit1", "1"), kd("Digit2", "2"), kd("Digit3", "3"), kd("Digit4", "4"), kd("Digit5", "5"), kd("Digit6", "6"), kd("Digit7", "7"),
    kd("Digit8", "8"), kd("Digit9", "9"),
    kd("ArrowUp", "UP"), kd("ArrowDown", "DOWN"), kd("ArrowLeft", "LEFT"), kd("ArrowRight", "RIGHT"),
    kd("Space", "SPACE"), kd("Tab", "TAB"), kd("CapsLock", "CAPS LOCK"),
    kd("ShiftLeft", "LEFT SHIFT"), kd("ShiftRight", "RIGHT SHIFT"), kd("ControlLeft", "LEFT CTRL"), kd("ControlRight", "RIGHT CTRL"),
    kd("AltLeft", "LEFT ALT"), kd("AltRight", "RIGHT ALT"),
    kd("Insert", "INSERT"), kd("Home", "HOME"), kd("End", "END"), kd("PageUp", "PAGE UP"), kd("PageDown", "PAGE DOWN"),
    kd("Minus", "MINUS"), kd("Equal", "EQUALS"), kd("BracketLeft", "LEFT BRACKET"), kd("BracketRight", "RIGHT BRACKET"), kd("Backslash", "BACKSLASH"),
    kd("Semicolon", "SEMICOLON"), kd("Quote", "QUOTE"), kd("Comma", "COMMA"), kd("Period", "PERIOD"), kd("Slash", "SLASH"), kd("Backquote", "BACKQUOTE"),
    kd("IntlBackslash", "INTL BACKSLASH"),
    kd("Numpad0", "NUM 0"), kd("Numpad1", "NUM 1"), kd("Numpad2", "NUM 2"), kd("Numpad3", "NUM 3"), kd("Numpad4", "NUM 4"), kd("Numpad5", "NUM 5"),
    kd("Numpad6", "NUM 6"), kd("Numpad7", "NUM 7"), kd("Numpad8", "NUM 8"), kd("Numpad9", "NUM 9"),
    kd("NumpadAdd", "NUM PLUS"), kd("NumpadSubtract", "NUM MINUS"), kd("NumpadMultiply", "NUM MULTIPLY"), kd("NumpadDivide", "NUM DIVIDE"),
    kd("NumpadDecimal", "NUM DECIMAL"),
    // (reserved from here on: the menus' back / accept, CLEAR, the function keys)
    kd("Escape", "ESCAPE"), kd("Enter", "ENTER"), kd("NumpadEnter", "NUM ENTER"), kd("Backspace", "BACKSPACE"), kd("Delete", "DELETE"),
    kd("F1", "F1"), kd("F2", "F2"), kd("F3", "F3"), kd("F4", "F4"), kd("F5", "F5"), kd("F6", "F6"), kd("F7", "F7"), kd("F8", "F8"), kd("F9", "F9"),
    kd("F10", "F10"), kd("F11", "F11"), kd("F12", "F12"),
];

/// Where the reserved keys start in [`KEYS`].
const FIRST_RESERVED: usize = 81;

impl Key {
    /// The key at a place of [`KEYS`].
    pub fn at(i: usize) -> Option<Key> {
        (i < KEYS.len()).then_some(Key(i as u8))
    }

    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// The W3C `KeyboardEvent.code` (winit's `KeyCode` variant name).
    pub fn code(self) -> &'static str {
        KEYS[self.index()].code
    }

    /// The name on the game's page and in the save.
    pub fn name(self) -> &'static str {
        KEYS[self.index()].name
    }

    pub fn from_code(code: &str) -> Option<Key> {
        KEYS.iter().position(|k| k.code == code).map(|i| Key(i as u8))
    }

    /// By page / save name, case and surrounding blanks ignored (the save and the test hooks).
    pub fn from_name(name: &str) -> Option<Key> {
        let name = name.trim();
        KEYS.iter().position(|k| k.name.eq_ignore_ascii_case(name)).map(|i| Key(i as u8))
    }

    /// A key no action may have (see the rules at the top).
    pub fn reserved(self) -> bool {
        self.index() >= FIRST_RESERVED
    }

    /// A key that can be given to an action.
    pub fn bindable(self) -> bool {
        !self.reserved()
    }
}

/// Alt+Enter (either Enter, either Alt): the window-mode key of the game (windowed -> borderless -> fullscreen). It is
/// fixed: never an action's, whatever the table says.
pub fn alt_enter(alt: bool, key: Key) -> bool {
    alt && matches!(key.code(), "Enter" | "NumpadEnter")
}

/// The keys the game keeps for itself, for the lists on the game's page and the launcher's FIXED KEYS card: (id, the
/// keys as text). The id is the launcher's locale key suffix (`ctl.fixed.<id>`); the order is the display order.
pub const FIXED: &[(&str, &str)] = &[
    ("back", "Esc"),
    ("accept", "Enter"),
    ("window_mode", "Alt+Enter"),
    ("clear", "Backspace / Delete"),
    ("screenshot", "F12"),
    ("developer", "F1 F2 F7"),
];

/// An action: its name (the pad map's where the pad has the action - then the name is also the game's text key, as in
/// the pad's editor), the port's own label, the default keys by `code`, "must be mapped", and whether the pad's maps
/// have an action of this name.
pub struct ActDef {
    pub name: &'static str,
    pub label: &'static str,
    pub keys: [Option<&'static str>; 2],
    pub must: bool,
    pub shared: bool,
}

const fn act(name: &'static str, label: &'static str, a: Option<&'static str>, b: Option<&'static str>, must: bool, shared: bool) -> ActDef {
    ActDef { name, label, keys: [a, b], must, shared }
}

/// The table. The defaults are the keys of the port before it had a table.
pub const ACTS: [ActDef; COUNT] = [
    act("STEER_LEFT", "STEER LEFT", Some("KeyA"), Some("ArrowLeft"), true, false),
    act("STEER_RIGHT", "STEER RIGHT", Some("KeyD"), Some("ArrowRight"), true, false),
    act("GAS", "GAS", Some("KeyW"), Some("ArrowUp"), true, true),
    act("BRAKE", "BRAKE/REVERSE", Some("KeyS"), Some("ArrowDown"), true, true),
    act("SHIFT_UP", "SHIFT UP", None, None, false, true),
    act("SHIFT_DOWN", "SHIFT DOWN", None, None, false, true),
    act("HANDBRAKE", "HANDBRAKE", Some("Space"), None, false, true),
    act("WEIGHT_XFER", "WEIGHT TRANSFER", Some("KeyT"), None, false, true),
    act("NITRO", "NITROUS", Some("ShiftLeft"), None, false, true),
    act("HEADLIGHTS", "HEADLIGHTS", Some("KeyF"), None, false, true),
    act("HORN", "HORN", Some("KeyH"), None, false, true),
    act("IN_ZONE", "SPECIAL ABILITY", Some("KeyE"), None, false, true),
    act("ABILITY_NEXT", "NEXT ABILITY", Some("KeyQ"), None, false, false),
    act("CAM_UP", "CHANGE CAMERA", Some("KeyV"), None, false, true),
    act("CAM_DOWN", "PREVIOUS CAMERA", None, None, false, false),
    act("LOOKBACK", "LOOK BACK", Some("KeyC"), None, false, true),
    act("MAP", "MAP", Some("KeyO"), None, false, true),
    act("GPS_MAP", "GPS MAP", Some("KeyM"), Some("Tab"), false, false),
    act("MUSIC_UP", "NEXT SONG", Some("KeyN"), None, false, true),
    act("MUSIC_DOWN", "PREVIOUS SONG", None, None, false, true),
    act("MUSIC_PAUSE", "MUSIC ON/OFF", Some("KeyB"), None, false, false),
    act("HYDRAULICS", "HYDRAULICS", Some("KeyL"), None, false, true),
    act("WHEEL_FRONT_LEFT", "HYDRAULICS FRONT LEFT", Some("KeyU"), None, false, false),
    act("WHEEL_FRONT_RIGHT", "HYDRAULICS FRONT RIGHT", Some("KeyI"), None, false, false),
    act("WHEEL_BACK_LEFT", "HYDRAULICS BACK LEFT", Some("KeyJ"), None, false, false),
    act("WHEEL_BACK_RIGHT", "HYDRAULICS BACK RIGHT", Some("KeyK"), None, false, false),
    act("CONVERTIBLE_UP_DN", "CONVERTIBLE TOP", None, None, false, true),
    act("LEAN_BACK", "LEAN BACK", Some("KeyX"), None, false, false),
    act("LEAN_FORWARD", "LEAN FORWARD", Some("KeyY"), None, false, false),
    act("PAUSE", "PAUSE", None, None, false, false),
    act("GARAGE", "GO TO GARAGE", Some("KeyG"), None, false, false),
    act("RESET_CAR", "RESET VEHICLE", Some("KeyR"), None, false, false),
];

/// An action by its name in the save.
pub fn action_by_name(name: &str) -> Option<usize> {
    ACTS.iter().position(|a| a.name == name)
}

/// The bindings: per action a key and an alternate one. Kept in order - an action with one key has it in the first
/// place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bindings {
    pub keys: [[Option<Key>; 2]; COUNT],
}

impl Default for Bindings {
    fn default() -> Self {
        let mut keys = [[None; 2]; COUNT];
        for (i, a) in ACTS.iter().enumerate() {
            for s in 0..2 {
                keys[i][s] = a.keys[s].and_then(Key::from_code);
            }
        }
        Bindings { keys }
    }
}

impl Bindings {
    /// No key anywhere (the start of [`Bindings::from_save`]).
    pub fn empty() -> Bindings {
        Bindings { keys: [[None; 2]; COUNT] }
    }

    /// One of the action's keys is down.
    pub fn down(&self, act: usize, is_down: impl Fn(Key) -> bool) -> bool {
        self.keys[act].iter().flatten().any(|k| is_down(*k))
    }

    /// The action a key belongs to.
    pub fn action_of(&self, k: Key) -> Option<usize> {
        self.keys.iter().position(|a| a.contains(&Some(k)))
    }

    /// Where a key is now (action, slot).
    pub fn place_of(&self, k: Key) -> Option<(usize, usize)> {
        (0..COUNT).find_map(|a| (0..2).find(|&s| self.keys[a][s] == Some(k)).map(|s| (a, s)))
    }

    /// The place that would lose the key if it were given to (`act`, `slot`): another action's, or the action's own
    /// other slot. None = the key is free or already there.
    pub fn conflict(&self, act: usize, slot: usize, k: Key) -> Option<(usize, usize)> {
        self.place_of(k).filter(|&p| p != (act, slot))
    }

    pub fn bound(&self, act: usize) -> bool {
        self.keys[act][0].is_some() || self.keys[act][1].is_some()
    }

    fn tidy(&mut self, act: usize) {
        if self.keys[act][0].is_none() {
            self.keys[act].swap(0, 1);
        }
    }

    /// A key for an action's first (`slot` 0) or alternate place. False = refused (a reserved key), nothing changed.
    /// Whatever else had the key loses it.
    pub fn assign(&mut self, act: usize, slot: usize, k: Key) -> bool {
        if !k.bindable() || act >= COUNT || slot > 1 {
            return false;
        }
        for a in 0..COUNT {
            for s in 0..2 {
                if self.keys[a][s] == Some(k) {
                    self.keys[a][s] = None;
                }
            }
        }
        self.keys[act][slot] = Some(k);
        for a in 0..COUNT {
            self.tidy(a);
        }
        true
    }

    pub fn clear(&mut self, act: usize, slot: usize) {
        if act < COUNT && slot < 2 {
            self.keys[act][slot] = None;
            self.tidy(act);
        }
    }

    /// Manual gears: both shifts have a key.
    pub fn manual(&self) -> bool {
        self.bound(SHIFT_UP) && self.bound(SHIFT_DOWN)
    }

    /// An action without a key that should have one: the steering and the pedals - and a shift while the other one
    /// is bound (one shift alone shifts nothing).
    pub fn must(&self, act: usize) -> bool {
        if self.bound(act) {
            return false;
        }
        ACTS[act].must || (act == SHIFT_UP && self.bound(SHIFT_DOWN)) || (act == SHIFT_DOWN && self.bound(SHIFT_UP))
    }

    pub fn unmapped(&self) -> bool {
        (0..COUNT).any(|a| self.must(a))
    }

    /// For the save: nothing while the table is the default one; else every action by its name with its keys' names.
    pub fn to_save(&self) -> Option<BTreeMap<String, Vec<String>>> {
        if *self == Bindings::default() {
            return None;
        }
        Some(ACTS.iter().enumerate().map(|(i, a)| (a.name.to_string(), self.keys[i].iter().flatten().map(|k| k.name().to_string()).collect())).collect())
    }

    /// From the save: no field = the defaults. An action the field does not name keeps its default keys - unless a
    /// named action has them; unknown and reserved keys, and a key given twice, are dropped (a key given twice stays
    /// with the action that comes first in the table), and no more than two keys are kept.
    pub fn from_save(saved: Option<&BTreeMap<String, Vec<String>>>) -> Bindings {
        let Some(saved) = saved else { return Bindings::default() };
        let mut out = Bindings::empty();
        let mut used: Vec<Key> = Vec::new();
        for named in [true, false] {
            for (i, a) in ACTS.iter().enumerate() {
                let keys: Vec<Key> = match (saved.get(a.name), named) {
                    (Some(names), true) => names.iter().filter_map(|n| Key::from_name(n)).collect(),
                    (None, false) => a.keys.iter().flatten().filter_map(|c| Key::from_code(c)).collect(),
                    _ => continue,
                };
                let mut at = 0;
                for k in keys {
                    if at < 2 && k.bindable() && !used.contains(&k) {
                        out.keys[i][at] = Some(k);
                        used.push(k);
                        at += 1;
                    }
                }
            }
        }
        out
    }
}

/// The game page's rows: every action, and after it its alternate key where the default table has one (the steering,
/// the pedals, the GPS map) - (action, place).
pub fn rows() -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for (i, a) in ACTS.iter().enumerate() {
        out.push((i, 0));
        if a.keys[1].is_some() {
            out.push((i, 1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(code: &str) -> Key {
        Key::from_code(code).unwrap_or_else(|| panic!("{code}"))
    }

    /// The defaults are the keys the port had hard-coded; names and codes are unique.
    #[test]
    fn defaults_and_names() {
        let b = Bindings::default();
        let want: [&[&str]; COUNT] = [
            &["KeyA", "ArrowLeft"], &["KeyD", "ArrowRight"], &["KeyW", "ArrowUp"], &["KeyS", "ArrowDown"], &[], &[], &["Space"], &["KeyT"], &["ShiftLeft"], &["KeyF"],
            &["KeyH"], &["KeyE"], &["KeyQ"], &["KeyV"], &[], &["KeyC"], &["KeyO"], &["KeyM", "Tab"], &["KeyN"], &[], &["KeyB"], &["KeyL"],
            &["KeyU"], &["KeyI"], &["KeyJ"], &["KeyK"], &[], &["KeyX"], &["KeyY"], &[], &["KeyG"], &["KeyR"],
        ];
        for (a, codes) in want.iter().enumerate() {
            let have: Vec<&str> = b.keys[a].iter().flatten().map(|k| k.code()).collect();
            assert_eq!(&have, codes, "{}", ACTS[a].name);
            // (every default code names a key)
            assert_eq!(ACTS[a].keys.iter().flatten().count(), codes.len());
        }
        for (i, d) in KEYS.iter().enumerate() {
            assert_eq!((Key::from_code(d.code), Key::from_name(d.name)), (Key::at(i), Key::at(i)));
            assert!(!KEYS[..i].iter().any(|e| e.code == d.code || e.name == d.name), "{}", d.name);
            assert_eq!(Key(i as u8).reserved(), i >= FIRST_RESERVED, "{}", d.code);
        }
        assert_eq!(KEYS[FIRST_RESERVED].code, "Escape");
        for (i, a) in ACTS.iter().enumerate() {
            assert!(!ACTS[..i].iter().any(|x| x.name == a.name));
            assert_eq!(action_by_name(a.name), Some(i));
        }
        let all: Vec<Key> = b.keys.iter().flatten().flatten().copied().collect();
        for (i, k) in all.iter().enumerate() {
            assert!(k.bindable() && !all[..i].contains(k), "{:?}", k);
        }
        assert!(!b.manual() && !b.unmapped() && b.to_save().is_none());
        assert_eq!(rows().len(), COUNT + 5);
        assert_eq!(Key::from_name(" left ctrl "), Some(k("ControlLeft")));
        assert_eq!((Key::from_code("PrintScreen"), Key::from_code("MetaLeft"), Key::from_name("NO SUCH KEY")), (None, None, None));
    }

    #[test]
    fn rebinding_conflicts_reserved() {
        let mut b = Bindings::default();
        assert!(b.assign(NITRO, 0, k("ControlLeft")));
        assert_eq!((b.keys[NITRO], b.action_of(k("ShiftLeft"))), ([Some(k("ControlLeft")), None], None));
        // (the launcher asks first: who has E now)
        assert_eq!((b.conflict(HORN, 0, k("KeyE")), b.conflict(IN_ZONE, 0, k("KeyE")), b.conflict(HORN, 0, k("KeyP"))), (Some((IN_ZONE, 0)), None, None));
        assert_eq!(b.conflict(GAS, 0, k("ArrowUp")), Some((GAS, 1)));
        assert!(b.assign(HORN, 0, k("KeyE")));
        assert_eq!((b.keys[HORN][0], b.keys[IN_ZONE], b.action_of(k("KeyH"))), (Some(k("KeyE")), [None, None], None));
        assert!(b.assign(HANDBRAKE, 0, k("KeyW")));
        assert_eq!((b.keys[GAS], b.keys[HANDBRAKE][0]), ([Some(k("ArrowUp")), None], Some(k("KeyW"))));
        assert!(b.assign(BRAKE, 1, k("KeyS")));
        assert_eq!(b.keys[BRAKE], [Some(k("KeyS")), None]);
        let was = b;
        for c in ["Escape", "Enter", "NumpadEnter", "Backspace", "Delete", "F1", "F2", "F5", "F7", "F12"] {
            assert!(!b.assign(HORN, 0, k(c)), "{c}");
        }
        assert_eq!(b, was);
        // (Alt+Enter is fixed; Alt alone is an ordinary key)
        assert!(alt_enter(true, k("Enter")) && alt_enter(true, k("NumpadEnter")) && !alt_enter(false, k("Enter")) && !alt_enter(true, k("KeyE")));
        assert!(k("AltLeft").bindable() && !k("Enter").bindable());
        assert!(FIXED.iter().any(|f| f.1 == "Alt+Enter"));
        b.clear(GAS, 0);
        assert!(b.must(GAS) && b.unmapped() && !b.must(HANDBRAKE));
        assert!(b.assign(GAS, 0, k("KeyW")) && !b.unmapped());
        assert!(b.assign(SHIFT_UP, 0, k("KeyP")));
        assert!(!b.manual() && b.must(SHIFT_DOWN) && b.unmapped());
        assert!(b.assign(SHIFT_DOWN, 0, k("Semicolon")));
        assert!(b.manual() && !b.unmapped());
    }

    #[test]
    fn save_field_round_trip() {
        let mut b = Bindings::default();
        assert!(b.assign(NITRO, 0, k("ControlLeft")) && b.assign(SHIFT_UP, 0, k("KeyP")) && b.assign(SHIFT_DOWN, 0, k("Semicolon")) && b.assign(HORN, 0, k("KeyE")));
        b.clear(GPS_MAP, 0);
        let f = b.to_save().unwrap();
        assert_eq!(f.len(), COUNT);
        assert_eq!((f["NITRO"].clone(), f["GAS"].clone(), f["IN_ZONE"].clone(), f["GPS_MAP"].clone()), (vec!["LEFT CTRL".to_string()], vec!["W".into(), "UP".into()], vec![], vec!["TAB".into()]));
        assert_eq!(Bindings::from_save(Some(&f)), b);
        assert_eq!(Bindings::from_save(None), Bindings::default());
        let hand: BTreeMap<String, Vec<String>> = [("HORN", vec!["w", "ESCAPE", "NO SUCH KEY"]), ("NITRO", vec!["W", "LEFT CTRL", "Z"]), ("IN_ZONE", vec![]), ("NOT_AN_ACTION", vec!["P"])]
            .into_iter()
            .map(|(a, v)| (a.to_string(), v.into_iter().map(String::from).collect()))
            .collect();
        let h = Bindings::from_save(Some(&hand));
        assert_eq!((h.keys[HORN], h.keys[NITRO], h.keys[IN_ZONE]), ([None, None], [Some(k("KeyW")), Some(k("ControlLeft"))], [None, None]));
        assert_eq!((h.keys[GAS], h.keys[BRAKE], h.action_of(k("KeyP")), h.action_of(k("KeyH"))), ([Some(k("ArrowUp")), None], [Some(k("KeyS")), Some(k("ArrowDown"))], None, None));
    }
}
