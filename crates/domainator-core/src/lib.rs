//! Core data model, sequence I/O, and tool logic for the Domainator Rust rewrite.
//!
//! This crate is the reusable library (CLI and the later Tauri GUI both call into
//! it), mirroring the Python suite's "every tool is also a library function"
//! convention. See `/home/sean/.claude/plans/make-a-plan-for-kind-brooks.md`.

pub mod seq;
pub mod io;
pub mod matrix;
pub mod parallel;
pub mod search;
pub mod tools;

/// Feature type for domain annotations added by domainate/domain_search
/// (`DOMAIN_FEATURE_NAME` in the Python `domainator/__init__.py`).
pub const DOMAIN_FEATURE_NAME: &str = "Domainator";

/// Feature type for the single best-hit feature added by domain_search.
pub const DOMAIN_SEARCH_BEST_HIT_NAME: &str = "Domain_Search";

/// NCBI taxid for "unidentified"; the internal default for records lacking a
/// taxon. Never written to output files.
pub const UNIDENTIFIED_TAXID: i64 = 32644;
