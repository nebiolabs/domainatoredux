//! `hmmer_select`: filter an HMM file by text matches on profile NAME / DESC /
//! ACC fields. A port of `domainator.hmmer_select`.
//!
//! Matching modes (`--regex`, `--exact`, `--contains`) are combined with OR: a
//! profile is kept if it matches ANY specification on ANY searched field. Output
//! is an HMM file (written via `hmmer_pure_rs::hmmfile::write_hmm`). This is a
//! different HMMER3 writer than pyhmmer, so output is semantically re-readable but
//! not byte-identical (it emits a `HMMER3/e` header); the retained profile SET is
//! the parity contract.

use std::io::Write;
use std::path::Path;

use hmmer_pure_rs::Hmm;
use regex::RegexBuilder;

/// Which profile fields to search.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Fields {
    pub name: bool,
    pub desc: bool,
    pub acc: bool,
}

impl Fields {
    /// Fields for a `--field` value (`name` | `desc` | `acc` | `all`).
    pub fn from_field(field: &str) -> Result<Fields, String> {
        Ok(match field {
            "name" => Fields { name: true, desc: false, acc: false },
            "desc" => Fields { name: false, desc: true, acc: false },
            "acc" => Fields { name: false, desc: false, acc: true },
            "all" => Fields { name: true, desc: true, acc: true },
            other => return Err(format!("--field must be name, desc, acc, or all, got {other:?}")),
        })
    }
}

/// Filter criteria (any may be empty; combined with OR).
pub struct SelectOptions {
    pub regex: Vec<String>,
    pub exact: Vec<String>,
    pub contains: Vec<String>,
    pub fields: Fields,
    pub case_sensitive: bool,
}

/// Whether `hmm` matches any criterion in `opts` on any searched field.
pub fn profile_matches(
    hmm: &Hmm,
    opts: &SelectOptions,
    regexes: &[regex::Regex],
) -> bool {
    let mut name = hmm.name.clone();
    let mut desc = hmm.desc.clone().unwrap_or_default();
    let mut acc = hmm.acc.clone().unwrap_or_default();
    if !opts.case_sensitive {
        name = name.to_lowercase();
        desc = desc.to_lowercase();
        acc = acc.to_lowercase();
    }

    // The field values to test (with the field enabled).
    let fields: [(bool, &str); 3] =
        [(opts.fields.name, &name), (opts.fields.desc, &desc), (opts.fields.acc, &acc)];

    // regex: RegexBuilder already folds case when !case_sensitive.
    for re in regexes {
        for (enabled, val) in &fields {
            if *enabled && re.is_match(val) {
                return true;
            }
        }
    }
    for target in &opts.exact {
        let target = if opts.case_sensitive { target.clone() } else { target.to_lowercase() };
        for (enabled, val) in &fields {
            if *enabled && **val == target {
                return true;
            }
        }
    }
    for needle in &opts.contains {
        let needle = if opts.case_sensitive { needle.clone() } else { needle.to_lowercase() };
        for (enabled, val) in &fields {
            if *enabled && val.contains(&needle) {
                return true;
            }
        }
    }
    false
}

/// Compile the `--regex` patterns (case-insensitive unless `case_sensitive`).
fn compile_regexes(opts: &SelectOptions) -> Result<Vec<regex::Regex>, String> {
    opts.regex
        .iter()
        .map(|p| {
            RegexBuilder::new(p)
                .case_insensitive(!opts.case_sensitive)
                .build()
                .map_err(|e| format!("invalid regex {p:?}: {e}"))
        })
        .collect()
}

/// Read HMM files, filter their profiles, and write the matches to `out`.
/// Returns the number of profiles written.
pub fn hmmer_select<W: Write>(
    input_files: &[String],
    opts: &SelectOptions,
    out: &mut W,
) -> Result<usize, String> {
    if !opts.fields.name && !opts.fields.desc && !opts.fields.acc {
        return Err("at least one of the searched fields must be enabled".into());
    }
    let regexes = compile_regexes(opts)?;
    let mut found = 0usize;
    for input_file in input_files {
        let hmms = hmmer_pure_rs::hmmfile::read_hmm_file(Path::new(input_file))
            .map_err(|e| format!("reading {input_file}: {e:?}"))?;
        for hmm in &hmms {
            if profile_matches(hmm, opts, &regexes) {
                hmmer_pure_rs::hmmfile::write_hmm(out, hmm)
                    .map_err(|e| format!("writing profile: {e:?}"))?;
                found += 1;
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmmer_pure_rs::alphabet::AlphabetType;

    fn hmm(name: &str, acc: Option<&str>, desc: Option<&str>) -> Hmm {
        let mut h = Hmm::new(1, AlphabetType::Amino, 20);
        h.name = name.to_string();
        h.acc = acc.map(str::to_string);
        h.desc = desc.map(str::to_string);
        h
    }

    fn opts(
        regex: &[&str],
        exact: &[&str],
        contains: &[&str],
        field: &str,
        cs: bool,
    ) -> SelectOptions {
        SelectOptions {
            regex: regex.iter().map(|s| s.to_string()).collect(),
            exact: exact.iter().map(|s| s.to_string()).collect(),
            contains: contains.iter().map(|s| s.to_string()).collect(),
            fields: Fields::from_field(field).unwrap(),
            case_sensitive: cs,
        }
    }

    fn matches(h: &Hmm, o: &SelectOptions) -> bool {
        let res = compile_regexes(o).unwrap();
        profile_matches(h, o, &res)
    }

    #[test]
    fn regex_all_fields() {
        let h = hmm("2-oxoacid_dh", Some("PF00198.25"), Some("2-oxoacid dehydrogenases"));
        assert!(matches(&h, &opts(&["dehyd.*"], &[], &[], "all", false)));
        assert!(!matches(&h, &opts(&["notmatching"], &[], &[], "all", false)));
    }

    #[test]
    fn exact_requires_full_match() {
        let h = hmm("TCAD9", None, None);
        assert!(matches(&h, &opts(&[], &["TCAD9"], &[], "name", false)));
        assert!(!matches(&h, &opts(&[], &["TCAD"], &[], "name", false)));
    }

    #[test]
    fn contains_substring() {
        let h = hmm("x", Some("PF19974.1"), None);
        assert!(matches(&h, &opts(&[], &[], &["PF19974"], "acc", false)));
    }

    #[test]
    fn case_insensitivity() {
        let h = hmm("x", Some("PF19974.1"), None);
        // insensitive: lowercase query matches
        assert!(matches(&h, &opts(&[], &[], &["pf19974"], "acc", false)));
        assert!(matches(&h, &opts(&["pf19974"], &[], &[], "acc", false)));
        // sensitive: no match
        assert!(!matches(&h, &opts(&[], &[], &["pf19974"], "acc", true)));
        assert!(!matches(&h, &opts(&["pf19974"], &[], &[], "acc", true)));
    }

    #[test]
    fn field_restriction() {
        let h = hmm("dehydrogenase", None, Some("something else"));
        // searching only acc (empty) should not match
        assert!(!matches(&h, &opts(&[], &[], &["dehyd"], "acc", false)));
        assert!(matches(&h, &opts(&[], &[], &["dehyd"], "name", false)));
    }
}
