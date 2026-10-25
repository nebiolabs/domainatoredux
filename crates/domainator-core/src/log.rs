//! Progress and warning messages on stderr, at upstream's levels (its
//! `cli.common.configure_logging`): progress (`info`) is shown by default, `debug`
//! only with `-v`, and `-q` leaves errors alone, hiding warnings too. The CLI sets
//! the level once its arguments are parsed. Errors are not logged here: they are
//! returned, and the CLI prints them.

use std::fmt::Display;
use std::sync::atomic::{AtomicU8, Ordering};

/// How much is shown, least first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    /// `-q`: errors only.
    Error = 0,
    Warning = 1,
    /// The default.
    Info = 2,
    /// `-v`.
    Debug = 3,
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);

/// Show messages up to `level` from now on.
pub fn set_level(level: Level) {
    LEVEL.store(level as u8, Ordering::Relaxed);
}

/// The level for a tool's `-v`/`-q` flags.
pub fn level_for(verbose: bool, quiet: bool) -> Level {
    if verbose {
        Level::Debug
    } else if quiet {
        Level::Error
    } else {
        Level::Info
    }
}

/// Whether messages at `level` are shown.
pub fn enabled(level: Level) -> bool {
    LEVEL.load(Ordering::Relaxed) >= level as u8
}

/// A problem the run works around: shown unless `-q`.
pub fn warn(message: impl Display) {
    if enabled(Level::Warning) {
        eprintln!("warning: {message}");
    }
}

/// Progress: shown unless `-q`.
pub fn info(message: impl Display) {
    if enabled(Level::Info) {
        eprintln!("{message}");
    }
}

/// Detail: shown with `-v`.
pub fn debug(message: impl Display) {
    if enabled(Level::Debug) {
        eprintln!("{message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_choose_upstreams_levels() {
        assert_eq!(level_for(false, false), Level::Info);
        assert_eq!(level_for(true, false), Level::Debug);
        assert_eq!(level_for(false, true), Level::Error);
        assert!(Level::Error < Level::Warning && Level::Info < Level::Debug);
    }
}
