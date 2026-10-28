use super::CommonArgs;

/// Writes the Domainator Viewer page, empty or opening a viewer bundle (upstream
/// `viewer.py`).
#[derive(clap::Args, Clone, Debug, Default)]
#[command(rename_all = "snake_case")]
pub struct ViewerArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// A Domainator Viewer bundle (.dvb) for the page to open on load: from build_ssn,
    /// build_tree or build_projection --dvb_out, or the viewer's Save session. Older
    /// network bundles (.dsnv) work too. Without it, the page opens the bundles you
    /// choose in it.
    #[arg(short, long)]
    pub input: Option<String>,
    /// Write the viewer page to this HTML file.
    #[arg(short = 'o', long)]
    pub html_out: Option<String>,
    /// The page's title. Defaults to the bundle's name.
    #[arg(long)]
    pub name: Option<String>,
}
