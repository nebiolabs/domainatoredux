//! `structure_to_genbank` (upstream `structure_to_genbank.py`): one protein record per
//! structure chain, so structures become ordinary inputs to the sequence tools.
//!
//! Chain splitting, structure parsing and naming are all done by the aligner (foldseek
//! `createdb`), so records are named exactly as they are in a search against the same
//! structures. Record coordinates index the *observed* residues, not PDB residue
//! numbers.

use std::io;

use super::external::TempDir;
use super::structure_lib::{
    build_protein_record, resolve_structure_inputs, Aligner, Resolved, StructureDb,
};
use crate::seq::SeqRecord;

/// The records for `inputs` (files, globs, directories, or one database prefix).
/// `db_out` keeps the database built along the way; `store_3di` adds each chain's 3Di
/// string as a `/threedi` qualifier.
pub fn structure_to_genbank(
    inputs: &[String],
    aligner: &Aligner,
    db_out: Option<&str>,
    store_3di: bool,
) -> io::Result<Vec<SeqRecord>> {
    match resolve_structure_inputs(inputs)? {
        Resolved::Db(prefix) => {
            if db_out.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "--db_out builds a database from structure files; the input is \
                         already a database ('{prefix}')."
                    ),
                ));
            }
            records(aligner, &StructureDb::prebuilt(&prefix), store_3di)
        }
        Resolved::Structures(files) => match db_out {
            Some(prefix) => records(aligner, &aligner.build_db(&files, prefix)?, store_3di),
            None => {
                let scratch = TempDir::new_in(
                    aligner.tmp_dir.as_deref().map(std::path::Path::new),
                    "domainatoredux_structure",
                )?;
                let prefix = scratch.path().join("structuredb");
                let db = aligner.build_db(&files, &prefix.to_string_lossy())?;
                records(aligner, &db, store_3di)
            }
        },
    }
}

fn records(aligner: &Aligner, db: &StructureDb, store_3di: bool) -> io::Result<Vec<SeqRecord>> {
    let threedi: std::collections::HashMap<String, String> = if store_3di {
        aligner.iter_3di(db)?.into_iter().collect()
    } else {
        Default::default()
    };
    Ok(aligner
        .iter_sequences(db)?
        .into_iter()
        .map(|(name, sequence, source)| {
            let threedi = store_3di
                .then(|| threedi.get(&name).map(String::as_str))
                .flatten();
            build_protein_record(&name, &sequence, source.as_deref(), threedi)
        })
        .collect())
}
