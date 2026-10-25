use super::CommonArgs;

/// Formats sequence databases for fast searching: shards them, compresses them as
/// BGZF, and indexes them (upstream `format_db.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct FormatDbArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// GenBank or FASTA database file(s) to format. May be plain, gzip, or BGZF.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Split each input into this many shards (balanced by record count). Shards are
    /// named `<base>.0.<ext>`, `<base>.1.<ext>`, ... Default: no sharding.
    #[arg(long, allow_negative_numbers = true)]
    pub shards: Option<i64>,
    /// Write BGZF (block-gzip) output (gunzip-readable, seekable, ~3-4x smaller).
    #[arg(long)]
    pub compress: bool,
    /// Write a .didx offset index for each output file (lets domain_search skip the
    /// per-run offset scan).
    #[arg(long)]
    pub index: bool,
    /// Directory to write outputs into. Default: alongside each input.
    #[arg(long)]
    pub output_dir: Option<String>,
    /// Base name for the output database (format/compression extensions are kept),
    /// e.g. --name mydb turns 'foo.gb' into 'mydb.gb' / 'mydb.0.gb.bgz'. Requires a
    /// single input; intended for --shards/--compress (warns on index-only runs).
    #[arg(long)]
    pub name: Option<String>,
    /// Number of parallel workers (also threads for the BGZF compressor). 0: one per
    /// physical core.
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// BGZF/deflate compression level (1-12).
    #[arg(long, default_value_t = 6, allow_negative_numbers = true)]
    pub level: i64,
    /// Overwrite existing outputs / rebuild indexes instead of skipping.
    #[arg(long)]
    pub force: bool,
    /// Write the log to this file. If not supplied, writes it to stderr.
    #[arg(long)]
    pub log_out: Option<String>,
}
