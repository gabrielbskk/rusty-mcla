//! Version comparison (semver). Tags may carry a leading `v`.

use semver::Version;
use std::cmp::Ordering;

/// `v1.2.3` / `1.2.3` / `1.2.3-rc.1`. Anything else (`1.2`, `latest`) is not a version.
pub fn parse(s: &str) -> Option<Version> {
    let s = s.trim();
    let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V')).unwrap_or(s);
    Version::parse(s).ok()
}

/// `None` when either side is not a version.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    Some(parse(a)?.cmp(&parse(b)?))
}

/// `a` is strictly newer than `b`. An unparsable `b` (nothing installed) counts as older than any valid `a`.
pub fn is_newer(a: &str, b: &str) -> bool {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a > b,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Normal form for display and storage: no leading `v`.
pub fn normalise(s: &str) -> Option<String> {
    parse(s).map(|v| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_with_and_without_v() {
        assert_eq!(parse("v1.2.3").unwrap(), Version::new(1, 2, 3));
        assert_eq!(parse(" 1.2.3 ").unwrap(), Version::new(1, 2, 3));
        assert!(parse("1.2").is_none());
        assert!(parse("latest").is_none());
        assert!(parse("").is_none());
    }

    #[test]
    fn compares_numerically_not_as_text() {
        assert!(is_newer("0.10.0", "0.9.0"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
    }

    #[test]
    fn prerelease_is_older_than_release() {
        assert!(is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(!is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(is_newer("1.0.0-rc.2", "1.0.0-rc.1"));
    }

    #[test]
    fn nothing_installed_is_older_but_garbage_is_never_newer() {
        assert!(is_newer("0.1.0", "none"));
        assert!(!is_newer("garbage", "0.1.0"));
        assert_eq!(compare("0.1.0", "0.1.0"), Some(Ordering::Equal));
        assert_eq!(compare("x", "0.1.0"), None);
    }

    #[test]
    fn normalise_strips_the_v() {
        assert_eq!(normalise("v0.3.1").unwrap(), "0.3.1");
        assert!(normalise("nope").is_none());
    }
}
