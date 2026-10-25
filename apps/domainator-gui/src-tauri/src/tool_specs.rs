//! The GUI's tool registry: one spec file per tool in `apps/domainator-gui/tools/`,
//! embedded at compile time by `build.rs` (which also refuses a file that does
//! not parse). See `tools/README.md` for the format.
//!
//! A spec is GUI policy over the tool's manifest entry
//! (`domainator_config::tool_manifest`, what `domainatoredux describe` prints): labels,
//! dialog order, which files an input offers, GUI defaults. It lists **every**
//! manifest parameter except `-v`/`-q` (which no dialog offers), shown or not, so a new CLI option fails a test
//! here until someone decides whether the dialog offers it. Types, choices, defaults,
//! roles and help come from the manifest, filled in when the specs load.

use std::sync::OnceLock;

use serde_json::Value;

pub use crate::tool_spec_types::{FieldSpec, ToolSpec};

include!(concat!(env!("OUT_DIR"), "/tool_specs.rs"));

/// Every spec, in file-name order, resolved against its tool's manifest.
pub fn specs() -> &'static [ToolSpec] {
    static SPECS: OnceLock<Vec<ToolSpec>> = OnceLock::new();
    SPECS.get_or_init(|| {
        TOOL_SPEC_FILES
            .iter()
            .map(|(name, text)| {
                // build.rs parsed the same text with the same types, so this
                // cannot fail on anything that compiled.
                let spec: ToolSpec =
                    serde_yaml::from_str(text).unwrap_or_else(|e| panic!("tools/{name}: {e}"));
                resolve(spec)
            })
            .collect()
    })
}

/// The manifest parameter `name` of a tool's manifest entry.
pub fn manifest_param<'a>(manifest: &'a Value, name: &str) -> Option<&'a Value> {
    manifest["parameters"]
        .as_array()?
        .iter()
        .find(|p| p["name"] == name)
}

/// The dialog role a manifest parameter implies: a report's column builder, an input
/// dropdown (a list of them for a multiple input), an output name, or a plain
/// value. A directory is an input only when the spec says which files it offers
/// (the taxonomy database); a program path is a plain value, and never shown.
pub fn derived_role(param: &Value, has_file_type: bool) -> &'static str {
    match (param["kind"].as_str(), param["role"].as_str()) {
        (Some("columns"), _) => "columns",
        (_, Some("input")) if param["multiple"] == true => "input_files",
        (_, Some("input")) => "input_file",
        (_, Some("output")) => "output_file",
        (_, Some("directory")) if has_file_type => "input_file",
        _ => "param",
    }
}

fn resolve(mut spec: ToolSpec) -> ToolSpec {
    // A spec for a tool the manifest does not know has no parameters to resolve
    // against; `one_spec_file_per_tool_named_after_it` names the file.
    spec.manifest = domainator_config::tool_manifest(&spec.id).unwrap_or(Value::Null);
    spec.category = spec.manifest["category"]
        .as_str()
        .and_then(crate::cmd_tools::category_label)
        .unwrap_or_default()
        .to_string();
    for f in &mut spec.fields {
        f.param = manifest_param(&spec.manifest, &f.name)
            .cloned()
            .unwrap_or(Value::Null);
        f.role = derived_role(&f.param, !f.file_type.is_empty()).to_string();
        f.required |= f.param["required"] == true;
    }
    spec
}

