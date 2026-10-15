//! One error type for the whole update system. `kind` is stable (the page maps `kind.code()` to `dyn.upd.err.<code>` in the
//! locale); `detail` is the technical text for the log.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// No network, DNS failure, timeout: not an error the user can fix by changing anything here.
    Offline,
    /// GitHub's unauthenticated rate limit (or a 429).
    RateLimited,
    /// An HTTP status we did not expect (404, 5xx ...).
    Http,
    /// The answer was not what the API documents (malformed JSON, missing fields).
    BadResponse,
    /// A body, a manifest field or an archive is bigger than the limit for it.
    TooLarge,
    /// manifest.json is invalid (schema, hashes, URLs, sizes).
    Manifest,
    /// A downloaded file's SHA-256 does not match the manifest.
    Hash,
    /// A downloaded file's size does not match the manifest (truncated or padded).
    Size,
    /// The launcher binary's signature does not verify (or no key is configured).
    Signature,
    /// The game package is not a valid zip, or it tries to write outside its folder.
    Zip,
    /// A URL is not https or not on the allow-list.
    Policy,
    /// File system problem (disk full, access denied, file in use).
    Io,
    Cancelled,
    /// Nothing to do / the release does not contain what is needed.
    Missing,
    /// A local package that no known manifest describes.
    UnknownPackage,
    Busy,
}

impl Kind {
    pub fn code(self) -> &'static str {
        match self {
            Kind::Offline => "offline",
            Kind::RateLimited => "rate_limited",
            Kind::Http => "http",
            Kind::BadResponse => "bad_response",
            Kind::TooLarge => "too_large",
            Kind::Manifest => "manifest",
            Kind::Hash => "hash",
            Kind::Size => "size",
            Kind::Signature => "signature",
            Kind::Zip => "zip",
            Kind::Policy => "policy",
            Kind::Io => "io",
            Kind::Cancelled => "cancelled",
            Kind::Missing => "missing",
            Kind::UnknownPackage => "unknown_package",
            Kind::Busy => "busy",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UpdateError {
    pub kind: Kind,
    pub detail: String,
    /// For `RateLimited`: unix seconds when the limit resets, if the server said.
    pub reset_at: Option<i64>,
}

pub type Result<T> = std::result::Result<T, UpdateError>;

impl UpdateError {
    pub fn new(kind: Kind, detail: impl Into<String>) -> Self {
        UpdateError { kind, detail: detail.into(), reset_at: None }
    }
    pub fn io(what: impl fmt::Display, e: &std::io::Error) -> Self {
        UpdateError::new(Kind::Io, format!("{what}: {e}"))
    }
    pub fn is_cancelled(&self) -> bool {
        self.kind == Kind::Cancelled
    }
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind.code(), self.detail)
    }
}

impl std::error::Error for UpdateError {}
