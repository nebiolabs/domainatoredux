/// The options every tool takes (upstream `cli.common.tool_parser`), plus `--config`
/// and `--print_config`, which upstream's jsonargparse adds.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(
    rename_all = "snake_case",
    group(clap::ArgGroup::new("verbosity").args(["verbose", "quiet"]).multiple(false))
)]
pub struct CommonArgs {
    /// Show debug output, and the full chain of causes for an error.
    #[arg(short, long)]
    pub verbose: bool,
    /// Show errors only: no progress messages or warnings.
    #[arg(short, long)]
    pub quiet: bool,
    /// Read options from this JSON or YAML file, keyed by option name (as
    /// --print_config writes them). Options given on the command line win.
    #[arg(long, value_name = "FILE")]
    pub config: Option<String>,
    /// Print the options, after reading --config, as a YAML config file, and exit.
    #[arg(long)]
    pub print_config: bool,
}

/// The taxonomy restriction every selecting tool takes (upstream
/// `cli.common.add_taxonomy_filters`), flattened into its args.
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct TaxonomyArgs {
    /// Include these NCBI taxids and their descendants (OR).
    #[arg(long, num_args = 1.., allow_negative_numbers = true)]
    pub include_taxids: Option<Vec<i64>>,
    /// Exclude these NCBI taxids and their descendants; exclusions take precedence.
    #[arg(long, num_args = 1.., allow_negative_numbers = true)]
    pub exclude_taxids: Option<Vec<i64>>,
    /// Boolean expression over taxid lineages using &, |, ~, and parentheses, e.g.
    /// "2 & ~1224". Mutually exclusive with include/exclude_taxids.
    #[arg(long)]
    pub taxonomy_expr: Option<String>,
    /// Unresolved taxonomy: exclude (default), include, evaluate against an empty
    /// lineage, or error. With ~2, exclude rejects unlabeled candidates; evaluate and
    /// include allow them. Only active with taxonomy criteria. Not inverted.
    #[arg(long, default_value = "exclude",
          value_parser = ["exclude", "include", "evaluate", "error"])]
    pub taxonomy_unknown: String,
    /// NCBI taxonomy database directory; downloaded if missing and filtering is
    /// active. It may also be the path of a taxdump.tar.gz. Default:
    /// /tmp/ncbi_taxonomy_USER, USER being your user name.
    #[arg(long)]
    pub ncbi_taxonomy_path: Option<String>,
    /// Check for an updated taxonomy database when filtering is active.
    #[arg(long)]
    pub taxonomy_update: bool,
}
