//! Identity of the application: where it lives, what it is licensed as, and the
//! diagnostics blob a bug report should carry.
//!
//! This module exists so there is exactly **one** place that knows the project's
//! URLs. The GUI had none at all — no link out, no version on screen — and the
//! first thing that grows when you add "About" and "Check for updates" is three
//! copies of the same repository URL drifting apart.
//!
//! [`is_allowed_url`] is the other half. The GUI opens URLs through the OS, which
//! is a capability worth keeping narrow: the webview never gets to name the
//! destination, it can only ask for one of ours, and the Tauri command checks it
//! here first. That matters because the SSN viewer runs untrusted-ish generated
//! HTML in a sandboxed iframe inside the same app.

/// The project's home. **Not publicly reachable yet** — the repository is private,
/// so links to it and the release check below both no-op until it is published.
/// Nothing else needs to change when that happens.
pub const PROJECT_URL: &str = "https://github.com/nebiolabs/domainatoredux";

/// Where to file a bug (the diagnostics blob is meant to be pasted into one).
pub const ISSUES_URL: &str = "https://github.com/nebiolabs/domainatoredux/issues";

/// The release the "check for updates" flow compares against, as JSON.
pub const RELEASES_API: &str =
    "https://api.github.com/repos/nebiolabs/domainatoredux/releases/latest";

/// The human-readable page that same flow offers to open.
pub const RELEASES_URL: &str = "https://github.com/nebiolabs/domainatoredux/releases/latest";

/// Matches `license` in the workspace `Cargo.toml`.
pub const LICENSE: &str = "MIT";

/// Hosts the app is willing to hand to the OS browser. Deliberately tiny: every
/// entry is a URL *we* ship, never one the frontend composes.
const ALLOWED_HOSTS: &[&str] = &["github.com", "api.github.com"];

/// Whether `url` is one this app will open.
///
/// Requires `https://`, a host in [`ALLOWED_HOSTS`] matched on the **whole** host
/// (so `github.com.evil.example` is rejected, not merely suffix-checked), no
/// embedded credentials, and no control characters — a newline in a URL handed to
/// a platform opener is an argument-injection primitive on more than one OS.
pub fn is_allowed_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    if url.chars().any(|c| c.is_control() || c == '"' || c == '\'') {
        return false;
    }
    // Everything up to the first `/`, `?` or `#` is the authority.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    // Strip a port before comparing, so `github.com:443` is still github.com.
    let host = authority.split(':').next().unwrap_or("");
    ALLOWED_HOSTS.iter().any(|h| host.eq_ignore_ascii_case(h))
}

/// One managed external binary, as the About box lists it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolLine {
    pub name: String,
    pub version: String,
    pub installed: bool,
}

/// Everything worth knowing about a running install.
///
/// Serialized to the GUI *and* rendered to text by [`render_diagnostics`], so the
/// dialog and the copy/paste blob can never describe different builds.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Diagnostics {
    pub product: String,
    pub version: String,
    pub tauri_version: String,
    pub webview_version: String,
    pub os: String,
    pub arch: String,
    pub profile: String,
    pub app_data_dir: String,
    pub prefs_path: String,
    pub managed_tools_dir: String,
    pub external_tools: Vec<ToolLine>,
}

/// The blob a user pastes into a bug report.
///
/// Plain `key: value` lines rather than JSON: it is read by a human in an issue
/// thread far more often than it is parsed.
pub fn render_diagnostics(d: &Diagnostics) -> String {
    let mut s = String::new();
    s.push_str(&format!("{} {}\n", d.product, d.version));
    s.push_str(&format!("license:      {LICENSE}\n"));
    s.push_str(&format!("tauri:        {}\n", d.tauri_version));
    s.push_str(&format!("webview:      {}\n", d.webview_version));
    s.push_str(&format!("platform:     {} {} ({})\n", d.os, d.arch, d.profile));
    s.push_str(&format!("app data:     {}\n", d.app_data_dir));
    s.push_str(&format!("preferences:  {}\n", d.prefs_path));
    s.push_str(&format!("managed bins: {}\n", d.managed_tools_dir));
    s.push_str("external tools:\n");
    if d.external_tools.is_empty() {
        s.push_str("  (none)\n");
    }
    for t in &d.external_tools {
        let state = if t.installed { "installed" } else { "not installed" };
        s.push_str(&format!("  {} {} — {}\n", t.name, t.version, state));
    }
    s.push_str(&format!("project:      {PROJECT_URL}\n"));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Diagnostics {
        Diagnostics {
            product: "Domainator".into(),
            version: "0.1.0".into(),
            tauri_version: "2.11.4".into(),
            webview_version: "120.0".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            profile: "release".into(),
            app_data_dir: "/home/u/.local/share/org.domainator.gui".into(),
            prefs_path: "/home/u/.local/share/org.domainator.gui/prefs.json".into(),
            managed_tools_dir: "/home/u/.local/share/org.domainator.gui/tools".into(),
            external_tools: vec![ToolLine {
                name: "diamond".into(),
                version: "2.2.5".into(),
                installed: true,
            }],
        }
    }

    #[test]
    fn allowed_urls_are_https_on_a_known_host() {
        assert!(is_allowed_url(PROJECT_URL));
        assert!(is_allowed_url(RELEASES_URL));
        assert!(is_allowed_url("https://github.com/nebiolabs/domainatoredux/releases/tag/v1.2"));
        assert!(is_allowed_url("https://GitHub.com/x"), "host match is case-insensitive");
    }

    /// Each of these was a real way to turn "open our project page" into "open
    /// whatever the caller wants".
    #[test]
    fn everything_else_is_refused() {
        for bad in [
            "http://github.com/x",                 // downgraded scheme
            "file:///etc/passwd",                  // local file
            "javascript:alert(1)",                 // script URL
            "https://evil.example/x",              // unknown host
            "https://github.com.evil.example/x",   // host is a *prefix*, not ours
            "https://evilgithub.com/x",            // ours is a suffix of theirs
            "https://user@github.com/x",           // embedded credentials
            "https://github.com/x\nmalicious",     // control character
            "https:///x",                          // empty authority
            "github.com/x",                        // no scheme
            "",
        ] {
            assert!(!is_allowed_url(bad), "should have refused {bad:?}");
        }
    }

    /// If someone adds a docs or forum URL and forgets `ALLOWED_HOSTS`, the button
    /// for it silently does nothing. Cheaper to fail here.
    #[test]
    fn every_url_we_ship_passes_our_own_validator() {
        for url in [PROJECT_URL, ISSUES_URL, RELEASES_API, RELEASES_URL] {
            assert!(is_allowed_url(url), "{url} is shipped but would be refused");
        }
    }

    #[test]
    fn the_diagnostics_blob_keeps_every_field() {
        let d = sample();
        let out = render_diagnostics(&d);
        for needle in [
            "Domainator", "0.1.0", "MIT", "2.11.4", "120.0", "linux", "x86_64", "release",
            &d.app_data_dir, &d.prefs_path, &d.managed_tools_dir, "diamond", "2.2.5",
            "installed", PROJECT_URL,
        ] {
            assert!(out.contains(needle), "diagnostics lost {needle:?}:\n{out}");
        }
    }

    #[test]
    fn a_missing_tool_is_reported_as_missing_not_omitted() {
        let mut d = sample();
        d.external_tools[0].installed = false;
        assert!(render_diagnostics(&d).contains("not installed"));
    }

    #[test]
    fn no_external_tools_still_renders() {
        let mut d = sample();
        d.external_tools.clear();
        assert!(render_diagnostics(&d).contains("(none)"));
    }
}
