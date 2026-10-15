//! Where releases come from, in one place: `launcher/release.json` (compiled in) with test overrides from the environment.
//!
//! * `release.json` holds the repository (`OWNER/rusty-mcla` until the real one exists), the API and web base URLs and the host
//!   allow-list. Every URL the updater follows (assets, redirects, the notes link) must be https and on that list.
//! * `RUSTY_MCLA_RELEASE_REPO=owner/name` points the launcher at another repository (a fork).
//! * `RUSTY_MCLA_TEST_MOCK=http://127.0.0.1:PORT` is the test hook: the API base becomes that URL, its host joins the allow-list and
//!   plain http is accepted **for that loopback host only**. `RUSTY_MCLA_TEST_PUBKEY=<file>` then replaces the embedded signing key.
//!   Without `RUSTY_MCLA_TEST_MOCK` neither key nor hosts can be changed from the environment.

use crate::error::{Kind, Result, UpdateError};
use serde::Deserialize;
use url::Url;

const EMBEDDED: &str = include_str!("../../release.json");
const EMBEDDED_KEY: &str = include_str!("../../update-key.pub");

#[derive(Debug, Clone, Deserialize)]
struct File {
    repo: String,
    api_base: String,
    web_base: String,
    allowed_hosts: Vec<String>,
    launcher_asset: String,
    manifest_asset: String,
}

#[derive(Debug, Clone)]
pub struct ReleaseConfig {
    /// `owner/name`
    pub repo: String,
    pub api_base: String,
    pub web_base: String,
    pub allowed_hosts: Vec<String>,
    pub launcher_asset: String,
    pub manifest_asset: String,
    pub user_agent: String,
    /// The minisign public key file text (2 lines). A file that does not decode means "no key set": self-update refuses.
    pub public_key: String,
    /// Test hook: the mock server's `host:port`; http is accepted for exactly this authority.
    pub mock_authority: Option<String>,
}

pub fn valid_repo(s: &str) -> bool {
    let mut it = s.split('/');
    let (Some(a), Some(b), None) = (it.next(), it.next(), it.next()) else { return false };
    let ok = |p: &str| !p.is_empty() && p.len() <= 100 && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    ok(a) && ok(b)
}

impl ReleaseConfig {
    /// The compiled-in configuration plus the environment overrides described above.
    pub fn from_env(app_version: &str) -> ReleaseConfig {
        let mut c = ReleaseConfig::embedded(app_version);
        if let Ok(r) = std::env::var("RUSTY_MCLA_RELEASE_REPO") {
            if valid_repo(&r) {
                c.repo = r;
            } else {
                log::warn!("RUSTY_MCLA_RELEASE_REPO ignored: not owner/name");
            }
        }
        if let Ok(base) = std::env::var("RUSTY_MCLA_TEST_MOCK") {
            let pubkey = std::env::var("RUSTY_MCLA_TEST_PUBKEY").ok().and_then(|p| std::fs::read_to_string(p).ok());
            c.enable_mock(&base, pubkey);
        }
        c
    }

    pub fn embedded(app_version: &str) -> ReleaseConfig {
        let f: File = serde_json::from_str(EMBEDDED).expect("release.json is valid");
        ReleaseConfig {
            repo: f.repo,
            api_base: f.api_base,
            web_base: f.web_base,
            allowed_hosts: f.allowed_hosts,
            launcher_asset: f.launcher_asset,
            manifest_asset: f.manifest_asset,
            user_agent: format!("RustyMCLA-launcher/{app_version} (+https://github.com)"),
            public_key: EMBEDDED_KEY.to_string(),
            mock_authority: None,
        }
    }

    /// For tests: a configuration that talks to a loopback mock server.
    pub fn for_mock(base: &str, public_key: Option<String>) -> ReleaseConfig {
        let mut c = ReleaseConfig::embedded("0.0.0-test");
        c.enable_mock(base, public_key);
        c
    }

