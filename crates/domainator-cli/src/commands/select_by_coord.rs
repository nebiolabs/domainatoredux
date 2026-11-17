use anyhow::{anyhow, Context, Result};
use domainator_config::args::SelectByCoordArgs;
use domainator_core::tools::select_by_coord::{parse_coordinate_table, CoordSelector};

use crate::input::{InputMode, SeqInputs};
use crate::output::{is_stdin, SeqWriter};

pub fn run(args: &SelectByCoordArgs) -> Result<()> {
    // Only validated here: the run's pool is sized from it before dispatch.
    super::cpu_threads(args.cpu)?;
    let table = std::fs::read_to_string(&args.coords)
        .with_context(|| format!("reading coordinate table {}", args.coords))?;
    let selections = parse_coordinate_table(&table).map_err(|e| anyhow!(e))?;
    let selector = CoordSelector::new(&selections);
    let mut out = SeqWriter::new(args.genbank_out.as_deref(), args.fasta_out.as_deref(), None)?;
    let mut regions = Vec::new();
    for path in &args.input {
        // The table's file column names a file by its basename.
        let basename = if is_stdin(path) {
            "<stdin>".to_string()
        } else {
            std::path::Path::new(path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone())
        };
        for rec in SeqInputs::new(std::slice::from_ref(path), InputMode::DETECT) {
            let rec = rec?;
            // Upstream reads only the contigs the table names.
            if !selector.wants(&rec.id) {
                continue;
            }
            regions.clear();
            selector
                .select(&basename, &rec, &mut regions)
                .map_err(|e| anyhow!(e))?;
            for region in &regions {
                out.write(region)?;
            }
        }
    }
    out.finish()?;
    Ok(())
}
