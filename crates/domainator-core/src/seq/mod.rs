//! The owned sequence data model: `SeqRecord`, `SeqFeature`, `Location`.
//!
//! Unlike the Python suite (which keeps a heavy Biopython `SeqRecord` for the hit
//! path and a separate `LeanContig` for the bulk scan), we use one owned model.
//! The "lean fast-path" is just borrowing iterators over `SeqRecord` (see
//! [`SeqRecord::cds_peptides`]); no separate type and no materialize step.
//!
//! Field naming note: we do NOT replicate Biopython's id/name swap. The original
//! swapped LOCUS name and VERSION into the "wrong" in-memory fields and stayed
//! bug-compatible forever; we are not bound by that. Here `name` = LOCUS name and
//! `id` = accession.version (the natural mapping). The GenBank writer emits the
//! locus name on the LOCUS line and accession.version on ACCESSION/VERSION, so the
//! on-disk file stays standard and re-readable by the original tool.

pub mod location;
pub mod translate;
pub mod taxid;
pub mod taxonomy;
pub mod protein_analysis;

pub use location::{CompoundOp, Location};

/// Molecule topology (LOCUS line).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Topology {
    #[default]
    Linear,
    Circular,
}

impl Topology {
    pub fn as_str(self) -> &'static str {
        match self {
            Topology::Linear => "linear",
            Topology::Circular => "circular",
        }
    }
}

/// A GenBank REFERENCE block, mirroring Biopython's `Reference`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reference {
    /// The `REFERENCE` line's base-range text after the number (e.g.
    /// "(bases 1 to 2552)"), preserved verbatim for round-tripping.
    pub location_text: Option<String>,
    pub authors: String,
    pub consrtm: String,
    pub title: String,
    pub journal: String,
    pub medline_id: String,
    pub pubmed_id: String,
    pub remark: String,
}

/// GenBank header annotations, mirroring the keys Biopython's parser produces in
/// `SeqRecord.annotations`. `compare_seqrecords` requires every key present in the
/// original to round-trip, so the reader captures these and the writer re-emits
/// them. `molecule_type`, `topology`, `date` live on [`SeqRecord`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Annotations {
    /// ACCESSION tokens (`annotations["accessions"]`); first is primary.
    pub accessions: Vec<String>,
    /// Integer from the VERSION line's `.N` suffix (`annotations["sequence_version"]`).
    pub sequence_version: Option<i64>,
    /// GI number from VERSION (`GI:nnn`), if present.
    pub gi: Option<String>,
    pub division: Option<String>,
    /// Date formatted `DD-MON-YYYY` (as written on the LOCUS line).
    pub date: Option<String>,
    /// KEYWORDS, `.`-stripped and `;`-split (Biopython yields `[""]` for just `.`).
    pub keywords: Vec<String>,
    pub source: Option<String>,
    pub organism: Option<String>,
    /// ORGANISM lineage, `;`-split, trailing `.` stripped.
    pub taxonomy: Vec<String>,
    /// DBLINK lines.
    pub dblink: Vec<String>,
    /// DBSOURCE (genpept).
    pub db_source: Option<String>,
    pub references: Vec<Reference>,
    /// Free COMMENT text (the non-structured part).
    pub comment: Option<String>,
    /// Structured comments: ordered `(block_name, ordered (key, value))`.
    pub structured_comment: Vec<(String, Vec<(String, String)>)>,
    /// The topology the LOCUS line actually *declared* (`annotations["topology"]`),
    /// as opposed to [`SeqRecord::topology`], which is what the geometry code acts
    /// on and defaults to linear. Many records leave the column blank, and
    /// Biopython's writer leaves it blank right back; writing `linear` there is a
    /// gratuitous difference from the reference file.
    pub topology: Option<Topology>,
}

/// One GenBank feature. Qualifiers are an ordered multimap (insertion order is
/// load-bearing for the writer and for `cds_id` / `domainator_<db>` qualifiers).
#[derive(Debug, Clone, PartialEq)]
pub struct SeqFeature {
    pub kind: String,
    pub location: Location,
    pub qualifiers: Vec<(String, Vec<String>)>,
}

impl SeqFeature {
    pub fn new(kind: impl Into<String>, location: Location) -> Self {
        SeqFeature { kind: kind.into(), location, qualifiers: Vec::new() }
    }

    /// First value for `key`, if any.
    pub fn get_first(&self, key: &str) -> Option<&str> {
        self.qualifiers
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, vs)| vs.first())
            .map(String::as_str)
    }

    pub fn has(&self, key: &str) -> bool {
        self.qualifiers.iter().any(|(k, _)| k == key)
    }

    /// Append a value, extending the existing entry for `key` or creating one.
    pub fn push(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        if let Some((_, vs)) = self.qualifiers.iter_mut().find(|(k, _)| k == key) {
            vs.push(value);
        } else {
            self.qualifiers.push((key.to_string(), vec![value]));
        }
    }

    /// Replace all values for `key` with a single value (creating the entry at the
    /// end if absent).
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        if let Some((_, vs)) = self.qualifiers.iter_mut().find(|(k, _)| k == key) {
            *vs = vec![value];
        } else {
            self.qualifiers.push((key.to_string(), vec![value]));
        }
    }
}

/// One sequence record (a "contig" in Domainator terms).
#[derive(Debug, Clone, PartialEq)]
pub struct SeqRecord {
    /// accession.version (or best available identifier).
    pub id: String,
    /// LOCUS name.
    pub name: String,
    /// DEFINITION.
    pub description: String,
    /// Residues. GenBank sequences are upper-cased on read; FASTA case is preserved.
    pub seq: Vec<u8>,
    /// "protein" | "DNA" | "RNA" | ...; `None` only transiently during parsing.
    pub molecule_type: Option<String>,
    pub topology: Topology,
    pub annotations: Annotations,
    pub features: Vec<SeqFeature>,
}

impl SeqRecord {
    pub fn len(&self) -> usize {
        self.seq.len()
    }

    pub fn is_empty(&self) -> bool {
        self.seq.is_empty()
    }

    pub fn is_protein(&self) -> bool {
        matches!(self.molecule_type.as_deref(), Some("protein"))
    }

    /// Iterate searchable CDS features as `(feature_index, peptide)` without
    /// allocating new feature objects — the borrowing "lean" fast-path that
    /// replaces Python's `LeanSearchContig.cds_peptides`. `dropped` names feature
    /// kinds to skip (e.g. cleared `Domainator` annotations), and indices are over
    /// the post-drop feature list. The peptide is the `/translation` if present,
    /// else the translated spliced nucleotide sequence; both alnum-filtered.
    pub fn cds_peptides<'a>(
        &'a self,
        dropped: &'a std::collections::HashSet<String>,
    ) -> impl Iterator<Item = (usize, String)> + 'a {
        self.features
            .iter()
            .filter(move |f| dropped.is_empty() || !dropped.contains(&f.kind))
            .enumerate()
            .filter(|(_, f)| translate::is_searchable_cds(f))
            .map(move |(idx, f)| (idx, translate::cds_peptide(f, &self.seq)))
    }
}
