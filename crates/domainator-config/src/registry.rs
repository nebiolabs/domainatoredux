//! The catalog of tools: what `describe` needs to know about each one that its clap
//! arguments cannot say.
//!
//! Mirrors upstream's `domainator.cli.registry` (categories and one-line summaries)
//! plus the per-parser facts upstream attaches to its argparse actions: which outputs
//! may be standard output, whether `-i -` is allowed, which outputs a run needs at least
//! one of. The CLI checks that every subcommand has an entry here and vice versa.

use crate::columns::{ColumnDef, ENUM_COLUMNS, HMMER_COLUMNS};

/// `(key, heading)` pairs, in the order upstream's `domainator --help` lists them.
pub const CATEGORIES: &[(&str, &str)] = &[
    ("annotate", "Annotate and search sequences and structures"),
    ("edit", "Select, filter, and edit annotated records"),
    ("convert", "Convert between formats"),
    ("profile", "Build, select, and compare HMM and CM profiles"),
    ("compare", "Compute similarity and distance matrices"),
    (
        "network",
        "Build, explore, and split similarity networks, trees, and projections",
    ),
    ("report", "Tables and summaries"),
    ("plot", "Figures"),
    ("database", "Download and prepare databases"),
];

/// One tool.
#[derive(Clone, Copy, Debug)]
pub struct ToolInfo {
    /// The subcommand, equal to upstream's tool name.
    pub name: &'static str,
    /// One of [`CATEGORIES`]' keys.
    pub category: &'static str,
    /// One line, shown by `--help` and `describe`.
    pub summary: &'static str,
    /// Listed by `describe`. The dev commands `inspect` and `convert` are not.
    pub in_manifest: bool,
    /// The tool's interface is a `crate::args` struct, and its `--config` file is
    /// expanded into command-line arguments (upstream's flat format). The rest still
    /// read the old per-tool config structs until they are migrated.
    pub migrated: bool,
    /// Whether `-i -` (standard input) is accepted; tools that read their input more
    /// than once need a file.
    pub stdin: bool,
    /// Outputs that cannot be written to standard output.
    pub no_stdout: &'static [&'static str],
    /// A run must give at least one of these outputs. `None`: no such rule.
    pub required_outputs: Option<&'static [&'static str]>,
    /// Options whose default depends on the machine (a path under the home or temp
    /// directory), which `describe` leaves out, as upstream does.
    pub dynamic_defaults: &'static [&'static str],
    /// `--x`/`--no_x` switches whose default is on.
    pub switches_on: &'static [&'static str],
    /// The catalog of a report's `--columns` (`describe` lists it under the option).
    pub columns: Option<&'static [ColumnDef]>,
    /// Upstream options this port does not have, each with upstream's default as JSON.
    /// A config file may name one at that default (or null), so upstream's
    /// `--print_config` output works here; any other value is an error that says the
    /// option is not supported. The golden test checks each against upstream's
    /// manifest.
    pub unsupported: &'static [(&'static str, &'static str)],
}

const fn tool(name: &'static str, category: &'static str, summary: &'static str) -> ToolInfo {
    ToolInfo {
        name,
        category,
        summary,
        in_manifest: true,
        migrated: false,
        stdin: true,
        no_stdout: &[],
        required_outputs: None,
        dynamic_defaults: &[],
        switches_on: &[],
        columns: None,
        unsupported: &[],
    }
}

