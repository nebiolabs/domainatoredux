use super::CommonArgs;

/// Calls genes (Prodigal-style CDS features) on nucleotide contigs with the external
/// Orphos binary. A domainatoredux tool with no upstream counterpart; upstream calls
/// genes inside domainate/domain_search with pyrodigal.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct GeneCallArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// GenBank or FASTA files of nucleotide contigs. Use - to read GenBank from
    /// standard input.
    #[arg(short, long, required = true, num_args = 1..)]
    pub input: Vec<String>,
    /// Write the contigs, with the called genes, to this GenBank file.
    #[arg(short = 'o', long, required = true)]
    pub genbank_out: String,
    /// Whether the sequences in FASTA inputs are protein or nucleotide. GenBank records
    /// declare their own molecule type.
    #[arg(long, default_value = "nucleotide", value_parser = ["nucleotide", "protein"])]
    pub alphabet: String,
    /// meta: pre-trained models chosen per contig (metagenomes, plasmids, phages, short
    /// contigs). single: train on all input records together (one complete genome).
    #[arg(long, default_value = "meta", value_parser = ["meta", "single"])]
    pub mode: String,
    /// unannotated: call only contigs with no CDS. all: delete every CDS (and its
    /// Domainator annotations) and re-call every contig.
    #[arg(long, default_value = "unannotated", value_parser = ["unannotated", "all"])]
    pub scope: String,
    /// Genetic code (1-6, 9-16, 21-25). Default: 11, or in meta mode the table of each
    /// contig's best model.
    #[arg(long)]
    pub translation_table: Option<u8>,
    /// Closed ends: no genes running off the ends of a contig.
    #[arg(long)]
    pub closed: bool,
    /// No genes across runs of N.
    #[arg(long)]
    pub mask: bool,
    /// How many Orphos threads to use. 0: all cores.
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// Path to the orphos binary. Default: $DOMAINATOREDUX_ORPHOS, then a bundled
    /// copy, then orphos on the PATH.
    #[arg(long)]
    pub orphos_path: Option<String>,
}
