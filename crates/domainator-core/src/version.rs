//! Which version this is, and which Python `domainator` it is at parity with.
//!
//! The port has its own SemVer line (`workspace.package.version`, reaching code as
//! `CARGO_PKG_VERSION`); the upstream release it reproduces is a separate fact,
//! kept here and reported next to it everywhere a version is shown. See
//! `docs/development.md`, "Versioning and releases".

/// This build's own version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Literals behind macros so `LONG_VERSION` can be assembled with `concat!` and
// handed to clap, which wants a `&'static str`.
macro_rules! upstream_version {
    () => {
        "1.0.0rc1"
    };
}
macro_rules! upstream_commit {
    () => {
        "920d690"
    };
}

/// Upstream's `__version__` at the pin — what the Python suite would call itself
/// if it had produced the same output. Also the value of a `.dsnv` bundle's
/// `domainator_version`, which is the key's meaning on the Python side.
pub const UPSTREAM_VERSION: &str = upstream_version!();

/// The upstream commit the port is caught up to (`UPSTREAM.md`'s pin). Upstream
/// often moves past a release before the next one, so the version alone does not
/// say how far the port has followed it.
pub const UPSTREAM_COMMIT: &str = upstream_commit!();

/// One line for `--version`, the About box and bug reports.
pub const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (parity with Python domainator ",
    upstream_version!(),
    " @ ",
    upstream_commit!(),
    ")"
);

#[cfg(test)]
mod tests {
    use super::*;

    /// Advancing the pin in `UPSTREAM.md` without updating [`UPSTREAM_COMMIT`] would
    /// make every build misreport what it is at parity with.
    #[test]
    fn upstream_commit_matches_the_pin_in_upstream_md() {
        let md = include_str!("../../../UPSTREAM.md");
        let pin = md
            .lines()
            .find_map(|l| l.split_once("**Caught up to:** `").map(|(_, rest)| rest))
            .and_then(|rest| rest.split('`').next())
            .expect("UPSTREAM.md has no `**Caught up to:**` line");
        assert_eq!(
            pin, UPSTREAM_COMMIT,
            "UPSTREAM.md is pinned to {pin}; update UPSTREAM_COMMIT (and UPSTREAM_VERSION, \
             if upstream's __version__ changed) in crates/domainator-core/src/version.rs"
        );
    }

    /// The viewer shell is rendered from upstream at the pin and bakes in its
    /// `__version__`, so it is an independent witness for [`UPSTREAM_VERSION`].
    #[test]
    fn upstream_version_matches_the_generated_viewer_shell() {
        let shell = include_str!("../assets/ssn_viewer_shell.html");
        let needle = format!("\"domainatorVersion\": \"{UPSTREAM_VERSION}\"");
        assert!(
            shell.contains(&needle),
            "the regenerated ssn_viewer_shell.html does not say `{needle}`: upstream's \
             __version__ changed, so update UPSTREAM_VERSION in version.rs"
        );
    }

    #[test]
    fn long_version_names_both_sides() {
        let v = LONG_VERSION;
        assert!(v.starts_with(VERSION));
        assert!(v.contains(UPSTREAM_VERSION) && v.contains(UPSTREAM_COMMIT));
    }
}
