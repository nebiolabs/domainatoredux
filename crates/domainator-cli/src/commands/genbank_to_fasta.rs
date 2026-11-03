use std::path::Path;

use anyhow::{Context, Result};
use domainator_config::args::GenbankToFastaArgs;
use domainator_core::io::genbank::read_genbank;
use domainator_core::io::{detect_filetype, FileType};
use domainator_core::tools::structure_lib::is_structure_path;

use super::read_stdin_genbank;
use crate::output::{is_stdin, write_seq_outputs};

pub fn run(args: &GenbankToFastaArgs) -> Result<()> {
    let mut records = Vec::new();
    for input in &args.input {
        if is_stdin(input) {
            records.extend(read_stdin_genbank()?);
            continue;
        }
        let path = Path::new(input);
        match detect_filetype(path) {
            // Not upstream's: it reads every input as GenBank and so refuses FASTA, but
            // converting FASTA to FASTA is harmless and the port always has.
            Some(FileType::Fasta) => records.extend(crate::load(path)?),
            None if is_structure_path(input) => return Err(crate::unknown_file_type(path)),
            // Upstream's `filetype_override="genbank"`: the name says nothing, so a file
            // staged as `input_seqs` reads like `input_seqs.gb`.
            _ => records.extend(read_genbank(input).with_context(|| format!("reading {input}"))?),
        }
    }
    write_seq_outputs(&records, None, args.fasta_out.as_deref())
}
