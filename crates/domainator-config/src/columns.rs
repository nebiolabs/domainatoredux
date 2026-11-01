//! The `--columns` option of the table reports (`enum_report`, `hmmer_report`),
//! upstream's `report_columns.py`.
//!
//! Each report has a catalog of columns. `--columns` takes column tokens in the order
//! the columns should appear: a column's name, then its arguments if it has any,
//! separated by `:`, as in `--columns domains taxid:genus qualifier:CDS:product`. The
//! last argument takes the rest of the token, so it can contain `:`; a column whose
//! last argument repeats (`taxid:genus:species`) splits on every `:` instead.
//!
//! A token is checked against the catalog when the command line is parsed (it is the
//! option's clap value type), so a config file or a GUI job with a bad column fails
//! before any input is read. The catalog is also what `describe` lists under the
//! option's `columns` and what the GUI's column builder offers.

use std::fmt;

use anyhow::{anyhow, Result};
use domainator_core::tools::enum_report::{self as er, Analysis};
use domainator_core::tools::hmmer_report as hr;

/// One column a report offers.
#[derive(Clone, Copy, Debug)]
pub struct ColumnDef {
    pub name: &'static str,
    pub help: &'static str,
    /// The names of the column's arguments.
    pub args: &'static [&'static str],
    /// The last argument takes one or more values (upstream's `VARIADIC`).
    pub repeat_last: bool,
}

const fn col(name: &'static str, help: &'static str) -> ColumnDef {
    ColumnDef {
        name,
        help,
        args: &[],
        repeat_last: false,
    }
}

const fn with_args(
    name: &'static str,
    help: &'static str,
    args: &'static [&'static str],
) -> ColumnDef {
    ColumnDef {
        name,
        help,
        args,
        repeat_last: false,
    }
}

const fn repeating(
    name: &'static str,
    help: &'static str,
    args: &'static [&'static str],
) -> ColumnDef {
    ColumnDef {
        name,
        help,
        args,
        repeat_last: true,
    }
}

const APPEND: ColumnDef = with_args(
    "append",
    "a column with this name and type (str, int or float) holding this value in every row.",
    &["name", "type", "value"],
);

/// `enum_report`'s columns, in upstream's order.
pub const ENUM_COLUMNS: &[ColumnDef] = &[
    col("domains", "the domain names, in alphabetical order and separated by '; ', each listed once."),
    col("domain_descriptions", "the domain descriptions, in alphabetical order of domain name and separated by '; ', each listed once."),
    col("score", "the sum of the domain scores."),
    col("identity", "the sum of the domain identities."),
    col("architecture", "the domain names in order of start position, separated by '; '."),
    col("architecture_detailed", "the domain architecture in order of start position. Within each CDS, domains are separated by '/'; CDSs are separated by ';'. CDSs with no domains are 'no_domains'."),
    col("start", "the start coordinate of the feature."),
    col("end", "the end coordinate of the feature."),
    col("strand", "the strand of the feature."),
    col("length", "the length of the contig, CDS or domain."),
    col("topology", "the topology (usually only meaningful with --by contig)."),
    col("sequence", "the sequence of the contig, CDS or domain."),
    col("cds_count", "the number of CDSs."),
    col("domain_count", "the total number of domains."),
    col("definition", "the record's definition line."),
    col("filename", "the name of the input file the record came from."),
    col("domain_search", "several columns describing the record's best domain_search hit: best_query, query_description, query_start, query_end, query_length, match_start, match_end, match_strand, match_score, match_identity."),
    col("taxid_lineage", "the taxids from the root to the record's taxid, separated by ';'."),
    col("taxname_lineage", "the taxon names from the root to the record's taxid, separated by ';'."),
    col("rank_lineage", "the ranks from the root to the record's taxid, separated by ';'."),
    repeating("taxid", "the taxid at each given rank (e.g. domain, genus, species; 'self' for the record's own taxid), in columns named taxid_[rank].", &["rank"]),
    repeating("taxname", "the taxon name at each given rank (e.g. domain, genus, species; 'self' for the record's own taxon), in columns named taxname_[rank].", &["rank"]),
    with_args("qualifier", "the values of this qualifier on features of this type, separated by ';'.", &["feature", "qualifier"]),
    with_args("feature_count", "the number of features of this type.", &["feature"]),
    with_args("motif_count", "the number of matches of this PROSITE motif in the sequence (nucleotide motifs for nucleotide sequences, protein motifs for proteins; see https://emboss.sourceforge.net/apps/release/6.5/emboss/apps/fuzzpro.html).", &["motif"]),
    col("isoelectric_point", "the isoelectric point of the protein. Only defined for contigs with exactly one protein (a nucleotide contig with one CDS, or a protein record)."),
    with_args("net_charge", "the net charge of the protein at this pH. Only defined for contigs with exactly one protein.", &["pH"]),
    with_args("repeat_count", "the number of repeats of at least this length; for example AGGKKK has repeat_count_1 = 3, repeat_count_2 = 2 and repeat_count_3 = 1.", &["length"]),
    with_args("named_domain_count", "the number of Domainator domains with this name, in a column named named_domain_count_[domain].", &["domain"]),
    col("starts_with", "the first character of the sequence."),
    col("molecular_weight", "the molecular weight of the sequence in Daltons, for protein, DNA or RNA according to its molecule type."),
    col("partial_count", "the number of partial CDSs: by contig, the CDSs with '<' or '>' in their location (for a protein record, 1 if it is a fragment); by cds, 1 if the CDS or protein is a fragment, otherwise 0; by domain, 1 if its CDS or protein is a fragment, whether or not the domain reaches the missing end."),
    col("fragment", "whether the protein or CDS is a fragment: complete, N (missing the N-terminus), C (missing the C-terminus), NC (missing both), or ? (missing ends unknown, e.g. from a UniProt ' (Fragment)' or MGnify 'FL=0' header). Only for rows that are a single protein or CDS; otherwise empty, with a warning."),
    APPEND,
];

