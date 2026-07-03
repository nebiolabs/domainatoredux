//! Shared config → `*Options` resolution: the default-filling and validation that
//! turns a serde `*Config` (from the CLI or the GUI form) into the borrowed
//! `*Options` a core tool expects. Keeping this here means the CLI and GUI resolve
//! parameters identically and can't drift.
//!
//! Resolvers are **pure**: file-based inputs (subset lists, colour tables) are read
//! by the caller and passed in, so this module needs no filesystem access and the
//! caller keeps control of I/O.

use std::collections::{HashMap, HashSet};

use anyhow::{bail, Result};
use domainator_core::tools::build_ssn::BuildSsnOptions;
use domainator_core::tools::build_ssn_viewer::BuildSsnViewerOptions;
use domainator_core::tools::transform_matrix::{TransformMatrixOptions, MODES};

use crate::{BuildSsnConfig, BuildSsnViewerConfig, TransformMatrixConfig};

/// Default lower bound / merge-event / metric constants (shared defaults).
pub const DEFAULT_MERGE_IMPACT_METRIC: &str = "min_child";
pub const DEFAULT_MAX_MERGE_EVENTS: usize = 500;

pub fn resolve_transform_matrix(cfg: &TransformMatrixConfig) -> Result<TransformMatrixOptions> {
    let mode = cfg.mode.clone().unwrap_or_else(|| "none".into());
    if !MODES.contains(&mode.as_str()) {
        bail!("invalid --mode {mode:?}; choose one of {MODES:?}");
    }
    Ok(TransformMatrixOptions {
        mode,
        input_type: cfg.input_type.clone(),
        lb: cfg.lb,
        mst_knn: cfg.mst_knn,
    })
}

/// Map a `BuildSsnConfig` to options. `subset_labels`/`color_table` are the
/// caller-resolved file inputs (from `--subset`/`--subset_file`/`--color_table`).
pub fn resolve_build_ssn(
    cfg: &BuildSsnConfig,
    subset_labels: Option<HashSet<String>>,
    color_table: Option<HashMap<String, String>>,
) -> BuildSsnOptions {
    BuildSsnOptions {
        lb: cfg.lb.unwrap_or(0.0),
        cluster: cfg.cluster || cfg.cluster_tsv.is_some(),
        mst: cfg.mst,
        mst_knn: cfg.mst_knn,
        metadata_files: cfg.metadata.clone(),
        subset_labels,
        color_by: cfg.color_by.clone(),
        color_table,
    }
}

/// Map a `BuildSsnViewerConfig` to options. `name` is caller-resolved (derived from
/// the output/html stem in the CLI, or the input stem in the GUI); `subset_labels`
/// is the caller-resolved `--subset`/`--subset_file` set.
pub fn resolve_build_ssn_viewer(
    cfg: &BuildSsnViewerConfig,
    name: String,
    subset_labels: Option<HashSet<String>>,
) -> BuildSsnViewerOptions {
    BuildSsnViewerOptions {
        name,
        metadata_files: cfg.metadata.clone(),
        subset_labels,
        merge_impact_metric: cfg
            .merge_impact_metric
            .clone()
            .unwrap_or_else(|| DEFAULT_MERGE_IMPACT_METRIC.into()),
        max_merge_events: cfg.max_merge_events.unwrap_or(DEFAULT_MAX_MERGE_EVENTS),
        color_by: cfg.color_by.clone(),
        label_by: cfg.label_by.clone(),
    }
}
