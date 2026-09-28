//! `hmmer_select`: filter an HMM file by text matches on profile NAME / DESC /
//! ACC fields. A port of `domainator.hmmer_select`.
//!
//! Matching modes (`--regex`, `--exact`, `--contains`) are combined with OR: a
//! profile is kept if it matches ANY specification on ANY searched field. Output
//! is an HMM file written via `search::hmm_write`, which reproduces
//! `p7_hmmfile_WriteASCII` — so selecting profiles out of a file and writing them
//! back is **byte-identical** to what upstream produces, cutoff lines included.
//! (The `hmmer-pure-rs` writer is not: it stamps a `HMMER3/e` header and silently
//! drops `GA`/`TC`/`NC`.) Read through `search::hmm_read` for the same reason.

use std::io::Write;
use std::path::Path;

use hmmer_pure_rs::alphabet::AlphabetType;
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
            "name" => Fields {
                name: true,
                desc: false,
                acc: false,
            },
            "desc" => Fields {
                name: false,
                desc: true,
                acc: false,
            },
            "acc" => Fields {
                name: false,
                desc: false,
                acc: true,
            },
            "all" => Fields {
                name: true,
                desc: true,
                acc: true,
            },
            other => {
                return Err(format!(
                    "--field must be name, desc, acc, or all, got {other:?}"
                ))
            }
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
    /// `--alphabet`: keep only input files whose profiles use this alphabet.
    /// Unlike the text criteria this is a *restriction*, not another OR term.
    pub alphabet: Option<AlphabetType>,
}

/// Parse an `--alphabet` value. `None` means "infer from the input".
pub fn parse_alphabet(name: Option<&str>) -> Result<Option<AlphabetType>, String> {
    match name.map(str::to_ascii_lowercase).as_deref() {
        None => Ok(None),
        Some("amino") => Ok(Some(AlphabetType::Amino)),
        Some("dna") => Ok(Some(AlphabetType::Dna)),
        Some("rna") => Ok(Some(AlphabetType::Rna)),
        Some(other) => Err(format!(
            "Unknown alphabet: {other}. Must be one of: amino, dna, rna."
        )),
    }
}

/// Display name of an alphabet, matching pyhmmer's `Alphabet.type`.
pub fn alphabet_name(alphabet: AlphabetType) -> &'static str {
    match alphabet {
        AlphabetType::Amino => "amino",
        AlphabetType::Dna => "DNA",
        AlphabetType::Rna => "RNA",
        AlphabetType::Unknown => "unknown",
    }
}