/// `hmmer_report`'s columns, in upstream's order.
pub const HMMER_COLUMNS: &[ColumnDef] = &[
    col("source", "the name of the hmm file."),
    col("acc", "the profile's accession."),
    col("desc", "the profile's description."),
    col("length", "the length of the profile (the number of nodes)."),
    col("consensus", "the profile's consensus residue line, if set."),
    col("alphabet", "the profile's alphabet (amino, DNA, or RNA)."),
    APPEND,
];

impl ColumnDef {
    /// How the token is written, for messages and help: `qualifier:feature:qualifier`,
    /// `taxid:rank[:rank...]`.
    pub fn token_format(&self) -> String {
        match self.args {
            [] => self.name.to_string(),
            [one] if self.repeat_last => format!("{}:{one}[:{one}...]", self.name),
            args => format!("{}:{}", self.name, args.join(":")),
        }
    }
}

/// One parsed column: a catalog entry and its argument values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub args: Vec<String>,
}

impl fmt::Display for Column {
    /// The token again: parsing it gives back this column.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        for a in &self.args {
            write!(f, ":{a}")?;
        }
        Ok(())
    }
}

/// Parse one `--columns` token (upstream `parse_columns`, with its messages).
pub fn parse_column(token: &str, catalog: &[ColumnDef]) -> Result<Column, String> {
    let (name, rest) = token.split_once(':').unwrap_or((token, ""));
    let Some(def) = catalog.iter().find(|c| c.name == name) else {
        let names: Vec<&str> = catalog.iter().map(|c| c.name).collect();
        return Err(format!(
            "Unknown column '{name}' in --columns. Columns: {}.",
            names.join(", ")
        ));
    };
    if def.args.is_empty() {
        if !rest.is_empty() {
            return Err(format!(
                "Column '{name}' takes no arguments, but got '{token}'."
            ));
        }
        return Ok(Column {
            name: name.to_string(),
            args: Vec::new(),
        });
    }
    if rest.is_empty() {
        return Err(format!(
            "Column '{name}' needs arguments: --columns {}",
            def.token_format()
        ));
    }
    let args: Vec<String> = if def.repeat_last {
        rest.split(':').map(str::to_string).collect()
    } else {
        let args: Vec<String> = rest
            .splitn(def.args.len(), ':')
            .map(str::to_string)
            .collect();
        if args.len() != def.args.len() {
            return Err(format!(
                "Column '{name}' takes {} arguments: --columns {}",
                def.args.len(),
                def.token_format()
            ));
        }
        args
    };
    Ok(Column {
        name: name.to_string(),
        args,
    })
}

/// The `--columns` help: how to write a token, then the catalog.
pub fn columns_help(catalog: &[ColumnDef]) -> String {
    let mut lines = vec![
        "The columns to report, in order. A column with arguments takes them after ':', as in \
         'qualifier:CDS:product'. Can be given more than once. Columns:"
            .to_string(),
    ];
    lines.extend(
        catalog
            .iter()
            .map(|c| format!("{}: {}", c.token_format(), c.help)),
    );
    lines.join("\n")
}

