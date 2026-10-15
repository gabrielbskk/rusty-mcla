//! Removes the Windows user name and the machine name from text and from file names before anything is exported.
//!
//! Rules (per name, case-insensitive):
//! * 4 or more characters: replaced wherever it occurs (so `QUINCY~1`, `Quincy-PC`, `%2FQuincy%2F` and JSON-escaped paths are covered);
//! * exactly 3 characters: replaced only as a whole word;
//! * shorter, or a very generic word (`user`, `test`, ...): only as a whole word *and* only the segment after `Users\` is touched
//!   for names under 3 characters, so `a` never eats every letter `a`.
//!
//! The URL-encoded forms of a name with a space (`%20`, `+`) are covered. The user is `<user>`, the machine is `<machine>`.

use std::collections::BTreeSet;

pub const USER: &str = "<user>";
pub const MACHINE: &str = "<machine>";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Anywhere,
    Bounded,
    AfterUsers,
}

#[derive(Clone, Debug)]
struct Rule {
    needle: Vec<char>, // lower-cased
    replacement: &'static str,
    mode: Mode,
}

#[derive(Clone, Debug, Default)]
pub struct Redactor {
    rules: Vec<Rule>,
}

/// Words that are common enough in logs that a user or machine of that name must not be replaced inside other words.
const GENERIC: &[&str] = &["user", "users", "test", "admin", "administrator", "game", "public", "default", "system", "windows", "local", "temp", "data", "home", "mcla", "owner", "guest"];

fn lower(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

fn lowered(s: &str) -> Vec<char> {
    s.chars().map(lower).collect()
}

fn variants(name: &str) -> Vec<String> {
    let mut v = vec![name.to_string()];
    if name.contains(' ') {
        v.push(name.replace(' ', "%20"));
        v.push(name.replace(' ', "+"));
    }
    v
}

impl Redactor {
    /// `users` and `machines` are the names to hide. Empty and whitespace-only names are ignored.
    pub fn new<S: AsRef<str>>(users: &[S], machines: &[S]) -> Redactor {
        let mut seen = BTreeSet::new();
        let mut rules = Vec::new();
        let mut add = |name: &str, replacement: &'static str| {
            let name = name.trim();
            if name.is_empty() || name.chars().any(|c| c.is_control()) {
                return;
            }
            for v in variants(name) {
                let needle = lowered(&v);
                if !seen.insert((needle.clone(), replacement)) {
                    continue;
                }
                let len = needle.len();
                let generic = GENERIC.contains(&v.to_lowercase().as_str());
                let mode = if len < 3 {
                    Mode::AfterUsers
                } else if len == 3 || generic {
                    Mode::Bounded
                } else {
                    Mode::Anywhere
                };
                rules.push(Rule { needle, replacement, mode });
                // a short or generic name is also hidden in `Users\<name>` where it is not a whole word (`Users\bob_work`)
                if mode == Mode::Bounded {
                    let again = rules.last().cloned().unwrap();
                    rules.push(Rule { mode: Mode::AfterUsers, ..again });
                }
            }
        };
        for u in users {
            add(u.as_ref(), USER);
        }
        for m in machines {
            add(m.as_ref(), MACHINE);
        }
        // the longest names first, so `Quincy.DESKTOP-1` wins over `Quincy`
        rules.sort_by(|a, b| b.needle.len().cmp(&a.needle.len()));
        Redactor { rules }
    }

    /// The names of the running user and machine (environment and the home folder's name).
    pub fn current() -> Redactor {
        let mut users: Vec<String> = Vec::new();
        for k in ["USERNAME", "USER", "LOGNAME"] {
            if let Ok(v) = std::env::var(k) {
                users.push(v);
            }
        }
        for k in ["USERPROFILE", "HOME"] {
            if let Some(n) = std::env::var_os(k).and_then(|v| std::path::PathBuf::from(v).file_name().map(|n| n.to_string_lossy().into_owned())) {
                users.push(n);
            }
        }
        if let Some(n) = dirs::home_dir().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())) {
            users.push(n);
        }
        let mut machines: Vec<String> = Vec::new();
        for k in ["COMPUTERNAME", "HOSTNAME", "USERDOMAIN"] {
            if let Ok(v) = std::env::var(k) {
                machines.push(v);
            }
        }
        // the domain of a PC that is not on a domain is the machine name itself; a real domain is not secret but is harmless to hide
        Redactor::new(&users, &machines)
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Replaces every occurrence in `s`. Returns the new text and how many replacements were made.
    pub fn text_counted(&self, s: &str) -> (String, usize) {
        if self.rules.is_empty() || s.is_empty() {
            return (s.to_string(), 0);
        }
        let chars: Vec<char> = s.chars().collect();
        let low: Vec<char> = chars.iter().map(|c| lower(*c)).collect();
        let mut out = String::with_capacity(s.len());
        let mut count = 0;
        let mut i = 0;
        'outer: while i < chars.len() {
            for r in &self.rules {
                let n = r.needle.len();
                if i + n > chars.len() || low[i] != r.needle[0] || low[i..i + n] != r.needle[..] {
                    continue;
                }
                let ok = match r.mode {
                    Mode::Anywhere => true,
                    Mode::Bounded => {
                        (i == 0 || !chars[i - 1].is_alphanumeric()) && (i + n == chars.len() || !chars[i + n].is_alphanumeric())
                    }
                    Mode::AfterUsers => after_users(&low, i),
                };
                if ok {
                    out.push_str(r.replacement);
                    i += n;
                    count += 1;
                    continue 'outer;
                }
            }
            out.push(chars[i]);
            i += 1;
        }
        (out, count)
    }

    pub fn text(&self, s: &str) -> String {
        self.text_counted(s).0
    }

    /// A name for a file inside the zip: redacted (`<user>` becomes `_user_`, because Windows cannot extract `<` or `>` in a file
    /// name) and reduced to a plain relative path with `/` separators.
    pub fn entry_name(&self, s: &str) -> String {
        let t = self.text(s).replace('\\', "/").replace(['<', '>'], "_");
        t.split('/').filter(|p| !p.is_empty() && *p != "." && *p != "..").collect::<Vec<_>>().join("/")
    }
}

