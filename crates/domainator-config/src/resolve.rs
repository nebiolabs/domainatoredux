//! Shared config → `*Options` resolution: the default-filling and validation that
//! turns a serde `*Config` (from the CLI or the GUI form) into the borrowed
//! `*Options` a core tool expects. Keeping this here means the CLI and GUI resolve
//! parameters identically and can't drift.
//!
//! Resolvers are **pure**: file-based inputs (subset lists, color tables) are read
//! by the caller and passed in, so this module needs no filesystem access and the
//! caller keeps control of I/O.

use domainator_core::tools::ssn_navigator::{NavMode, SsnNavigatorOptions};

use crate::SsnNavigatorConfig;

/// Default lower bound / merge-event / metric constants (shared defaults).
pub const DEFAULT_MERGE_IMPACT_METRIC: &str = "min_child";
pub const DEFAULT_MAX_MERGE_EVENTS: usize = 500;

/// Map an `SsnNavigatorConfig` to options. Defaults mirror Python's: mode `overview`,
/// `min_size` 1, `top_n` 50, `max_merge_events` 500, and a `threshold` of `None`
/// meaning the floor (one cluster per connected component), **not** infinity.
pub fn resolve_ssn_navigator(cfg: &SsnNavigatorConfig) -> SsnNavigatorOptions {
    SsnNavigatorOptions {
        mode: cfg
            .mode
            .as_deref()
            .map(NavMode::parse)
            .transpose()
            .unwrap_or(None)
            .unwrap_or(NavMode::Overview),
        threshold: cfg.threshold.clone(),
        cluster_id: cfg.id,
        node: cfg.node.clone(),
        min_size: cfg.min_size.unwrap_or(1),
        top_n: cfg.top_n.unwrap_or(50),
        members: cfg.members,
        max_merge_events: cfg.max_merge_events.unwrap_or(DEFAULT_MAX_MERGE_EVENTS),
    }
}
