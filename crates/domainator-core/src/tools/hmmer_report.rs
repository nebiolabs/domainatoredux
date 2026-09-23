//! `hmmer_report`: tabulate HMM-profile metadata to TSV (or a JSON array). A port
//! of `domainator.hmmer_report` (TSV output; the HTML writer is out of scope,
//! mirroring how `enum_report`'s HTML was deferred — `--json` is offered instead).
//!
//! Columns follow flag order, exactly like `enum_report`. The leading `name`
//! column is always present; the requested analyses append after it.

use std::io::Write;
use std::path::Path;

use hmmer_pure_rs::Hmm;

/// Column value type (drives TSV/JSON serialization).
#[derive(Clone, Copy, PartialEq)]
pub enum ColType {
    Str,
    Int,
    Float,
}

/// A computed cell value.
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
}

/// One requested analysis: the column it contributes and how to compute it.
pub struct Analysis {
    pub column: String,
    pub col_type: ColType,
    kind: Kind,
}

enum Kind {
    Source,
    Acc,
    Desc,
    Length,
    Consensus,
    Alphabet,
    /// A literal value appended to every row (`--append name type value`).
    Append(Value),
}

/// Build a static (no-argument) analysis from its flag name. Returns `None`
/// for an unknown name.
pub fn static_analysis(name: &str) -> Option<Analysis> {
    Some(match name {
        "source" => Analysis { column: "source".into(), col_type: ColType::Str, kind: Kind::Source },
        "acc" => Analysis { column: "acc".into(), col_type: ColType::Str, kind: Kind::Acc },
        "desc" => Analysis { column: "desc".into(), col_type: ColType::Str, kind: Kind::Desc },
        "length" => Analysis { column: "length".into(), col_type: ColType::Int, kind: Kind::Length },
        "consensus" => {
            Analysis { column: "consensus".into(), col_type: ColType::Str, kind: Kind::Consensus }
        }
        "alphabet" => {
            Analysis { column: "alphabet".into(), col_type: ColType::Str, kind: Kind::Alphabet }
        }
        _ => return None,
    })
}

/// Build an `--append` analysis: a constant column of the given type.
pub fn append_analysis(name: String, ctype: &str, value: &str) -> Result<Analysis, String> {
    let (t, v) = match ctype {
        "int" => (
            ColType::Int,
            Value::Int(value.parse().map_err(|_| format!("append int value {value:?}"))?),
        ),
        "float" => (
            ColType::Float,
            Value::Float(value.parse().map_err(|_| format!("append float value {value:?}"))?),
        ),
        "str" => (ColType::Str, Value::Str(value.to_string())),
        _ => return Err("append spec must be one of int, float, str".into()),
    };
    Ok(Analysis { column: name, col_type: t, kind: Kind::Append(v) })
}

/// The profile name (`NAME` line) as a string.
fn profile_name(hmm: &Hmm) -> String {
    hmm.name.clone()
}

/// The profile's alphabet, spelled as pyhmmer's `Alphabet.type` does: `amino`,
/// `DNA` or `RNA` (note the case difference between the three).
fn alphabet_name(hmm: &Hmm) -> &'static str {
    use hmmer_pure_rs::alphabet::AlphabetType;
    match hmm.abc_type {
        AlphabetType::Amino => "amino",
        AlphabetType::Dna => "DNA",
        AlphabetType::Rna => "RNA",
        AlphabetType::Unknown => "",
    }
}

/// The consensus residue line (nodes 1..=M), matching pyhmmer's `rec.consensus`.
fn consensus_str(hmm: &Hmm) -> String {
    match &hmm.consensus {
        Some(c) if c.len() > hmm.m => {
            String::from_utf8_lossy(&c[1..=hmm.m]).into_owned()
        }
        _ => String::new(),
    }
}

impl Analysis {
    fn compute(&self, source: &str, hmm: &Hmm) -> Value {
        match &self.kind {
            Kind::Source => Value::Str(source.to_string()),
            Kind::Acc => Value::Str(hmm.acc.clone().unwrap_or_default()),
            Kind::Desc => Value::Str(hmm.desc.clone().unwrap_or_default()),
            Kind::Length => Value::Int(hmm.m as i64),
            Kind::Consensus => Value::Str(consensus_str(hmm)),
            Kind::Alphabet => Value::Str(alphabet_name(hmm).to_string()),
            Kind::Append(v) => match v {
                Value::Int(i) => Value::Int(*i),
                Value::Float(f) => Value::Float(*f),
                Value::Str(s) => Value::Str(s.clone()),
            },
        }
    }
}

/// Output serialization for [`hmmer_report`].
#[derive(Clone, Copy, PartialEq)]
pub enum OutputFormat {
    /// Tab-separated values (the Python-parity format).
    Tsv,
    /// A JSON array of row objects (agent-friendly; native to this port).
    Json,
}