/// True when the text before `i` ends with `users` and one to three separators (`\`, `/`, `\\`, `%5C`, `%2F`).
fn after_users(low: &[char], i: usize) -> bool {
    let mut j = i;
    let mut seps = 0;
    while j > 0 && seps < 6 {
        let c = low[j - 1];
        if c == '\\' || c == '/' {
            j -= 1;
            seps += 1;
        } else if j >= 3 && low[j - 3] == '%' && ((low[j - 2] == '5' && low[j - 1] == 'c') || (low[j - 2] == '2' && low[j - 1] == 'f')) {
            j -= 3;
            seps += 1;
        } else {
            break;
        }
    }
    seps > 0 && j >= 5 && low[j - 5..j] == ['u', 's', 'e', 'r', 's']
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r() -> Redactor {
        Redactor::new(&["Quincy", "Quincy.DESKTOP-9X1"], &["DESKTOP-9X1QZ"])
    }

    #[test]
    fn paths_in_every_spelling() {
        let r = r();
        assert_eq!(r.text(r"C:\Users\Quincy\AppData\Local\RustyMCLA\logs"), r"C:\Users\<user>\AppData\Local\RustyMCLA\logs");
        assert_eq!(r.text("C:/Users/quincy/AppData"), "C:/Users/<user>/AppData");
        assert_eq!(r.text(r#"{"path":"C:\\Users\\QUINCY\\saves"}"#), r#"{"path":"C:\\Users\\<user>\\saves"}"#);
        assert_eq!(r.text("file:///C:/Users/Quincy/x"), "file:///C:/Users/<user>/x");
        assert_eq!(r.text("C:\\PROGRA~1\\QUINCY~1\\x"), "C:\\PROGRA~1\\<user>~1\\x");
        assert_eq!(r.text(r"\\DESKTOP-9X1QZ\share\f"), r"\\<machine>\share\f");
        assert_eq!(r.text("welcome Quincy, you are on desktop-9x1qz"), "welcome <user>, you are on <machine>");
        // the longer profile folder name wins over the plain name
        assert_eq!(r.text(r"C:\Users\Quincy.DESKTOP-9X1\x"), r"C:\Users\<user>\x");
    }

    #[test]
    fn fake_profile_in_several_log_lines_leaves_no_trace() {
        let r = Redactor::new(&["Zelda Quinn"], &["QUINN-PC"]);
        let log = "\
[10:00:01] INFO  portable root C:\\Users\\Zelda Quinn\\Desktop\\Rusty MCLA\n\
[10:00:01] INFO  open C:\\Users\\Zelda%20Quinn\\x and https://x/?p=Zelda+Quinn\n\
[10:00:02] ERROR cannot write \\\\QUINN-PC\\c$\\a (user zelda quinn)\n";
        let (out, n) = r.text_counted(log);
        assert!(!out.to_lowercase().contains("zelda") && !out.to_lowercase().contains("quinn"), "{out}");
        assert_eq!(n, 5, "{out}");
        assert!(out.contains("<user>") && out.contains("<machine>"));
    }

    #[test]
    fn short_and_generic_names_do_not_wreck_the_text() {
        let a = Redactor::new(&["a"], &[""]);
        assert_eq!(a.text("data and aaa"), "data and aaa");
        assert_eq!(a.text(r"C:\Users\a\x"), r"C:\Users\<user>\x");
        let bob = Redactor::new(&["Bob"], &[""] as &[&str]);
        assert_eq!(bob.text("Bobcat and Bob and bob."), "Bobcat and <user> and <user>.");
        let user = Redactor::new(&["user"], &[""] as &[&str]);
        assert_eq!(user.text("users of user"), "users of <user>");
        assert_eq!(user.text(r"C:\Users\user_work\y"), r"C:\Users\<user>_work\y");
    }

    #[test]
    fn nothing_to_hide_changes_nothing() {
        let r = Redactor::new::<&str>(&[], &[]);
        assert!(r.is_empty());
        assert_eq!(r.text("C:\\Users\\X"), "C:\\Users\\X");
    }

    #[test]
    fn entry_names_are_redacted_and_plain() {
        let r = r();
        assert_eq!(r.entry_name(r"logs\game-Quincy.log"), "logs/game-_user_.log");
        assert_eq!(r.entry_name("../../Quincy/x"), "_user_/x");
        assert_eq!(r.entry_name("/abs//x"), "abs/x");
    }

    #[test]
    fn non_ascii_names() {
        let r = Redactor::new(&["Bärbel"], &[""] as &[&str]);
        assert_eq!(r.text(r"C:\Users\BÄRBEL\x"), r"C:\Users\<user>\x");
    }
}
