//! Support features of the launcher, without any UI toolkit:
//! * [`redact`] - the user's Windows name and the machine name never leave the computer in an export;
//! * [`sysinfo`] - the system information text (OS, CPU, memory, graphics, Vulkan / DirectX files, displays);
//! * [`export`] - the Export Logs zip, built only from an allow-list of file roles;
//! * [`saves`] - the save manager (status, backups, restore, import) over `mcla_updater::backup`;
//! * [`launch`] - starting the game, noticing an early exit, remembering a crash for the next launcher start.

pub mod export;
pub mod launch;
pub mod redact;
pub mod saves;
pub mod sysinfo;

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// SHA-256 of a file as lower-case hex, read in 1 MiB steps.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(mcla_updater::hex(&h.finalize()))
}

/// A `Command` that never opens a console window (the launcher has none; a child console program would flash one).
pub fn quiet_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[allow(unused_mut)]
    let mut c = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c
}

/// An error as the page shows it: `code` selects the translated text (`dyn.sup.err.<code>`), `detail` is the technical line for the log.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Error {
    pub code: &'static str,
    pub detail: String,
}

impl Error {
    pub fn new(code: &'static str, detail: impl Into<String>) -> Error {
        Error { code, detail: detail.into() }
    }
    pub fn io(what: impl std::fmt::Display, e: &std::io::Error) -> Error {
        Error { code: "io", detail: format!("{what}: {e}") }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
