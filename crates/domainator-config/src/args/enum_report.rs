use super::CommonArgs;
use crate::columns::EnumColumn;

/// Tabulates each contig, CDS or domain of GenBank files (upstream `enum_report.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct EnumReportArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Genbank filenames. Use - to read standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the table to this tab-separated file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub tsv_out: Option<String>,
    /// Write streaming NDJSON (one compact JSON object per record) to this file. Use -
    /// for standard output.
    #[arg(long)]
    pub json_out: Option<String>,
    /// only examine contigs with ids in this list. Additive with --contigs_file.
    /// default: all contigs.
    #[arg(long, num_args = 1..)]
    pub contigs: Vec<String>,
    /// only examine contigs with ids listed in this file (one per line). Additive with
    /// --contigs. default: all contigs.
    #[arg(long)]
    pub contigs_file: Option<String>,
    /// One line in output for every by contig, cds or domain. For protein genbanks,
    /// contig and cds are treated the same.
    #[arg(long, default_value = "contig", ignore_case = true,
          value_parser = ["contig", "cds", "domain"])]
    pub by: String,
    /// Which rows to report, by fragment status. include: all. exclude: skip
    /// fragments. only: only fragments. A CDS is a fragment if its location has '<'
    /// or '>', or it has a /partial qualifier. A protein record is a fragment if it
    /// has a UniProt ' (Fragment)' or MGnify 'FL=0' header, a 'Flags: Fragment'
    /// definition or NON_TER features (UniProt text entries), or a CDS/Protein
    /// feature with '<' or '>' spanning it. By contig, a nucleotide contig is a
    /// fragment if any of its CDSs is. By domain, the CDS or protein the domain is on
    /// decides.
    #[arg(long, default_value = "include", value_parser = ["include", "exclude", "only"])]
    pub partial: String,
    /// Ignore domain annotations not from these databases. default: consider all
    /// databases.
    #[arg(long, num_args = 1..)]
    pub databases: Option<Vec<String>>,
    /// The columns to report, in order, as name:arg:arg tokens (--help lists them).
    #[arg(long, num_args = 1.., value_name = "COLUMN",
          long_help = crate::columns::columns_help(crate::columns::ENUM_COLUMNS))]
    pub columns: Vec<EnumColumn>,
    /// Path to an NCBI taxonomy database: a directory of extracted .dmp files, or a
    /// taxdump.tar.gz. Default: /tmp/ncbi_taxonomy_USER, USER being your user name.
    /// Unlike upstream, it is never downloaded.
    #[arg(long)]
    pub ncbi_taxonomy_path: Option<String>,
    /// If supplied, then this list will be used instead of the default column names.
    #[arg(long, num_args = 1..)]
    pub column_names: Option<Vec<String>>,
}