/// An `enum_report` column token, checked when the command line is parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumColumn(pub Column);

/// A `hmmer_report` column token, checked when the command line is parsed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HmmerColumn(pub Column);

impl std::str::FromStr for EnumColumn {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        parse_column(s, ENUM_COLUMNS).map(EnumColumn)
    }
}

impl std::str::FromStr for HmmerColumn {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        parse_column(s, HMMER_COLUMNS).map(HmmerColumn)
    }
}

impl fmt::Display for EnumColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Display for HmmerColumn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl EnumColumn {
    /// The analysis that computes this column.
    pub fn to_analysis(&self) -> Result<Analysis> {
        let Column { name, args } = &self.0;
        let arg = |i: usize| args[i].clone();
        let analysis = match name.as_str() {
            "taxid" => er::taxid_analysis(args.clone()),
            "taxname" => er::taxname_analysis(args.clone()),
            "qualifier" => er::qualifier_analysis(arg(0), arg(1)),
            "feature_count" => er::feature_count_analysis(arg(0)),
            "motif_count" => er::motif_count_analysis(arg(0)),
            "net_charge" => er::net_charge_analysis(&args[0]).map_err(|e| anyhow!(e))?,
            "repeat_count" => er::repeat_count_analysis(&args[0]).map_err(|e| anyhow!(e))?,
            "named_domain_count" => er::named_domain_count_analysis(arg(0)),
            "append" => er::append_analysis(arg(0), &args[1], &args[2]).map_err(|e| anyhow!(e))?,
            other => er::static_analysis(other)
                .ok_or_else(|| anyhow!("enum_report column {other} has no analysis"))?,
        };
        Ok(analysis)
    }
}

impl HmmerColumn {
    /// The analysis that computes this column.
    pub fn to_analysis(&self) -> Result<hr::Analysis> {
        let Column { name, args } = &self.0;
        match name.as_str() {
            "append" => {
                hr::append_analysis(args[0].clone(), &args[1], &args[2]).map_err(|e| anyhow!(e))
            }
            other => hr::static_analysis(other)
                .ok_or_else(|| anyhow!("hmmer_report column {other} has no analysis")),
        }
    }
}

/// The analyses for `enum_report` column tokens, in order.
pub fn enum_analyses(tokens: &[EnumColumn]) -> Result<Vec<Analysis>> {
    tokens.iter().map(EnumColumn::to_analysis).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A token made from the catalog's own argument names, for every column.
    fn example(def: &ColumnDef) -> String {
        let mut token = def.name.to_string();
        for a in def.args {
            token.push(':');
            token.push_str(match *a {
                "pH" => "7.0",
                "length" => "2",
                "type" => "int",
                "value" => "3",
                "rank" => "species",
                other => other,
            });
        }
        token
    }

    #[test]
    fn every_column_round_trips_and_has_an_analysis() {
        for def in ENUM_COLUMNS {
            let token = example(def);
            let c: EnumColumn = token.parse().unwrap();
            assert_eq!(c.to_string(), token);
            c.to_analysis()
                .unwrap_or_else(|e| panic!("enum_report {token}: {e}"));
        }
        for def in HMMER_COLUMNS {
            let token = example(def);
            let c: HmmerColumn = token.parse().unwrap();
            assert_eq!(c.to_string(), token);
            c.to_analysis()
                .unwrap_or_else(|e| panic!("hmmer_report {token}: {e}"));
        }
    }

    #[test]
    fn tokens_split_as_upstream_does() {
        let p = |t: &str| parse_column(t, ENUM_COLUMNS);
        assert_eq!(p("taxid:genus:species").unwrap().args, ["genus", "species"]);
        assert_eq!(
            p("append:note:str:a:b").unwrap().args,
            ["note", "str", "a:b"],
            "the last argument keeps its ':'"
        );
        assert_eq!(
            p("qualifier:CDS").unwrap_err(),
            "Column 'qualifier' takes 2 arguments: --columns qualifier:feature:qualifier"
        );
        assert_eq!(
            p("taxid").unwrap_err(),
            "Column 'taxid' needs arguments: --columns taxid:rank[:rank...]"
        );
        assert_eq!(
            p("domains:x").unwrap_err(),
            "Column 'domains' takes no arguments, but got 'domains:x'."
        );
        assert!(p("nope").unwrap_err().starts_with("Unknown column 'nope'"));
    }
}
