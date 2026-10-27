//! [`SearchResult`] — one domain hit, mirroring domainate's `SearchResult`
//! NamedTuple, plus the conversion to a `Domainator` GenBank feature.

use crate::seq::{Location, SeqFeature};

/// A single domain hit (matches `domainate.SearchResult`). Coordinates: `start`
/// is 0-based (env_from-1), `end` is env_to (half-open); `rstart`/`rend` are the
/// 1-based HMM coords; `rlen` is the HMM length.
#[derive(Debug, Clone, Default)]
pub struct SearchResult {
    pub name: String,
    pub desc: String,
    pub acc: String,
    pub evalue: f64,
    pub score: f64,
    pub start: i64,
    pub end: i64,
    pub database: String,
    pub identity: f64,
    pub rstart: i64,
    pub rend: i64,
    pub rlen: i64,
    pub program: String,
    pub strand: i32,
    /// Optional whole-alignment metrics a structural aligner may report
    /// (`structure_domainate`/`structure_search --metrics`). Each is written only when
    /// present, so GenBank from the sequence-based tools is byte-identical to before.
    ///
    /// They are on their own scales rather than the `score` bitscore scale, so they
    /// should not be compared against `score` or against each other.
    pub tmscore: Option<f64>,
    pub lddt: Option<f64>,
    pub rmsd: Option<f64>,
    pub prob: Option<f64>,
}

/// Format a float like Python's `%.1e`: one-decimal mantissa, signed exponent
/// zero-padded to at least two digits (`6.0e-03`, `1.2e+01`, `4.0e-100`).
pub fn py_sci(x: f64) -> String {
    let s = format!("{:.1e}", x); // Rust: "6.0e-3", "1.2e1", "4.0e-100"
    match s.split_once('e') {
        Some((m, e)) => {
            let exp: i32 = e.parse().unwrap_or(0);
            let sign = if exp < 0 { '-' } else { '+' };
            format!("{m}e{sign}{:02}", exp.abs())
        }
        None => s,
    }
}

/// Format a float like Python's `f"{x:g}"`: six significant digits with trailing zeros
/// (and a bare `.`) stripped, in exponent form when the exponent of the *rounded* value is
/// below -4 or at least 6 (`999999.5` is `1e+06`).
pub fn py_g(x: f64) -> String {
    if x.is_nan() {
        return "nan".to_string();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-inf" } else { "inf" }.to_string();
    }
    let strip = |text: &str| -> String {
        if text.contains('.') {
            text.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            text.to_string()
        }
    };
    if x == 0.0 {
        return if x.is_sign_negative() { "-0" } else { "0" }.to_string();
    }
    let rounded = format!("{x:.5e}"); // Rust: "1.00000e6"
    let (mantissa, exp) = rounded.split_once('e').unwrap_or((&rounded, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    if (-4..6).contains(&exp) {
        let decimals = (5 - exp).unsigned_abs() as usize;
        strip(&format!("{x:.decimals$}"))
    } else {
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{}e{sign}{:02}", strip(mantissa), exp.abs())
    }
}

impl SearchResult {
    /// Build a `Domainator`/`Domain_Search` feature with qualifiers in the exact
    /// order and formatting `add_protein_annotations` uses. `cds_id` is supplied by
    /// the caller (e.g. `0_1_{len}` for a protein contig). `feature_type` is
    /// `DOMAIN_FEATURE_NAME` or `DOMAIN_SEARCH_BEST_HIT_NAME`.
    pub fn to_feature(&self, feature_type: &str, cds_id: &str) -> SeqFeature {
        let desc = if self.desc.trim().is_empty() {
            "."
        } else {
            self.desc.as_str()
        };
        let acc = if self.acc.trim().is_empty() {
            "."
        } else {
            self.acc.as_str()
        };
        let mut f = SeqFeature::new(
            feature_type,
            Location::range(self.start, self.end, self.strand),
        );
        f.push("program", self.program.clone());
        f.push("database", self.database.clone());
        f.push("description", desc);
        f.push("accession", acc);
        f.push("evalue", py_sci(self.evalue));
        f.push("score", format!("{:.1}", self.score));
        f.push("name", self.name.clone());
        f.push("identity", format!("{:.1}", self.identity));
        f.push("cds_id", cds_id);
        f.push("rstart", self.rstart.to_string());
        f.push("rend", self.rend.to_string());
        f.push("rlen", self.rlen.to_string());
        // Structural metrics, when the aligner computed them. Absent for every
        // sequence-based backend, which is what keeps their output unchanged.
        for (key, value) in [
            ("tmscore", self.tmscore),
            ("lddt", self.lddt),
            ("rmsd", self.rmsd),
            ("prob", self.prob),
        ] {
            if let Some(v) = value {
                f.push(key, format!("{v:.3}"));
            }
        }
        f
    }
}

#[cfg(test)]
mod format_tests {
    use super::py_g;

    #[test]
    fn py_g_matches_pythons_g_format() {
        // Each pair is Python's format(x, "g").
        for (x, expected) in [
            (10.0, "10"),
            (4.627564518750634, "4.62756"),
            (0.5, "0.5"),
            (0.0, "0"),
            (-0.0, "-0"),
            (123456.0, "123456"),
            (1234567.0, "1.23457e+06"),
            (999999.5, "1e+06"),
            (999999.4, "999999"),
            (0.0001, "0.0001"),
            (1.234e-05, "1.234e-05"),
            (9.9999996e-05, "0.0001"),
            (71.2, "71.2"),
            (287.0, "287"),
            (-3.5, "-3.5"),
            (1e22, "1e+22"),
            (1.5e-300, "1.5e-300"),
            (100000.0, "100000"),
            (1e6, "1e+06"),
        ] {
            assert_eq!(py_g(x), expected, "{x:?}");
        }
    }
}
