//! Sequence inputs, read the way upstream's `parse_seqfiles` reads them: lazily, one
//! record at a time, file after file, so a tool that handles each record on its own
//! holds one record in memory however large its input is.
//!
//! `-` and `/dev/stdin` are standard input, read as GenBank (gzip or BGZF content is
//! decompressed, as it is for a file). A file's format comes from its name: FASTA
//! records get the molecule type the tool asks for, as upstream's
//! `default_molecule_type`.

use std::collections::{HashSet, VecDeque};
use std::path::Path;

use anyhow::{Context, Result};
use domainator_core::io::genbank::GenbankReader;
use domainator_core::io::{decompress_stream, detect_filetype, fasta, genbank, FileType};
use domainator_core::seq::SeqRecord;
use domainator_core::tools::select_common::parse_name_list;
use domainator_core::tools::structure_lib::is_structure_path;

use crate::output::is_stdin;

/// How a file's format is decided.
#[derive(Clone, Copy, Debug)]
pub enum InputMode<'a> {
    /// By its name: GenBank or FASTA, anything else an error. FASTA records get the
    /// molecule type `fasta_type` (`protein` or `nucleotide`).
    Detect { fasta_type: &'a str },
    /// GenBank whatever the name, as upstream's `filetype_override="genbank"`, except
    /// that a file named as FASTA is read as (protein) FASTA, and a structure file is
    /// refused. `genbank_to_fasta`'s rule: workflow engines stage inputs under names
    /// like `input_seqs`, with no extension to go by.
    GenbankUnlessFasta,
}

impl InputMode<'static> {
    /// [`InputMode::Detect`] with FASTA read as protein, the default for every tool
    /// without an `--alphabet` option.
    pub const DETECT: InputMode<'static> = InputMode::Detect {
        fasta_type: "protein",
    };
}

type Records = Box<dyn Iterator<Item = std::io::Result<SeqRecord>> + Send>;

/// The records of a list of inputs, in order. Each file is opened only when the one
/// before it is used up, and closed as soon as it is.
pub struct SeqInputs<'a> {
    pending: VecDeque<String>,
    mode: InputMode<'a>,
    contigs: Option<HashSet<String>>,
    current: Option<(String, Records)>,
}

impl<'a> SeqInputs<'a> {
    pub fn new(inputs: &[String], mode: InputMode<'a>) -> Self {
        SeqInputs {
            pending: inputs.iter().cloned().collect(),
            mode,
            contigs: None,
            current: None,
        }
    }

    /// Keep only the records whose id is in `--contigs`/`--contigs_file` (upstream's
    /// `parse_seqfiles(..., contigs)`), dropping the rest as they are read. A no-op
    /// when both are empty.
    pub fn contigs(mut self, contigs: &[String], contigs_file: Option<&str>) -> Result<Self> {
        self.contigs = parse_name_list(contigs, contigs_file)?;
        Ok(self)
    }

    /// Keep only the records whose id is in `contigs`, an already parsed name list.
    pub fn with_contigs(mut self, contigs: HashSet<String>) -> Self {
        self.contigs = Some(contigs);
        self
    }

    fn open(&self, input: &str) -> Result<Records> {
        if is_stdin(input) {
            let stream = decompress_stream(std::io::stdin()).context("reading standard input")?;
            return Ok(Box::new(GenbankReader::from_reader(stream, None)));
        }
        let path = Path::new(input);
        let filetype = detect_filetype(path);
        let (filetype, fasta_type) = match self.mode {
            InputMode::Detect { fasta_type } => match filetype {
                Some(t) => (t, fasta_type),
                None => return Err(crate::unknown_file_type(path)),
            },
            InputMode::GenbankUnlessFasta => match filetype {
                Some(FileType::Fasta) => (FileType::Fasta, "protein"),
                None if is_structure_path(input) => return Err(crate::unknown_file_type(path)),
                _ => (FileType::Genbank, "protein"),
            },
        };
        let opened: Records = match filetype {
            FileType::Genbank => Box::new(genbank::iter_genbank(input)?),
            FileType::Fasta => Box::new(fasta::iter_fasta(input, Some(fasta_type))?),
        };
        Ok(opened)
    }

    fn describe(input: &str) -> String {
        if is_stdin(input) {
            "reading standard input".to_string()
        } else {
            format!("reading {input}")
        }
    }
}

impl Iterator for SeqInputs<'_> {
    type Item = Result<SeqRecord>;

    fn next(&mut self) -> Option<Result<SeqRecord>> {
        loop {
            if self.current.is_none() {
                let input = self.pending.pop_front()?;
                match self.open(&input) {
                    Ok(records) => self.current = Some((input, records)),
                    Err(e) => {
                        self.pending.clear();
                        return Some(Err(e.context(Self::describe(&input))));
                    }
                }
            }
            let (input, records) = self.current.as_mut()?;
            match records.next() {
                Some(Ok(rec)) => {
                    if self
                        .contigs
                        .as_ref()
                        .is_some_and(|set| !set.contains(&rec.id))
                    {
                        continue;
                    }
                    return Some(Ok(rec));
                }
                Some(Err(e)) => {
                    let e = anyhow::Error::new(e).context(Self::describe(input));
                    self.current = None;
                    self.pending.clear();
                    return Some(Err(e));
                }
                None => self.current = None,
            }
        }
    }
}

/// Every record of `inputs`, for a tool that needs them all at once.
pub fn load_all(inputs: &[String], mode: InputMode<'_>) -> Result<Vec<SeqRecord>> {
    SeqInputs::new(inputs, mode).collect()
}

/// Each record's results, in input order: `f` turns one record into the records it
/// yields (none, one, or several). For a tool that streams per-record work from
/// [`SeqInputs`] into [`crate::output::write_seq_stream`].
pub fn flat_map_records<I, F>(records: I, mut f: F) -> impl Iterator<Item = Result<SeqRecord>>
where
    I: Iterator<Item = Result<SeqRecord>>,
    F: FnMut(SeqRecord) -> Result<Vec<SeqRecord>>,
{
    records.flat_map(move |rec| match rec.and_then(&mut f) {
        Ok(out) => out.into_iter().map(Ok).collect::<Vec<_>>(),
        Err(e) => vec![Err(e)],
    })
}

/// The values of `iter` up to its first error, which is put in `error`: for a core
/// function that takes plain records. Check `error` after the function returns, before
/// its result, since a read error ends its input early.
pub fn until_error<'a, T: 'a>(
    iter: impl Iterator<Item = Result<T>> + 'a,
    error: &'a mut Option<anyhow::Error>,
) -> impl Iterator<Item = T> + 'a {
    iter.map_while(move |r| match r {
        Ok(v) => Some(v),
        Err(e) => {
            *error = Some(e);
            None
        }
    })
}