/// Whether `hmm` matches any criterion in `opts` on any searched field.
pub fn profile_matches(hmm: &Hmm, opts: &SelectOptions, regexes: &[regex::Regex]) -> bool {
    let mut name = hmm.name.clone();
    let mut desc = hmm.desc.clone().unwrap_or_default();
    let mut acc = hmm.acc.clone().unwrap_or_default();
    if !opts.case_sensitive {
        name = name.to_lowercase();
        desc = desc.to_lowercase();
        acc = acc.to_lowercase();
    }

    // The field values to test (with the field enabled).
    let fields: [(bool, &str); 3] = [
        (opts.fields.name, &name),
        (opts.fields.desc, &desc),
        (opts.fields.acc, &acc),
    ];

    // regex: RegexBuilder already folds case when !case_sensitive.
    for re in regexes {
        for (enabled, val) in &fields {
            if *enabled && re.is_match(val) {
                return true;
            }
        }
    }
    for target in &opts.exact {
        let target = if opts.case_sensitive {
            target.clone()
        } else {
            target.to_lowercase()
        };
        for (enabled, val) in &fields {
            if *enabled && **val == target {
                return true;
            }
        }
    }
    for needle in &opts.contains {
        let needle = if opts.case_sensitive {
            needle.clone()
        } else {
            needle.to_lowercase()
        };
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

/// How a run of [`hmmer_select`] turned out.
pub struct SelectSummary {
    pub found: usize,
    /// Input files skipped because their alphabet did not match `--alphabet`.
    pub skipped_files: usize,
}

/// Read HMM files, filter their profiles, and write the matches to `out`.
///
/// HMMER locks an `.hmm` file to the alphabet of its first profile, so every
/// profile written to one output must share one — an input set that mixes them is
/// an error rather than a file that cannot be read past its first profile. The
/// alphabet of a file is the alphabet of its first profile, so `--alphabet`
/// selects whole *files*, which is how a mixed set gets split one alphabet at a
/// time.
pub fn hmmer_select<W: Write>(
    input_files: &[String],
    opts: &SelectOptions,
    out: &mut W,
) -> Result<SelectSummary, String> {
    if !opts.fields.name && !opts.fields.desc && !opts.fields.acc {
        return Err("at least one of the searched fields must be enabled".into());
    }
    let regexes = compile_regexes(opts)?;
    // `--alphabet` on its own leaves the text criteria nothing to match, so it
    // means "every profile in the files that survive". With no criteria at all,
    // keep the historical behavior of selecting nothing.
    let match_all = regexes.is_empty()
        && opts.exact.is_empty()
        && opts.contains.is_empty()
        && opts.alphabet.is_some();

    let mut summary = SelectSummary {
        found: 0,
        skipped_files: 0,
    };
    let mut selected = opts.alphabet;
    let mut selected_source = opts.alphabet.map(|_| "--alphabet".to_string());
    for input_file in input_files {
        let hmms = crate::search::hmm_read::read_hmm_file(Path::new(input_file))
            .map_err(|e| format!("reading {input_file}: {e:?}"))?;
        let Some(file_alphabet) = hmms.first().map(|h| h.abc_type) else {
            continue; // no profiles in this file
        };
        if let Some(requested) = opts.alphabet {
            if file_alphabet != requested {
                eprintln!(
                    "Skipping '{input_file}': its profiles are {}, not the requested {}.",
                    alphabet_name(file_alphabet),
                    alphabet_name(requested)
                );
                summary.skipped_files += 1;
                continue;
            }
        }
        match selected {
            None => {
                selected = Some(file_alphabet);
                selected_source = Some(input_file.clone());
            }
            Some(chosen) if chosen != file_alphabet => {
                return Err(format!(
                    "The input hmm files use different alphabets: '{}' is {}, but '{input_file}' \
                     is {}. HMMER locks an hmm file to the alphabet of its first profile, so \
                     writing both into one output file would produce a file that cannot be read \
                     past its first profile. Pass --alphabet to keep only one alphabet, or run \
                     hmmer-select once per alphabet.",
                    selected_source.as_deref().unwrap_or("--alphabet"),
                    alphabet_name(chosen),
                    alphabet_name(file_alphabet),
                ));
            }
            Some(_) => {}
        }
        for hmm in &hmms {
            if match_all || profile_matches(hmm, opts, &regexes) {
                crate::search::hmm_write::write_hmm(out, hmm)
                    .map_err(|e| format!("writing profile: {e:?}"))?;
                summary.found += 1;
            }
        }
    }
    Ok(summary)
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
            alphabet: None,
        }
    }

    fn matches(h: &Hmm, o: &SelectOptions) -> bool {
        let res = compile_regexes(o).unwrap();
        profile_matches(h, o, &res)
    }

    /// Write `hmms` to a temp `.hmm` and return its path (kept alive by `dir`).
    fn hmm_file(dir: &Path, stem: &str, hmms: &[Hmm]) -> String {
        let path = dir.join(format!("{stem}.hmm"));
        let mut out = std::fs::File::create(&path).unwrap();
        for h in hmms {
            crate::search::hmm_write::write_hmm(&mut out, h).unwrap();
        }
        path.to_string_lossy().into_owned()
    }

    fn typed(name: &str, abc: AlphabetType, k: usize) -> Hmm {
        let mut h = Hmm::new(1, abc, k);
        h.name = name.to_string();
        h.mat[1] = vec![1.0 / k as f32; k];
        h.ins[1] = vec![1.0 / k as f32; k];
        h.ins[0] = vec![1.0 / k as f32; k];
        h.t[0] = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        h.t[1] = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0];
        h
    }

    fn select(inputs: &[String], o: &SelectOptions) -> Result<(usize, usize), String> {
        let mut out = Vec::new();
        hmmer_select(inputs, o, &mut out).map(|s| (s.found, s.skipped_files))
    }

    fn scratch() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("domainator-hmmsel-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn alphabet_selects_whole_files_and_implies_match_all() {
        let dir = scratch();
        let aa = hmm_file(&dir, "aa", &[typed("aa_1", AlphabetType::Amino, 20)]);
        let dna = hmm_file(
            &dir,
            "dna",
            &[
                typed("dna_1", AlphabetType::Dna, 4),
                typed("dna_2", AlphabetType::Dna, 4),
            ],
        );

        // With no text criteria, --alphabet means "everything in the files that
        // survive" -- otherwise there would be nothing for it to select.
        let mut o = opts(&[], &[], &[], "all", false);
        o.alphabet = Some(AlphabetType::Dna);
        assert_eq!(select(&[aa.clone(), dna.clone()], &o).unwrap(), (2, 1));

        // No criteria *and* no alphabet keeps the historical "select nothing".
        let o = opts(&[], &[], &[], "all", false);
        assert_eq!(select(std::slice::from_ref(&dna), &o).unwrap(), (0, 0));

        // With criteria it narrows rather than replaces them.
        let mut o = opts(&[], &[], &["_1"], "name", false);
        o.alphabet = Some(AlphabetType::Dna);
        assert_eq!(select(&[aa.clone(), dna.clone()], &o).unwrap(), (1, 1));
    }

    #[test]
    fn mixing_alphabets_without_the_flag_is_an_error() {
        // HMMER locks a file to the alphabet of its first profile, so the merged
        // output would be unreadable past that profile -- refuse rather than write it.
        let dir = scratch();
        let aa = hmm_file(&dir, "aa2", &[typed("aa_1", AlphabetType::Amino, 20)]);
        let dna = hmm_file(&dir, "dna2", &[typed("dna_1", AlphabetType::Dna, 4)]);
        let o = opts(&[], &[], &["_1"], "name", false);
        let err = select(&[dna, aa], &o).unwrap_err();
        assert!(err.contains("different alphabets"), "{err}");
    }

    #[test]
    fn alphabet_names_and_parsing() {
        assert_eq!(parse_alphabet(None).unwrap(), None);
        assert_eq!(
            parse_alphabet(Some("DNA")).unwrap(),
            Some(AlphabetType::Dna)
        );
        assert_eq!(
            parse_alphabet(Some("rna")).unwrap(),
            Some(AlphabetType::Rna)
        );
        assert!(parse_alphabet(Some("protein"))
            .unwrap_err()
            .contains("amino, dna, rna"));
        // pyhmmer's Alphabet.type spelling, which the skip message quotes.
        assert_eq!(alphabet_name(AlphabetType::Amino), "amino");
        assert_eq!(alphabet_name(AlphabetType::Dna), "DNA");
    }

    #[test]
    fn regex_all_fields() {
        let h = hmm(
            "2-oxoacid_dh",
            Some("PF00198.25"),
            Some("2-oxoacid dehydrogenases"),
        );
        assert!(matches(&h, &opts(&["dehyd.*"], &[], &[], "all", false)));
        assert!(!matches(
            &h,
            &opts(&["notmatching"], &[], &[], "all", false)
        ));
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
