use anyhow::{anyhow, Context, Result};
use domainator_config::args::SelectByCoordArgs;
use domainator_core::seq::SeqRecord;
use domainator_core::tools::select_by_coord::{parse_coordinate_table, select_by_coord};

use super::read_stdin_genbank;
use crate::output::{is_stdin, write_seq_outputs};

pub fn run(args: &SelectByCoordArgs) -> Result<()> {
    let table = std::fs::read_to_string(&args.coords)
        .with_context(|| format!("reading coordinate table {}", args.coords))?;
    let selections = parse_coordinate_table(&table).map_err(|e| anyhow!(e))?;
    // Each record with the basename of its file, which the table's file column names.
    let mut records: Vec<(String, SeqRecord)> = Vec::new();
    for path in &args.input {
        if is_stdin(path) {
            records.extend(
                read_stdin_genbank()?
                    .into_iter()
                    .map(|r| ("<stdin>".to_string(), r)),
            );
            continue;
        }
        let p = std::path::Path::new(path);
        let base = p
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        records.extend(crate::load(p)?.into_iter().map(|r| (base.clone(), r)));
    }
    let out = select_by_coord(&records, &selections).map_err(|e| anyhow!(e))?;
    write_seq_outputs(&out, args.genbank_out.as_deref(), args.fasta_out.as_deref())
}
