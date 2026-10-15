//! `mcla-install`: the install pipeline of the Rusty MCLA launcher, as a library with no UI code.
//!
//! Disc image in, ready data folder out: preflight, ISO validation (fast checks + background SHA-256), extraction of
//! `xarchive_cache.rpf`, ffmpeg lookup / pinned download, audio and video conversion, a stamped `manifest.local.json`,
//! verify and repair. Long jobs report [`progress::Event`]s through a [`progress::Ctx`] and stop on its [`progress::Cancel`] flag.
//! Errors are [`error::InstallError`] values with a stable kind for the UI to translate.
//!
//! The launcher (MSVC) and `mcla-tool` (GNU) both use it; keep it free of toolchain-specific code.

pub mod audio;
pub mod error;
pub mod extract;
pub mod ffmpeg;
pub mod install;
pub mod layout;
pub mod manifest;
pub mod preflight;
pub mod progress;
pub mod shaders;
pub mod validate;
pub mod verify;
pub mod video;

pub use error::{ErrorKind, InstallError, Result};
pub use install::{install, repair, InstallOptions, InstallOutcome};
pub use layout::{Layout, LocalManifest};
pub use manifest::Manifest;
pub use progress::{Cancel, Check, CheckStatus, Ctx, Event, Stage, Unit};
pub use validate::{hash_iso, Disc, DumpState, HashResult, IsoIdentity};
