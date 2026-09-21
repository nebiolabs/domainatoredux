//! Comparing this build against the latest published release.
//!
//! Pure logic only — parsing, comparison, and the back-off decision. The HTTP
//! fetch is a thin shim in the GUI (`cmd_update.rs`), so everything that can be
//! wrong about a version comparison is testable without a network.
//!
//! No `semver` dependency: it is in `Cargo.lock` only transitively via `tauri`,
//! and adding it here would put a GUI update-check dependency into the library
//! every CLI user links. What we need is a numeric triple with prereleases
//! sorting below their release, which is the code below.

use serde::Serialize;

/// A published release, as the app cares about it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag: String,
    pub url: String,
    pub published: Option<String>,
}

/// The outcome of a check.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheck {
    pub current: String,
    pub latest: Option<ReleaseInfo>,
    /// True only when `latest` parses *and* is strictly newer.
    pub newer: bool,
    /// Set when the check could not be completed (offline, rate-limited, or no
    /// public releases yet). Never an error the caller should surface loudly.
    pub note: Option<String>,
}

/// Parse `1.2.3`, `v1.2.3` or `1.2.3-rc.1` into `(major, minor, patch, prerelease)`.
///
/// Returns `None` for anything else, and callers treat `None` as "not newer" —
/// the app must never nag because a tag was unparseable.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64, Option<String>)> {
    let s = s.trim();
    let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V')).unwrap_or(s);
    // Build metadata (`+…`) is not ordering-significant; drop it.
    let s = s.split('+').next()?;
    let (core, pre) = match s.split_once('-') {
        Some((c, p)) if !p.is_empty() => (c, Some(p.to_string())),
        Some(_) => return None,
        None => (s, None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, pre))
}

/// Whether `latest` is strictly newer than `current`.
///
/// A prerelease sorts *below* the same numeric release (`1.0.0-rc.1 < 1.0.0`),
/// matching semver. Unparseable input on either side is "not newer".
pub fn is_newer(latest: &str, current: &str) -> bool {
    let (Some(l), Some(c)) = (parse_version(latest), parse_version(current)) else {
        return false;
    };
    match (l.0, l.1, l.2).cmp(&(c.0, c.1, c.2)) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        // Same numbers: a release beats a prerelease, and two prereleases compare
        // lexically, which is right for the `rc.1`/`rc.2` shapes we would use.
        std::cmp::Ordering::Equal => match (&l.3, &c.3) {
            (None, Some(_)) => true,
            (Some(_), None) | (None, None) => false,
            (Some(a), Some(b)) => a > b,
        },
    }
}

/// Pick the newest non-draft, non-prerelease entry from a GitHub releases payload.
///
/// Accepts either the `/releases/latest` object or the `/releases` array, since
/// the former 404s on a repository with no published releases.
pub fn pick_latest_release(body: &serde_json::Value) -> Option<ReleaseInfo> {
    let one = |v: &serde_json::Value| -> Option<ReleaseInfo> {
        if v.get("draft").and_then(|d| d.as_bool()).unwrap_or(false) {
            return None;
        }
        if v.get("prerelease").and_then(|d| d.as_bool()).unwrap_or(false) {
            return None;
        }
        Some(ReleaseInfo {
            tag: v.get("tag_name")?.as_str()?.to_string(),
            url: v.get("html_url").and_then(|u| u.as_str()).unwrap_or_default().to_string(),
            published: v.get("published_at").and_then(|u| u.as_str()).map(|s| s.to_string()),
        })
    };

    match body {
        serde_json::Value::Array(items) => {
            // The API returns newest first; take the first that qualifies.
            items.iter().find_map(one)
        }
        v => one(v),
    }
}

/// Whether enough time has passed to check again.
///
/// `None` (never checked) is always due.
pub fn should_check_now(last_ms: Option<u64>, now_ms: u64, interval_ms: u64) -> bool {
    match last_ms {
        None => true,
        // A stamp in the future (a clock that moved backwards) should not lock
        // the check out forever.
        Some(last) if last > now_ms => true,
        Some(last) => now_ms - last >= interval_ms,
    }
}

/// 24 hours, the default back-off for the launch-time check.
pub const DEFAULT_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_shapes_a_tag_actually_takes() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3, None)));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3, None)));
        assert_eq!(parse_version(" 0.1.0 "), Some((0, 1, 0, None)));
        assert_eq!(parse_version("1.0.0-rc.1"), Some((1, 0, 0, Some("rc.1".into()))));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0, None)));
        assert_eq!(parse_version("1.2.3+build7"), Some((1, 2, 3, None)));
    }

    #[test]
    fn refuses_what_it_cannot_understand() {
        for bad in ["", "latest", "v", "1.2.3.4", "1.x.3", "nightly-2026", "1.0.0-"] {
            assert_eq!(parse_version(bad), None, "should not have parsed {bad:?}");
        }
    }

    #[test]
    fn orders_releases() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("1.1.0", "1.0.9"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.1"));
        assert!(!is_newer("v0.1.0", "0.1.0"), "a leading v is not a version bump");
    }

    #[test]
    fn a_prerelease_is_older_than_its_release() {
        assert!(!is_newer("1.0.0-rc.1", "1.0.0"));
        assert!(is_newer("1.0.0", "1.0.0-rc.1"));
        assert!(is_newer("1.0.0-rc.2", "1.0.0-rc.1"));
    }

    /// The important one: a garbage tag must never produce a nag.
    #[test]
    fn an_unparseable_tag_is_never_newer() {
        assert!(!is_newer("nightly", "0.1.0"));
        assert!(!is_newer("", "0.1.0"));
        assert!(!is_newer("1.0.0", "not-a-version"));
    }

    #[test]
    fn skips_drafts_and_prereleases() {
        let body = serde_json::json!([
            { "tag_name": "v2.0.0", "draft": true,  "html_url": "u1" },
            { "tag_name": "v1.9.0", "prerelease": true, "html_url": "u2" },
            { "tag_name": "v1.8.0", "html_url": "u3", "published_at": "2026-01-02T00:00:00Z" },
        ]);
        let got = pick_latest_release(&body).unwrap();
        assert_eq!(got.tag, "v1.8.0");
        assert_eq!(got.url, "u3");
        assert_eq!(got.published.as_deref(), Some("2026-01-02T00:00:00Z"));
    }

    #[test]
    fn reads_a_single_release_object_too() {
        let body = serde_json::json!({ "tag_name": "v1.0.0", "html_url": "u" });
        assert_eq!(pick_latest_release(&body).unwrap().tag, "v1.0.0");
    }

    #[test]
    fn a_payload_with_nothing_publishable_yields_none() {
        assert_eq!(pick_latest_release(&serde_json::json!([])), None);
        assert_eq!(
            pick_latest_release(&serde_json::json!([{ "tag_name": "v1", "draft": true }])),
            None
        );
        // A 404 body has no tag_name at all.
        assert_eq!(
            pick_latest_release(&serde_json::json!({ "message": "Not Found" })),
            None
        );
    }

    #[test]
    fn backs_off_between_checks() {
        const HOUR: u64 = 60 * 60 * 1000;
        let now = 100 * HOUR;
        assert!(should_check_now(None, now, DEFAULT_INTERVAL_MS), "never checked");
        assert!(!should_check_now(Some(now - HOUR), now, DEFAULT_INTERVAL_MS), "an hour ago");
        assert!(should_check_now(Some(now - 25 * HOUR), now, DEFAULT_INTERVAL_MS), "yesterday");
        assert!(should_check_now(Some(now + HOUR), now, DEFAULT_INTERVAL_MS), "clock went back");
    }
}