impl FieldSpec {
    /// What the dialog seeds: the GUI override, else the CLI default.
    pub fn effective_default(&self) -> Option<&Value> {
        self.gui_default
            .as_ref()
            .or(self.param.get("default").filter(|d| !d.is_null()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd_tools::ROLES;
    use crate::tool_spec_types::Select;
    use domainator_config::columns::{parse_column, ColumnDef};
    use domainator_config::registry::tool_info;

    /// Parameters every tool has and no spec lists (`-v`/`-q`): a job's log level is
    /// the app's business.
    const COMMON_PARAMETERS: [&str; 2] = ["verbose", "quiet"];

    fn catalog(tool: &str) -> &'static [ColumnDef] {
        tool_info(tool)
            .and_then(|t| t.columns)
            .unwrap_or_else(|| panic!("{tool} has no column catalog"))
    }

    /// Whether `v` is a value of manifest parameter `param`: its type, one of its
    /// choices, within its bounds; a list of such for a multiple parameter.
    fn fits_param(v: &Value, param: &Value) -> Result<(), String> {
        if param["multiple"] == true {
            let Some(items) = v.as_array() else {
                return Err(format!("{v} is not a list"));
            };
            return items.iter().try_for_each(|i| fits_one(i, param));
        }
        fits_one(v, param)
    }

    fn fits_one(v: &Value, param: &Value) -> Result<(), String> {
        let ty = param["type"].as_str().unwrap_or("string");
        let ok = match ty {
            "boolean" => v.is_boolean(),
            "integer" => v.as_f64().is_some_and(|n| n.fract() == 0.0),
            "number" => v.is_number(),
            _ => v.is_string(),
        };
        if !ok {
            return Err(format!("{v} is not a {ty}"));
        }
        if let Some(choices) = param["choices"].as_array() {
            if !choices.contains(v) {
                return Err(format!("{v} is not one of {choices:?}"));
            }
        }
        let n = v.as_f64();
        let below = |key: &str, strict: bool| {
            param[key]
                .as_f64()
                .zip(n)
                .is_some_and(|(b, n)| if strict { n <= b } else { n < b })
        };
        let above = param["max"].as_f64().zip(n).is_some_and(|(b, n)| n > b);
        if below("min", false) || below("exclusive_min", true) || above {
            return Err(format!("{v} is out of bounds"));
        }
        Ok(())
    }

    /// Every `file_type` a spec may name: the types `workspace::classify_file`
    /// gives files, `taxonomy` (a registry format only: an NCBI taxdump), and the
    /// two groups `toolInputs.ts::typeMatches` expands.
    const FILE_TYPES: [&str; 13] = [
        "genbank",
        "fasta",
        "hmm",
        "hdf5",
        "cm",
        "stockholm",
        "table",
        "text",
        "svg",
        "ssn_viewer",
        "taxonomy",
        "sequence",
        "reference",
    ];

    /// `column_types` relabels and seeds the builder the manifest defines, so every
    /// key has to name a real column and argument. A misspelled one would otherwise
    /// be ignored without a word.
    #[test]
    fn column_type_overrides_name_real_columns_and_arguments() {
        for spec in specs() {
            for f in &spec.fields {
                let at = format!("{}.{}", spec.id, f.name);
                if f.role != "columns" {
                    assert!(
                        f.column_types.is_empty(),
                        "{at}: `column_types` only applies to the columns role"
                    );
                    continue;
                }
                let catalog = catalog(&spec.id);
                for (name, over) in &f.column_types {
                    let def = catalog.iter().find(|c| c.name == name).unwrap_or_else(|| {
                        panic!("{at}: column_types names unknown column {name:?}")
                    });
                    for (arg, p) in &over.params {
                        let last = def.args.last() == Some(&arg.as_str());
                        assert!(
                            def.args.contains(&arg.as_str()),
                            "{at}.{name}: no argument {arg:?} (it has {:?})",
                            def.args
                        );
                        // An argument is text; the repeating last one is a list of it.
                        let fits = |v: &Value| {
                            v.is_string()
                                || (def.repeat_last
                                    && last
                                    && v.as_array().is_some_and(|a| a.iter().all(Value::is_string)))
                        };
                        if let Some(d) = &p.default {
                            assert!(fits(d), "{at}.{name}.{arg}: default {d} is not text");
                        }
                        for c in &p.choices {
                            assert!(c.is_string(), "{at}.{name}.{arg}: choice {c} is not text");
                        }
                    }
                }
                assert!(
                    catalog
                        .iter()
                        .any(|c| f.column_types.get(c.name).is_none_or(|o| o.show)),
                    "{at}: column_types hides every column"
                );
            }
        }
    }

    /// `toolInputs.ts::typeMatches`: whether a database of `format` is offered in a
    /// field declared as `file_type`.
    fn format_fits(format: &str, file_type: &str) -> bool {
        match file_type {
            "sequence" => matches!(format, "genbank" | "fasta"),
            "reference" => matches!(format, "hmm" | "fasta"),
            ft => ft == format,
        }
    }

    #[test]
    fn one_spec_file_per_tool_named_after_it() {
        let mut ids = Vec::new();
        for ((file, _), spec) in TOOL_SPEC_FILES.iter().zip(specs()) {
            assert_eq!(
                *file,
                format!("{}.yml", spec.id),
                "tools/{file} declares id {:?}",
                spec.id
            );
            assert!(
                tool_info(&spec.id).is_some_and(|t| t.in_manifest),
                "tools/{file}: {} is not a tool in `domainatoredux describe`",
                spec.id
            );
            assert!(
                !spec.category.is_empty(),
                "{}: its manifest category {} has no entry in cmd_tools::CATEGORY_LABELS",
                spec.id,
                spec.manifest["category"]
            );
            ids.push(spec.id.clone());
        }
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(n, ids.len(), "duplicate tool id");
    }

    /// A manifest parameter missing from the spec is a CLI option nobody decided
    /// about; a spec field the manifest lacks is a value the CLI would reject.
    #[test]
    fn every_manifest_parameter_is_listed_once() {
        for spec in specs() {
            let params: Vec<&str> = spec.manifest["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|p| p["name"].as_str())
                .filter(|n| !COMMON_PARAMETERS.contains(n))
                .collect();
            let mut names: Vec<&str> = spec.fields.iter().map(|f| f.name.as_str()).collect();
            for n in &names {
                assert!(
                    params.contains(n),
                    "{}: field {n:?} is not a parameter of `domainatoredux describe --tool {}`",
                    spec.id,
                    spec.id
                );
            }
            for p in &params {
                assert!(
                    names.contains(p),
                    "{}: CLI option {p:?} is not listed in tools/{}.yml (add it, with `show: false` if the dialog should not offer it)",
                    spec.id,
                    spec.id
                );
            }
            let n = names.len();
            names.sort_unstable();
            names.dedup();
            assert_eq!(n, names.len(), "{}: a field is listed twice", spec.id);
        }
    }

    #[test]
    fn gui_defaults_fit_their_property() {
        for spec in specs() {
            for f in spec.fields.iter().filter(|f| f.gui_default.is_some()) {
                let v = f.gui_default.as_ref().unwrap();
                let at = format!("{}.{}", spec.id, f.name);
                // A hidden field is never sent, so its override would do nothing.
                assert!(f.show, "{at}: gui_default on a hidden field has no effect");
                assert!(
                    matches!(f.role.as_str(), "param" | "columns"),
                    "{at}: gui_default on a {} field (files are chosen with `select`/`output_name`)",
                    f.role
                );
                if f.role == "columns" {
                    let catalog = catalog(&spec.id);
                    for (i, token) in v.as_array().unwrap().iter().enumerate() {
                        let token = token
                            .as_str()
                            .unwrap_or_else(|| panic!("{at}[{i}]: a column is a token string"));
                        let c = parse_column(token, catalog)
                            .unwrap_or_else(|e| panic!("{at}[{i}]: {e}"));
                        assert!(
                            f.column_types.get(&c.name).is_none_or(|o| o.show),
                            "{at}[{i}]: starts with column {}, which column_types hides",
                            c.name
                        );
                    }
                } else if let Err(e) = fits_param(v, &f.param) {
                    panic!("{at}: gui_default {e}");
                }
            }
        }
    }

    #[test]
    fn input_fields_declare_sources_and_select() {
        let dbs = crate::reference_dbs::specs();
        for spec in specs() {
            for f in &spec.fields {
                let at = format!("{}.{}", spec.id, f.name);
                assert!(
                    ROLES.contains(&f.role.as_str()),
                    "{at}: ToolDialog cannot render role {:?}",
                    f.role
                );
                let input = matches!(f.role.as_str(), "input_file" | "input_files");
                if input && f.show {
                    assert!(f.sources.is_some(), "{at}: an input needs `sources`");
                    assert!(f.select.is_some(), "{at}: an input needs `select`");
                    assert!(!f.file_type.is_empty(), "{at}: an input needs `file_type`");
                    for ft in &f.file_type {
                        // Anything else matches no file, so the dropdown is always empty.
                        assert!(
                            FILE_TYPES.contains(&ft.as_str()),
                            "{at}: unknown file_type {ft:?}; expected one of {FILE_TYPES:?}"
                        );
                    }
                    if let Some(Select::Database(id)) = &f.select {
                        let db = dbs.iter().find(|d| d.id == id.as_str()).unwrap_or_else(|| {
                            panic!("{at}: select names unknown reference database {id:?}")
                        });
                        let format = db.kind.as_str();
                        assert!(
                            f.file_type.iter().any(|ft| format_fits(format, ft)),
                            "{at}: database {id:?} ({format}) never appears in a {:?} dropdown",
                            f.file_type
                        );
                    }
                } else if !input {
                    assert!(
                        f.sources.is_none() && f.select.is_none() && f.prefill_priority.is_none(),
                        "{at}: `sources`/`select`/`prefill_priority` only apply to inputs"
                    );
                    assert!(
                        f.file_type.is_empty(),
                        "{at}: `file_type` only applies to inputs"
                    );
                }
                if f.output_name.is_some() {
                    assert_eq!(
                        f.role, "output_file",
                        "{at}: `output_name` only applies to outputs"
                    );
                }
                if !f.show {
                    assert!(!f.required, "{at}: a required field cannot be hidden");
                    assert!(
                        f.help.is_none(),
                        "{at}: `help` on a hidden field is never shown"
                    );
                }
                // A program path is the External tools panel's business.
                assert!(
                    !(f.show && f.param["role"] == "program"),
                    "{at}: a program path is never shown"
                );
                if let Some(h) = &f.help {
                    assert!(
                        !h.trim().is_empty(),
                        "{at}: an empty `help` hides the manifest's; delete the key instead"
                    );
                }
            }
        }
    }

    fn check_name_template(at: &str, t: &str) {
        let rest = t.replace("{stem}", "");
        assert!(
            !rest.contains('{') && !rest.contains('}'),
            "{at}: {t:?} has a placeholder other than {{stem}}"
        );
        assert!(
            !rest.contains('/') && !rest.contains('\\'),
            "{at}: {t:?} is a path, not a file name"
        );
    }

    /// The seeded output name is the only thing between a user and a run, and the
    /// dialog substitutes nothing but `{stem}`. A tool that needs one of several
    /// outputs (the manifest's `required_outputs`) seeds at least one.
    #[test]
    fn required_outputs_have_a_name_template() {
        for spec in specs() {
            for f in spec.fields.iter().filter(|f| f.role == "output_file") {
                let at = format!("{}.{}", spec.id, f.name);
                match f.output_name.as_deref() {
                    Some(t) => check_name_template(&at, t),
                    None => assert!(!f.required, "{at}: a required output needs an output_name"),
                }
            }
            if let Some(any_of) = spec.manifest["required_outputs"].as_array() {
                assert!(
                    spec.fields.iter().any(|f| f.show
                        && f.output_name.is_some()
                        && any_of.iter().any(|o| *o == f.name)),
                    "{}: a run needs one of {any_of:?}; give one a shown field with an output_name",
                    spec.id
                );
            }
        }
    }
}