/// Every subcommand of `domainatoredux` that is a tool (not `describe`, `completion`,
/// `schema`), in upstream's order, then the port-only ones.
pub const TOOLS: &[ToolInfo] = &[
    // annotate
    ToolInfo {
        migrated: true,
        stdin: false,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        dynamic_defaults: &["ncbi_taxonomy_path"],
        unsupported: &[
            ("foldseek", "null"),
            ("esm2_3di_weights", "null"),
            ("esm2_3di_device", "\"cuda:0\""),
            ("foldseek_device", "\"cpu\""),
            ("foldseek_path", "null"),
            ("taxonomy_update", "false"),
        ],
        ..tool(
            "domainate",
            "annotate",
            "Annotate contigs or proteins with hits to HMM, sequence, or CM references.",
        )
    },
    ToolInfo {
        migrated: true,
        stdin: false,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        dynamic_defaults: &["ncbi_taxonomy_path"],
        unsupported: &[("taxonomy_update", "false")],
        ..tool(
            "domain_search",
            "annotate",
            "Search a large database with a few references and extract the hits.",
        )
    },
    // edit
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        no_stdout: &["log_out"],
        ..tool(
            "clean_sequences",
            "edit",
            "Clean sequence names and residues, and filter out sequences with non-canonical residues.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        no_stdout: &["color_table_out"],
        ..tool(
            "color_genbank",
            "edit",
            "Add color qualifiers to CDS and domain features.",
        )
    },
    ToolInfo {
        migrated: true,
        stdin: false,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        no_stdout: &["log_out", "cluster_tsv_out"],
        ..tool(
            "deduplicate_genbank",
            "edit",
            "Reduce redundancy by clustering sequences (cd-hit, usearch, diamond, or exact hashes).",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        ..tool(
            "extract_domains",
            "edit",
            "Extract annotated domains as their own records.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        ..tool(
            "extract_peptides",
            "edit",
            "Extract translated CDSs as protein records, keeping their domain annotations.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        ..tool(
            "extract_unannotated",
            "edit",
            "Extract the regions of proteins not covered by any domain annotation.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        ..tool(
            "filter_domains",
            "edit",
            "Filter domain annotations by e-value, overlap, database, or name.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        switches_on: &["deduplicate"],
        ..tool(
            "select_by_cds",
            "edit",
            "Extract contigs or neighborhoods around selected CDSs.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        dynamic_defaults: &["ncbi_taxonomy_path"],
        unsupported: &[("taxonomy_update", "false")],
        ..tool(
            "select_by_contig",
            "edit",
            "Select whole contigs by their domain content and other properties.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        ..tool(
            "select_by_coord",
            "edit",
            "Extract contig regions listed in a coordinate table.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        ..tool(
            "sort_contigs",
            "edit",
            "Sort contigs by their Domain_Search score.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["genbank_out", "fasta_out"]),
        ..tool("trim_contigs", "edit", "Trim the ends of contigs.")
    },
    // convert
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["fasta_out"]),
        ..tool(
            "genbank_to_fasta",
            "convert",
            "Convert GenBank files to FASTA.",
        )
    },
    // profile
    tool(
        "hmmer_build",
        "profile",
        "Build an HMM profile from a multiple sequence alignment.",
    ),
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["hmm_out"]),
        ..tool(
            "hmmer_select",
            "profile",
            "Extract a subset of profiles from HMM files.",
        )
    },
    tool(
        "hmmer_search",
        "profile",
        "Select HMM profiles by their similarity to reference profiles.",
    ),
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["tsv_out", "alignments_out"]),
        ..tool(
            "hmmer_compare",
            "profile",
            "Score and align pairs of HMM profiles.",
        )
    },
    // compare
    ToolInfo {
        migrated: true,
        stdin: false,
        required_outputs: Some(&["hdf_out", "dense_text_out"]),
        no_stdout: &["hdf_out"],
        ..tool(
            "seq_dist",
            "compare",
            "Compute similarity scores between sequences or profiles.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["hdf_out", "dense_text_out"]),
        no_stdout: &["hdf_out"],
        unsupported: &[
            ("genbank_out", "null"),
            ("name_by_order", "false"),
            ("linkage", "\"average\""),
            ("leaf_ordering", "\"greedy\""),
            ("skip_sort_memory_check", "false"),
        ],
        ..tool(
            "compare_contigs",
            "compare",
            "Compute similarities between gene neighborhoods from their domain content and order.",
        )
    },
    ToolInfo {
        migrated: true,
        stdin: false,
        required_outputs: Some(&["hdf_out", "dense_text_out"]),
        no_stdout: &["hdf_out"],
        ..tool(
            "transform_matrix",
            "compare",
            "Convert score matrices into normalized scores or distances.",
        )
    },
    // network
    ToolInfo {
        migrated: true,
        stdin: false,
        required_outputs: Some(&["xgmml_out"]),
        no_stdout: &["xgmml_out", "color_table_out", "cluster_tsv_out"],
        ..tool(
            "build_ssn",
            "network",
            "Build a sequence similarity network as a Cytoscape file.",
        )
    },
    ToolInfo {
        migrated: true,
        stdin: false,
        required_outputs: Some(&["dsnv_out", "html_out"]),
        no_stdout: &["dsnv_out", "html_out"],
        ..tool(
            "build_ssn_viewer",
            "network",
            "Build a Domainator Similarity Network Viewer bundle and HTML page.",
        )
    },
    ToolInfo {
        migrated: true,
        stdin: false,
        required_outputs: Some(&["json_out"]),
        ..tool(
            "ssn_navigator",
            "network",
            "Query a similarity network bundle: clusters, members, and metadata.",
        )
    },
    // report
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["tsv_out", "json_out"]),
        dynamic_defaults: &["ncbi_taxonomy_path"],
        columns: Some(ENUM_COLUMNS),
        unsupported: &[
            ("html_out", "null"),
            ("html_max_height", "null"),
            ("taxonomy_update", "false"),
        ],
        ..tool(
            "enum_report",
            "report",
            "Tabulate properties of each contig, CDS, or domain.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&["tsv_out", "json_out"]),
        columns: Some(HMMER_COLUMNS),
        unsupported: &[("html_out", "null"), ("html_max_height", "null")],
        ..tool(
            "hmmer_report",
            "report",
            "Tabulate properties of each HMM profile.",
        )
    },
    ToolInfo {
        migrated: true,
        required_outputs: Some(&[
            "text_out",
            "json_out",
            "domains_tsv_out",
            "co_occurrence_fraction_out",
            "co_occurrence_count_out",
        ]),
        no_stdout: &[
            "domains_tsv_out",
            "co_occurrence_fraction_out",
            "co_occurrence_count_out",
        ],
        dynamic_defaults: &["ncbi_taxonomy_path"],
        unsupported: &[("html_out", "null"), ("taxonomy_update", "false")],
        ..tool(
            "summary_report",
            "report",
            "Summarize the domain and taxonomy content of a dataset.",
        )
    },
    ToolInfo {
        migrated: true,
        stdin: false,
        required_outputs: Some(&["text_out", "json_out"]),
        unsupported: &[("html_out", "null"), ("knn_projection", "false")],
        ..tool("matrix_report", "report", "Summarize a similarity matrix.")
    },
    // plot
    tool(
        "color_table_to_legend",
        "plot",
        "Draw an SVG legend from a color table.",
    ),
    // database
    tool(
        "format_db",
        "database",
        "Shard, compress, and index a database for domain_search.",
    ),
    // port-only
    tool(
        "gene_call",
        "annotate",
        "Call genes with Orphos (Prodigal): add CDS features to contigs (domainatoredux only).",
    ),
    ToolInfo {
        in_manifest: false,
        ..tool(
            "inspect",
            "convert",
            "Parse a sequence file and summarize each record (diagnostic).",
        )
    },
    ToolInfo {
        in_manifest: false,
        ..tool(
            "convert",
            "convert",
            "Read a GenBank or FASTA file and write it back as GenBank (round trip).",
        )
    },
];

/// The registry entry for a subcommand.
pub fn tool_info(name: &str) -> Option<&'static ToolInfo> {
    TOOLS.iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_is_listed_once_in_a_known_category() {
        let mut seen = std::collections::HashSet::new();
        for t in TOOLS {
            assert!(seen.insert(t.name), "{} listed twice", t.name);
            assert!(
                CATEGORIES.iter().any(|(k, _)| *k == t.category),
                "{}: unknown category {}",
                t.name,
                t.category
            );
            for out in t.required_outputs.unwrap_or_default() {
                assert!(out.ends_with("_out"), "{}: {out} is not an output", t.name);
            }
        }
    }
}
