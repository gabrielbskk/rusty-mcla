//! A harmless stand-in for the game exe, used by the launcher's Play checks: it records how it was started and then behaves as told.
//! `STANDIN_OUT=<file>` receives the arguments, the working directory and the interesting environment;
//! `STANDIN_EXIT=<code>` exits at once with that code; `STANDIN_SLEEP=<seconds>` (default 8) keeps it running.
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Ok(out) = std::env::var("STANDIN_OUT") {
        let text = format!(
            "args: {}\ncwd: {}\nexe: {}\nRUSTY_MCLA_USER_DIR: {}\n",
            args.join(" | "),
            std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default(),
            std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default(),
            std::env::var("RUSTY_MCLA_USER_DIR").unwrap_or_default()
        );
        let _ = std::fs::write(out, text);
    }
    if let Ok(code) = std::env::var("STANDIN_EXIT") {
        if let Ok(dir) = std::env::var("STANDIN_CRASH_DIR") {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(std::path::Path::new(&dir).join("crash-standin.txt"), "stand-in crash report\n");
        }
        std::process::exit(code.parse().unwrap_or(1));
    }
    let secs: u64 = std::env::var("STANDIN_SLEEP").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    std::thread::sleep(std::time::Duration::from_secs(secs));
}
