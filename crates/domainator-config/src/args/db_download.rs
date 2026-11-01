use super::CommonArgs;

/// The `--db` choices, upstream's sorted `FASTA_DBS | GENBANK_DBS`.
pub const DB_CHOICES: [&str; 13] = [
    "ncbi_all",
    "ncbi_complete_genome_nonredundant_proks",
    "ncbi_complete_genome_proks",
    "ncbi_nonredundant_all",
    "ncbi_nonredundant_proks",
    "ncbi_representative_all",
    "ncbi_representative_proks",
    "swissprot",
    "swissprot_gb",
    "trembl",
    "trembl_gb",
    "uniprot",
    "uniprot_gb",
];

/// Download sequence databases from the internet in formats appropriate for
/// Domainator (upstream `db_download.py`).
///
/// UniProt (SwissProt and TrEMBL) is downloaded as FASTA, or with a _gb suffix as
/// GenBank with UniProt's annotations as features. The NCBI databases are genome
/// assemblies from NCBI GenBank, written as GenBank:
/// ncbi_complete_genome_proks (assembly_level "Complete Genome"),
/// ncbi_complete_genome_nonredundant_proks (the same, one genome per taxid),
/// ncbi_representative_proks and ncbi_representative_all (representative and reference
/// genomes), ncbi_nonredundant_proks and ncbi_nonredundant_all (one genome per taxid),
/// and ncbi_all (every genome, possibly 10 TB or more). The _proks databases exclude
/// Eukaryota. The nonredundant databases keep every metagenome and environmental
/// sample.
///
/// Key taxa: 2 bacteria, 2157 archaea, 2759 eukaryota, 10239 viruses, 81490
/// prokaryotic environmental samples, 408169 metagenomes.
///
/// WARNING: Downloading large databases can take a long time and use a lot of disk
/// space!
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct DbDownloadArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Database to download. swissprot, trembl and uniprot are downloaded as FASTA
    /// (write them with --fasta_out); the _gb UniProt databases and the ncbi databases
    /// are downloaded as GenBank (write them with -o/--genbank_out).
    #[arg(long, required = true, ignore_case = true, value_parser = DB_CHOICES)]
    pub db: String,
    /// Write the downloaded GenBank records to this file. Use - for standard output.
    #[arg(short = 'o', long)]
    pub genbank_out: Option<String>,
    /// Write the downloaded FASTA records to this file. Use - for standard output.
    #[arg(long)]
    pub fasta_out: Option<String>,
    /// Which UniProt records to download, by fragment status: include (all, the
    /// default), exclude (complete proteins only) or only (fragments only). Only for the
    /// UniProt databases.
    #[arg(long, default_value = "include", value_parser = ["include", "exclude", "only"])]
    pub partial: String,
    /// Space separated list of taxids to include.
    #[arg(long, num_args = 1.., allow_negative_numbers = true)]
    pub include_taxids: Option<Vec<i64>>,
    /// Space separated list of taxids to exclude.
    #[arg(long, num_args = 1.., allow_negative_numbers = true)]
    pub exclude_taxids: Option<Vec<i64>>,
    /// A boolean expression over taxids using operators & (AND), | (OR), ~ (NOT), and
    /// parentheses, e.g. "2 & ~1224" (within Bacteria but not Proteobacteria). A taxid
    /// is true for a record when it is in the record's lineage. Mutually exclusive with
    /// --include_taxids/--exclude_taxids. For prokaryote-only (_proks) databases,
    /// Eukaryota are excluded in addition to this expression.
    #[arg(long)]
    pub taxonomy_expr: Option<String>,
    /// Path to NCBI taxonomy database directory. Will be created and downloaded if it
    /// does not exist. Default: /tmp/ncbi_taxonomy_USER, USER being your user name.
    #[arg(long)]
    pub ncbi_taxonomy_path: Option<String>,
    /// If the taxonomy database exists, check it against the version on the NCBI
    /// server and update it if there is a newer version (the default).
    #[arg(long, overrides_with = "no_taxonomy_update")]
    pub taxonomy_update: bool,
    /// Skip the check for a newer version of the taxonomy database.
    #[arg(long, overrides_with = "taxonomy_update")]
    pub no_taxonomy_update: bool,
    /// Path to log file for successfully written records: the accessions of the
    /// genomes written, for the ncbi databases. It is rewritten on every run.
    #[arg(long)]
    pub success_log_out: Option<String>,
    /// Exclude genbank records with these accessions. Should be versionless (i.e.
    /// without the '.' and number after the '.').
    #[arg(long, num_args = 1..)]
    pub exclude_accessions: Option<Vec<String>>,
    /// Text file containing accessions of genbank records to exclude. Should be
    /// versionless (i.e. without the '.' and number after the '.').
    #[arg(long)]
    pub exclude_accessions_file: Option<String>,
    /// For the UniProt databases: read the release files (uniprot_sprot.fasta.gz,
    /// uniprot_trembl.dat.gz, ...) from this directory, a local mirror of UniProt's
    /// knowledgebase/complete directory, instead of downloading them.
    #[arg(long)]
    pub uniprot_dir: Option<String>,
    /// Path to NCBI summary file. Normally it is downloaded from the Internet on every
    /// run, but you may want to pre-download it, or make custom subsets; the url is
    /// <https://ftp.ncbi.nlm.nih.gov/genomes/genbank/assembly_summary_genbank.txt>. In a
    /// local file, an ftp_path that is not a URL is read as a local directory laid out
    /// like the NCBI genomes server (relative to the summary file's directory), for a
    /// local mirror with --download_backend direct.
    #[arg(long)]
    pub ncbi_summary: Option<String>,
    /// For nucleotide databases, such as GenBank, add CDS annotations by calling genes
    /// in metagenomic mode (with Orphos, a Rust Prodigal). If 'all', any existing CDS
    /// annotations are deleted and all contigs are re-annotated. If 'unannotated', only
    /// contigs without CDS annotations are annotated.
    #[arg(long, value_parser = ["all", "unannotated"])]
    pub gene_call: Option<String>,
    /// Stop after downloading this many records. For UniProt, this is the number of
    /// proteins. For genbank, the number of genomes, including multi-contig genomes.
    /// This is mainly for testing.
    #[arg(long, allow_negative_numbers = true)]
    pub max_records: Option<i64>,
    /// Append to the existing output file instead of overwriting it.
    #[arg(long)]
    pub append: bool,
    /// For ncbi databases, only download records released before this date. Format is
    /// YYYY-MM-DD.
    #[arg(long)]
    pub before: Option<String>,
    /// For ncbi databases, only download records released after this date. Format is
    /// YYYY-MM-DD.
    #[arg(long)]
    pub after: Option<String>,
    /// The number of threads used to process downloaded ncbi genomes (gene calling and
    /// conversion). 0: one per physical core.
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    pub cpu: i64,
    /// Network backend for ncbi genome downloads. 'datasets' uses the NCBI Datasets CLI
    /// dehydrate/rehydrate workflow (recommended for large jobs; NCBI's own bulk tool
    /// handles throttling). 'direct' downloads .gbff.gz files over HTTPS directly.
    #[arg(long, default_value = "datasets", ignore_case = true, value_parser = ["direct", "datasets"])]
    pub download_backend: String,
    /// For the 'direct' backend: number of simultaneous HTTPS connections to the NCBI
    /// genomes server. NCBI rate-limits by concurrency, so keep this low (~3). 0 = use
    /// --cpu. With --gene_call set, gene calling runs inside these workers.
    #[arg(long, default_value = "3", allow_negative_numbers = true)]
    pub download_workers: i64,
    /// Contact email included in the User-Agent header sent to NCBI (politeness; not
    /// an API key). Falls back to the NCBI_EMAIL environment variable.
    #[arg(long)]
    pub email: Option<String>,
    /// NCBI API key passed to the 'datasets' backend (does not affect the raw FTP
    /// server / 'direct' backend). Falls back to the NCBI_API_KEY environment variable.
    #[arg(long)]
    pub ncbi_api_key: Option<String>,
    /// For the 'datasets' backend: working directory for the dehydrated package and
    /// rehydrated files. Required when --download_backend is 'datasets'.
    #[arg(long)]
    pub datasets_workdir: Option<String>,
    /// For the 'datasets' backend: value passed to `datasets ... --include`. Only the
    /// GenBank flat file (gbff) is used downstream, so the genomic FASTA is not
    /// requested by default.
    #[arg(long, default_value = "gbff")]
    pub datasets_include: String,
    /// For the 'datasets' backend: value passed to `datasets rehydrate --max-workers`
    /// (allowed range 1-30). 10 is the NCBI Datasets default and matches the 10 req/s
    /// API-key limit.
    #[arg(long, default_value = "10", allow_negative_numbers = true)]
    pub datasets_max_workers: i64,
    /// Path to the `datasets` (NCBI Datasets CLI) binary. If not given, it is looked
    /// up on PATH.
    #[arg(long)]
    pub datasets_path: Option<String>,
    /// For the 'datasets' backend: pass `--gzip` to `datasets rehydrate` so the
    /// rehydrated genomic.gbff files are stored gzip-compressed (much smaller scratch
    /// dir; read transparently). Recommended for very large downloads.
    #[arg(long)]
    pub datasets_gzip: bool,
}