    fn enable_mock(&mut self, base: &str, public_key: Option<String>) {
        let Ok(u) = Url::parse(base) else { return };
        let Some(host) = u.host_str() else { return };
        if !is_loopback(host) {
            log::warn!("test mock ignored: {host} is not a loopback host");
            return;
        }
        let authority = match u.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };
        self.api_base = base.trim_end_matches('/').to_string();
        self.web_base = base.trim_end_matches('/').to_string();
        self.mock_authority = Some(authority);
        if let Some(k) = public_key {
            self.public_key = k;
        }
        log::warn!("TEST MODE: release API is {base}");
    }

    /// Releases API URL for this repository: `<api_base>/repos/<owner>/<name>/<tail>`.
    pub fn api_url(&self, tail: &str) -> String {
        format!("{}/repos/{}/{}", self.api_base.trim_end_matches('/'), self.repo, tail.trim_start_matches('/'))
    }

    /// The page with the release notes on the web.
    pub fn web_release_url(&self, tag: &str) -> String {
        format!("{}/{}/releases/tag/{}", self.web_base.trim_end_matches('/'), self.repo, tag)
    }

    /// Accepts a URL only if it is https (or the mock's http), has no credentials, and its host is on the allow-list.
    /// Returns the parsed URL.
    pub fn check_url(&self, s: &str) -> Result<Url> {
        let bad = |why: &str| UpdateError::new(Kind::Policy, format!("{why}: {}", s.chars().take(200).collect::<String>()));
        if s.len() > 2048 {
            return Err(bad("url too long"));
        }
        let u = Url::parse(s).map_err(|_| bad("not a url"))?;
        if !u.username().is_empty() || u.password().is_some() {
            return Err(bad("credentials in url"));
        }
        let host = u.host_str().ok_or_else(|| bad("no host"))?.to_ascii_lowercase();
        let authority = match u.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.clone(),
        };
        if let Some(mock) = &self.mock_authority {
            if &authority == mock && matches!(u.scheme(), "http" | "https") {
                return Ok(u);
            }
        }
        if u.scheme() != "https" {
            return Err(bad("not https"));
        }
        if u.port().is_some() {
            return Err(bad("unexpected port"));
        }
        if !self.allowed_hosts.iter().any(|h| h.eq_ignore_ascii_case(&host)) {
            return Err(bad("host not allowed"));
        }
        Ok(u)
    }

    /// Like [`check_url`](Self::check_url), and for a `github.com` URL also that it points into this repository's release downloads.
    pub fn check_asset_url(&self, s: &str) -> Result<Url> {
        let u = self.check_url(s)?;
        if u.host_str().map(|h| h.eq_ignore_ascii_case("github.com")).unwrap_or(false) {
            let want = format!("/{}/releases/download/", self.repo);
            if !u.path().to_ascii_lowercase().starts_with(&want.to_ascii_lowercase()) {
                return Err(UpdateError::new(Kind::Policy, format!("asset is not in this repository's releases: {}", u.path())));
            }
        }
        Ok(u)
    }
}

pub fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ReleaseConfig {
        ReleaseConfig::embedded("0.1.0")
    }

    #[test]
    fn embedded_config_is_the_project_repo() {
        let c = cfg();
        assert_eq!(c.repo, "holdmysocks/rusty-mcla");
        assert_eq!(c.api_url("releases/latest"), "https://api.github.com/repos/holdmysocks/rusty-mcla/releases/latest");
        assert!(valid_repo(&c.repo));
    }

    #[test]
    fn urls_must_be_https_and_on_the_list() {
        let c = cfg();
        assert!(c.check_url("https://api.github.com/x").is_ok());
        assert!(c.check_url("https://objects.githubusercontent.com/a/b").is_ok());
        assert!(c.check_url("http://api.github.com/x").is_err());
        assert!(c.check_url("https://evil.example/x").is_err());
        assert!(c.check_url("https://api.github.com.evil.example/x").is_err());
        assert!(c.check_url("https://user:pw@api.github.com/x").is_err());
        assert!(c.check_url("https://api.github.com:8443/x").is_err());
        assert!(c.check_url("ftp://api.github.com/x").is_err());
        assert!(c.check_url("file:///C:/x").is_err());
        assert!(c.check_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn github_assets_must_belong_to_the_repo() {
        let c = cfg();
        assert!(c.check_asset_url("https://github.com/holdmysocks/rusty-mcla/releases/download/v1/a.zip").is_ok());
        assert!(c.check_asset_url("https://github.com/someone/else/releases/download/v1/a.zip").is_err());
        assert!(c.check_asset_url("https://github.com/holdmysocks/rusty-mcla/raw/main/a.zip").is_err());
    }

    #[test]
    fn mock_allows_http_for_loopback_only() {
        let c = ReleaseConfig::for_mock("http://127.0.0.1:4100", None);
        assert!(c.check_url("http://127.0.0.1:4100/a").is_ok());
        assert!(c.check_url("http://127.0.0.1:4101/a").is_err());
        assert!(c.check_url("http://evil.example/a").is_err());
        let not_loopback = ReleaseConfig::for_mock("http://evil.example:4100", None);
        assert!(not_loopback.mock_authority.is_none());
        assert!(not_loopback.check_url("http://evil.example:4100/a").is_err());
    }

    #[test]
    fn repo_names_are_checked() {
        assert!(valid_repo("a/b"));
        assert!(!valid_repo("a"));
        assert!(!valid_repo("a/b/c"));
        assert!(!valid_repo("a b/c"));
        assert!(!valid_repo("../x"));
    }
}
