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