/// Format one value for TSV output.
fn format_value(v: &Value) -> String {
    match v {
        Value::Str(s) => s.replace('\t', " "),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => {
            // Match Python's default float stringification for `2.0` -> "2.0".
            if f.fract() == 0.0 && f.is_finite() {
                format!("{f:.1}")
            } else {
                f.to_string()
            }
        }
    }
}

/// Convert a cell to a typed JSON value.
fn json_value(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Str(s) => J::String(s.clone()),
        Value::Int(i) => J::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f).map(J::Number).unwrap_or(J::Null),
    }
}

/// Load HMM profiles from `.hmm` files, returning `(source_name, profiles)` per
/// file. `source_name` is [`crate::io::db_name_from_path`] (`pdonr_hmms.hmm` and
/// `pdonr_hmms.hmm.gz` both -> `pdonr_hmms`).
pub fn read_hmms_to_iterators(hmm_files: &[String]) -> Result<Vec<(String, Vec<Hmm>)>, String> {
    let mut out = Vec::new();
    for f in hmm_files {
        let name = crate::io::db_name_from_path(f);
        let hmms = crate::search::hmm_read::read_hmm_file(Path::new(f))
            .map_err(|e| format!("reading {f}: {e:?}"))?;
        out.push((name, hmms));
    }
    Ok(out)
}

/// Run the report, writing to `out` in `format`.
pub fn hmmer_report<W: Write>(
    records: &[(String, Vec<Hmm>)],
    analyses: &[Analysis],
    column_names: Option<&[String]>,
    format: OutputFormat,
    out: &mut W,
) -> Result<(), String> {
    // Header: leading `name`, then one column per analysis (flag order).
    let mut headers: Vec<String> = vec!["name".to_string()];
    for a in analyses {
        headers.push(a.column.clone());
    }
    if let Some(cn) = column_names {
        if cn.len() != headers.len() {
            return Err(format!(
                "The number of override column names must be the same as the number of columns.\nColumns:\n{}\nOverride columns:\n{}",
                headers.join("; "),
                cn.join("; ")
            ));
        }
        headers = cn.to_vec();
    }

    // Build all rows.
    let mut rows: Vec<Vec<Value>> = Vec::new();
    for (source, hmms) in records {
        for hmm in hmms {
            let mut row: Vec<Value> = vec![Value::Str(profile_name(hmm))];
            for a in analyses {
                row.push(a.compute(source, hmm));
            }
            rows.push(row);
        }
    }

    match format {
        OutputFormat::Tsv => {
            writeln!(out, "{}", headers.join("\t")).map_err(|e| e.to_string())?;
            for row in &rows {
                let cells: Vec<String> = row.iter().map(format_value).collect();
                writeln!(out, "{}", cells.join("\t")).map_err(|e| e.to_string())?;
            }
        }
        OutputFormat::Json => {
            // Streaming NDJSON, one compact object per profile -- upstream's agent
            // contract, not a JSON array.
            for row in rows.iter() {
                let mut obj = serde_json::Map::new();
                for (i, v) in row.iter().enumerate() {
                    obj.insert(headers[i].clone(), json_value(v));
                }
                let line = serde_json::to_string(&serde_json::Value::Object(obj))
                    .map_err(|e| e.to_string())?;
                writeln!(out, "{line}").map_err(|e| e.to_string())?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_typing() {
        assert!(matches!(append_analysis("n".into(), "int", "1").unwrap().col_type, ColType::Int));
        assert!(matches!(
            append_analysis("n".into(), "float", "2.0").unwrap().col_type,
            ColType::Float
        ));
        assert!(matches!(append_analysis("n".into(), "str", "x").unwrap().col_type, ColType::Str));
        assert!(append_analysis("n".into(), "bogus", "x").is_err());
        assert!(append_analysis("n".into(), "int", "notanint").is_err());
    }

    #[test]
    fn static_names() {
        assert_eq!(static_analysis("source").unwrap().column, "source");
        assert_eq!(static_analysis("acc").unwrap().column, "acc");
        assert_eq!(static_analysis("desc").unwrap().column, "desc");
        assert_eq!(static_analysis("length").unwrap().column, "length");
        assert!(matches!(static_analysis("length").unwrap().col_type, ColType::Int));
        assert_eq!(static_analysis("consensus").unwrap().column, "consensus");
        assert!(static_analysis("bogus").is_none());
    }

    #[test]
    fn append_float_formats_like_python() {
        assert_eq!(format_value(&Value::Float(2.0)), "2.0");
        assert_eq!(format_value(&Value::Int(1)), "1");
        assert_eq!(format_value(&Value::Str("three".into())), "three");
    }
}
