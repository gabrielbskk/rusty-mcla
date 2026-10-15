//! `mkpkg <out> <file> [CON|LIVE|PIRS] [chained]`: wraps a file as `mc4.sav` in a synthetic STFS package (for trying the launcher's import with a
//! package; the real console files come from a console).
use mcla_savedecode::stfs::testing::{build, File, Options};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: mkpkg <out> <file> [CON|LIVE|PIRS] [chained]");
        std::process::exit(2);
    }
    let data = std::fs::read(&a[2]).expect("read input");
    let magic = match a.get(3).map(String::as_str) {
        Some("LIVE") => *b"LIVE",
        Some("PIRS") => *b"PIRS",
        _ => *b"CON ",
    };
    let chained = a.get(4).map(|s| s == "chained").unwrap_or(false);
    let o = Options { magic, male: &magic == b"CON ", ..Options::default() };
    std::fs::write(&a[1], build(&[File::new("mc4.sav", &data, !chained)], &o)).expect("write output");
}
