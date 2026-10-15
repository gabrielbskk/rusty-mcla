//! Decoder of a save of the original game (Xbox 360 `mc4.sav`, or a PS3 `RAGE.SAV` after decryption) into a save of the port.
//!
//! A port of the Python tools `mcla-rs/tools/import_mc4_save.py` and `mcla-rs/tools/mc4save/*.py` (`career`, `options`, `small`, `vehicle`), which
//! stay as the reference: the same input gives the same JSON value (`tests/` compares them on the real saves). No Python, no key material, no
//! Windows-only code; the game's GNU workspace and the launcher's MSVC workspace both build it.
//!
//! * [`convert`] takes the save's bytes, the port's current save (or `{}`) and the game's race table and returns the new save.
//! * [`stfs::extract`] takes `mc4.sav` out of an Xbox 360 console package (CON / LIVE / PIRS); no key is involved.
//! * [`looks_like_original`] / [`container`] recognise the container without decoding it.
//!
//! What is *not* ported: the reference dump `<out>.mc4full.json` the Python importer writes beside its output (the port never reads it).

#![recursion_limit = "512"]

mod career;
mod container;
mod convert;
mod options;
pub mod ps3;
mod small;
pub mod stfs;
mod tables;
mod util;
mod vehicle;

pub use container::{looks_like_original, Container, Entry};
pub use convert::{convert, race_table, Converted, Summary};

/// Checks that `bytes` is a save of the original game this reader understands (the container: version, entries, sizes) without decoding it.
pub fn validate(bytes: &[u8]) -> Result<()> {
    Container::parse(bytes).map(|_| ())
}

/// Helpers for the tests of this crate and of the launcher (not part of the API).
#[doc(hidden)]
pub mod testing {
    use crate::container::REQUIRED;

    /// A valid save of a game that was never played: every entry zero-filled, the Career entry at layout version 11. `version` is 14 or 15.
    pub fn blank_save(version: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&version.to_be_bytes());
        v.extend_from_slice(&(REQUIRED.len() as u32).to_be_bytes());
        for (name, size) in REQUIRED {
            v.extend_from_slice(&(name.len() as u32).to_be_bytes());
            v.extend_from_slice(name.as_bytes());
            v.push(1);
            v.extend_from_slice(&(*size as u32).to_be_bytes());
            let start = v.len();
            v.resize(start + size, 0);
            if *name == "Career" {
                v[start + 4] = 11;
            }
        }
        v
    }
}

use std::fmt;

/// Why a file could not be converted. `code` is a short stable word (the launcher maps it to a message), `detail` is for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub code: &'static str,
    pub detail: String,
}

impl Error {
    pub(crate) fn new(code: &'static str, detail: impl Into<String>) -> Error {
        Error { code, detail: detail.into() }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
