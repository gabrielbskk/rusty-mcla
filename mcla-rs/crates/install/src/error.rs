//! Friendly error values for the UI. Every failure of the pipeline is an [`InstallError`]: a stable `kind` (the launcher maps it to a
//! locale key, `install.err.<code>`), an English `message` that is good enough on its own (CLI, logs) and a technical `detail`.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The user cancelled.
    Cancelled,
    /// The file or folder cannot be opened or read (missing, locked, network share gone).
    Unreadable,
    /// The path is a folder, a GoD package or a loose `.xex`, not a disc image.
    NotAnImage,
    /// `.cso`, `.zip`, `.7z`, `.rar`, `.gz` ...: has to be unpacked to a plain `.iso` first.
    CompressedImage,
    /// Smaller than the disc image should be (partial download or copy).
    Truncated,
    /// A disc image, but not an Xbox 360 game disc layout we can read.
    NotXbox360,
    /// An Xbox 360 disc of another game.
    WrongGame,
    /// This game, but not the supported edition / region (the archive key cannot be found).
    UnsupportedEdition,
    /// A required file of the disc is missing or damaged.
    MissingFile,
    /// Not enough free space.
    DiskSpace,
    /// The install folder cannot be written to.
    NotWritable,
    /// The install folder path is too long for the files we create.
    PathTooLong,
    /// The install folder is not usable (inside the ISO's own folder tree of the data, a file, ...).
    BadFolder,
    /// No ffmpeg and it cannot be fetched (offline) or the given path is not ffmpeg.
    FfmpegMissing,
    /// The ffmpeg download failed (network, HTTP status).
    FfmpegDownload,
    /// The downloaded ffmpeg did not match its pinned SHA-256: it was deleted and never run.
    FfmpegHash,
    /// ffmpeg ran but failed on a file.
    Convert,
    /// The install manifest is broken or too new.
    Manifest,
    /// Something is wrong in the built data folder (verify).
    DataDamaged,
    /// Other I/O error (disk full, access denied while writing, ...).
    Io,
}

impl ErrorKind {
    /// Stable snake_case code, also the suffix of the launcher's locale key.
    pub fn code(self) -> &'static str {
        match self {
            ErrorKind::Cancelled => "cancelled",
            ErrorKind::Unreadable => "unreadable",
            ErrorKind::NotAnImage => "not_an_image",
            ErrorKind::CompressedImage => "compressed_image",
            ErrorKind::Truncated => "truncated",
            ErrorKind::NotXbox360 => "not_xbox360",
            ErrorKind::WrongGame => "wrong_game",
            ErrorKind::UnsupportedEdition => "unsupported_edition",
            ErrorKind::MissingFile => "missing_file",
            ErrorKind::DiskSpace => "disk_space",
            ErrorKind::NotWritable => "not_writable",
            ErrorKind::PathTooLong => "path_too_long",
            ErrorKind::BadFolder => "bad_folder",
            ErrorKind::FfmpegMissing => "ffmpeg_missing",
            ErrorKind::FfmpegDownload => "ffmpeg_download",
            ErrorKind::FfmpegHash => "ffmpeg_hash",
            ErrorKind::Convert => "convert",
            ErrorKind::Manifest => "manifest",
            ErrorKind::DataDamaged => "data_damaged",
            ErrorKind::Io => "io",
        }
    }

    /// English text for a user (the launcher shows its own translation of the same code).
    pub fn friendly(self) -> &'static str {
        match self {
            ErrorKind::Cancelled => "Stopped. You can resume later; finished work is kept.",
            ErrorKind::Unreadable => "The file cannot be read. Check that it exists, is not in use, and (for a network drive) is reachable.",
            ErrorKind::NotAnImage => "This is not a disc image. Choose the .iso file of the Xbox 360 disc (not a folder, a Games on Demand package or a .xex).",
            ErrorKind::CompressedImage => "This image is compressed or archived. Unpack it to a plain .iso first (for example with 7-Zip), then choose the .iso.",
            ErrorKind::Truncated => "The image is smaller than the disc it should hold. The copy or download is probably incomplete.",
            ErrorKind::NotXbox360 => "This file is not an Xbox 360 game disc image (no readable Xbox disc layout).",
            ErrorKind::WrongGame => "This is an Xbox 360 disc, but of a different game. Choose the Midnight Club: Los Angeles - Complete Edition disc.",
            ErrorKind::UnsupportedEdition => "This looks like the right game, but not the supported edition or region (Complete Edition, USA / Europe). Its data cannot be read.",
            ErrorKind::MissingFile => "A required file is missing from the disc image. The image is probably incomplete or a different release.",
            ErrorKind::DiskSpace => "Not enough free space in the install folder's drive.",
            ErrorKind::NotWritable => "The install folder cannot be written to. Choose another folder or check its permissions.",
            ErrorKind::PathTooLong => "The install folder's path is too long. Choose a shorter folder (for example C:\\RustyMCLA).",
            ErrorKind::BadFolder => "This folder cannot be used for the install. Choose another one.",
            ErrorKind::FfmpegMissing => "ffmpeg was not found. Choose an ffmpeg.exe, or allow the launcher to download it.",
            ErrorKind::FfmpegDownload => "ffmpeg could not be downloaded. Check the internet connection, or choose an ffmpeg.exe you already have.",
            ErrorKind::FfmpegHash => "The downloaded ffmpeg did not match its expected checksum and was discarded. It was not run.",
            ErrorKind::Convert => "ffmpeg could not convert one of the files. See the build log.",
            ErrorKind::Manifest => "The install manifest is damaged or from a newer launcher.",
            ErrorKind::DataDamaged => "The built data folder is incomplete or damaged. Run Repair.",
            ErrorKind::Io => "A file could not be written (disk full, access denied or the file is in use).",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstallError {
    pub kind: ErrorKind,
    /// Technical detail for the log (paths, OS error text, counts).
    pub detail: String,
}

impl InstallError {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        InstallError { kind, detail: detail.into() }
    }
    pub fn cancelled() -> Self {
        InstallError::new(ErrorKind::Cancelled, "cancelled by the user")
    }
    pub fn is_cancelled(&self) -> bool {
        self.kind == ErrorKind::Cancelled
    }
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
    pub fn message(&self) -> &'static str {
        self.kind.friendly()
    }
    pub fn io(what: impl fmt::Display, e: &std::io::Error) -> Self {
        // disk full / access denied are worth their own kinds
        let kind = match e.kind() {
            std::io::ErrorKind::PermissionDenied => ErrorKind::NotWritable,
            std::io::ErrorKind::NotFound => ErrorKind::Unreadable,
            _ => match e.raw_os_error() {
                Some(112) | Some(28) => ErrorKind::DiskSpace, // ERROR_DISK_FULL, ENOSPC
                _ => ErrorKind::Io,
            },
        };
        InstallError::new(kind, format!("{what}: {e}"))
    }
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}: {})", self.kind.friendly(), self.kind.code(), self.detail)
    }
}

impl std::error::Error for InstallError {}

pub type Result<T> = std::result::Result<T, InstallError>;

pub(crate) trait IoCtx<T> {
    fn ctx(self, what: impl fmt::Display) -> Result<T>;
}

impl<T> IoCtx<T> for std::io::Result<T> {
    fn ctx(self, what: impl fmt::Display) -> Result<T> {
        self.map_err(|e| InstallError::io(what, &e))
    }
}
