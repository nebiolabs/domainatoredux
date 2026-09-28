//! The GUI's tool registry: one spec file per tool in `apps/domainator-gui/tools/`,
//! embedded at compile time by `build.rs` (which also refuses a file that does
//! not parse). See `tools/README.md` for the format.
//!
//! A spec lists **every** property of the tool's schema, whether or not the dialog
//! shows it, next to the CLI default. That is what lets a developer see and override
//! the defaults in one place, and it is why the tests below compare each spec
//! against `domainator_config::schema`. The schema is what the config struct
//! deserializes, and every config struct is `deny_unknown_fields`.

use std::sync::OnceLock;

pub use crate::tool_spec_types::{FieldSpec, ToolSpec};

include!(concat!(env!("OUT_DIR"), "/tool_specs.rs"));

/// Every spec, in file-name order.
pub fn specs() -> &'static [ToolSpec] {
    static SPECS: OnceLock<Vec<ToolSpec>> = OnceLock::new();
    SPECS.get_or_init(|| {
        TOOL_SPEC_FILES
            .iter()
            .map(|(name, text)| {
                // build.rs parsed the same text with the same types, so this
                // cannot fail on anything that compiled.
                serde_yaml::from_str(text).unwrap_or_else(|e| panic!("tools/{name}: {e}"))
            })
            .collect()
    })
}

impl FieldSpec {
    /// What the dialog seeds: the GUI override, else the CLI default.
    pub fn effective_default(&self) -> Option<&serde_json::Value> {
        self.gui_default.as_ref().or(self.cli_default.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd_tools::{ROLES, TOOL_CATEGORIES};
    use crate::tool_spec_types::Select;
    use serde_json::Value;

    fn schema_props(tool: &str) -> serde_json::Map<String, Value> {
        let text =
            domainator_config::schema(tool).unwrap_or_else(|e| panic!("{tool}: no schema: {e}"));
        let v: Value = serde_json::from_str(&text).unwrap();
        v["properties"].as_object().unwrap().clone()
    }

    /// Numbers compare by value: the schema writes `1000.0`, and a spec that says
    /// `1000` means the same default.
    fn same_value(a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
            (Value::Array(x), Value::Array(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same_value(a, b))
            }
            _ => a == b,
        }
    }

    /// The JSON types a schema property admits (`"type"` may be a string or a
    /// list). An `enum` without a `type` has none; membership is its check.
    fn prop_types(prop: &Value) -> Vec<String> {
        match &prop["type"] {
            Value::String(s) => vec![s.clone()],
            Value::Array(a) => a
                .iter()
                .filter_map(|t| t.as_str().map(str::to_string))
                .collect(),
            _ => Vec::new(),
        }
    }

    fn value_fits_type(v: &Value, t: &str) -> bool {
        match t {
            "string" => v.is_string(),
            "boolean" => v.is_boolean(),
            "integer" => v.as_f64().is_some_and(|n| n.fract() == 0.0),
            "number" => v.is_number(),
            "array" => v.is_array(),
            "object" => v.is_object(),
            "null" => v.is_null(),
            _ => false,
        }
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

    /// A `columns` property's `items.oneOf`: one entry per column type.
    fn column_variants(prop: &Value) -> Vec<Value> {
        prop["items"]["oneOf"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn variant_type(v: &Value) -> &str {
        v["properties"]["type"]["const"].as_str().unwrap_or("")
    }

    /// Whether `v` fits schema property `prop`: its enum when it has one, else its type.
    fn fits_prop(v: &Value, prop: &Value) -> bool {
        if let Some(options) = prop["enum"].as_array() {
            return options.iter().any(|o| same_value(o, v));
        }
        let types = prop_types(prop);
        types.is_empty() || types.iter().any(|t| value_fits_type(v, t))
    }

    /// Check one column object against its `oneOf` entry, the way the report's
    /// `deny_unknown_fields` column enum will read it; returns its type.
    fn check_column(at: &str, variants: &[Value], column: &Value) -> String {
        let ty = column["type"]
            .as_str()
            .unwrap_or_else(|| panic!("{at}: a column needs a `type`"));
        let variant = variants
            .iter()
            .find(|v| variant_type(v) == ty)
            .unwrap_or_else(|| panic!("{at}: unknown column type {ty:?}"));
        let props = variant["properties"].as_object().unwrap();
        for (k, val) in column.as_object().unwrap() {
            let prop = props
                .get(k)
                .unwrap_or_else(|| panic!("{at}: {ty} columns take no {k:?}"));
            if k != "type" {
                assert!(
                    fits_prop(val, prop),
                    "{at}: {ty}.{k} = {val} does not fit {prop}"
                );
            }
        }
        for r in variant["required"].as_array().unwrap() {
            let r = r.as_str().unwrap();
            assert!(column.get(r).is_some(), "{at}: a {ty} column needs {r:?}");
        }
        ty.to_string()
    }

    /// `column_types` relabels and seeds the builder the schema defines, so every
    /// key has to name something the schema has. A misspelled type or parameter
    /// would otherwise be ignored without a word.
    #[test]
    fn column_type_overrides_name_real_types_and_parameters() {
        for spec in specs() {
            let props = schema_props(&spec.id);
            for f in &spec.fields {
                let at = format!("{}.{}", spec.id, f.name);
                if f.role != "columns" {
                    assert!(
                        f.column_types.is_empty(),
                        "{at}: `column_types` only applies to the columns role"
                    );
                    continue;
                }
                let variants = column_variants(&props[&f.name]);
                assert!(
                    !variants.is_empty(),
                    "{at}: the schema's columns.items has no oneOf to build from"
                );
                for (ty, over) in &f.column_types {
                    let variant = variants
                        .iter()
                        .find(|v| variant_type(v) == ty)
                        .unwrap_or_else(|| panic!("{at}: column_types names unknown type {ty:?}"));
                    for (param, p) in &over.params {
                        let prop = variant["properties"]
                            .get(param)
                            .filter(|_| param != "type")
                            .unwrap_or_else(|| panic!("{at}.{ty}: no parameter {param:?}"));
                        if let Some(d) = &p.default {
                            assert!(
                                fits_prop(d, prop),
                                "{at}.{ty}.{param}: default {d} does not fit {prop}"
                            );
                        }
                        // A choice is one entry of a list parameter, or a whole value otherwise.
                        let one = if prop["type"] == "array" {
                            &prop["items"]
                        } else {
                            prop
                        };
                        for c in &p.choices {
                            assert!(
                                fits_prop(c, one),
                                "{at}.{ty}.{param}: choice {c} does not fit {one}"
                            );
                        }
                    }
                }
                assert!(
                    variants
                        .iter()
                        .any(|v| f.column_types.get(variant_type(v)).is_none_or(|o| o.show)),
                    "{at}: column_types hides every column type"
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
                TOOL_CATEGORIES.contains(&spec.category.as_str()),
                "{}: unknown category {:?}",
                spec.id,
                spec.category
            );
            ids.push(spec.id.clone());
        }
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(n, ids.len(), "duplicate tool id");
    }

    /// A schema property missing from the spec is a CLI option nobody decided
    /// about; a spec field the schema lacks is worse, because the config struct is
    /// `deny_unknown_fields` and would reject the run.
    #[test]
    fn every_schema_property_is_listed_once() {
        for spec in specs() {
            let props = schema_props(&spec.id);
            let mut names: Vec<&str> = spec.fields.iter().map(|f| f.name.as_str()).collect();
            for n in &names {
                assert!(
                    props.contains_key(*n),
                    "{}: field {n:?} is not a config property",
                    spec.id
                );
            }
            for p in props.keys() {
                assert!(
                    names.contains(&p.as_str()),
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
    fn cli_defaults_match_the_schema() {
        for spec in specs() {
            let props = schema_props(&spec.id);
            for f in &spec.fields {
                let schema_default = props[&f.name].get("default");
                match (&f.cli_default, schema_default) {
                    (None, None) => {}
                    (Some(a), Some(b)) => assert!(
                        same_value(a, b),
                        "{}.{}: cli_default is {a}, but the schema's default is {b}",
                        spec.id,
                        f.name
                    ),
                    (a, b) => panic!(
                        "{}.{}: cli_default is {a:?}, but the schema's default is {b:?}",
                        spec.id, f.name
                    ),
                }
            }
        }
    }

    #[test]
    fn gui_defaults_fit_their_property() {
        for spec in specs() {
            let props = schema_props(&spec.id);
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
                let prop = &props[&f.name];
                if let Some(options) = prop["enum"].as_array() {
                    assert!(
                        options.iter().any(|o| same_value(o, v)),
                        "{at}: {v} is not one of {options:?}"
                    );
                }
                let types = prop_types(prop);
                if !types.is_empty() {
                    assert!(
                        types.iter().any(|t| value_fits_type(v, t)),
                        "{at}: {v} is not a {types:?}"
                    );
                }
                if f.role == "columns" {
                    let variants = column_variants(prop);
                    for (i, column) in v.as_array().unwrap().iter().enumerate() {
                        let ty = check_column(&format!("{at}[{i}]"), &variants, column);
                        assert!(
                            f.column_types.get(&ty).is_none_or(|o| o.show),
                            "{at}[{i}]: starts with column type {ty:?}, which column_types hides"
                        );
                    }
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
                if matches!(f.role.as_str(), "input_file" | "input_files") {
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
                } else {
                    assert!(
                        f.sources.is_none() && f.select.is_none(),
                        "{at}: `sources`/`select` only apply to inputs"
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
                if let Some(h) = &f.help {
                    assert!(!h.trim().is_empty(), "{at}: an empty `help` hides the schema's description; delete the key instead");
                }
            }
        }
    }

    /// The seeded output name is the only thing between a user and a run, and the
    /// dialog substitutes nothing but `{stem}`.
    #[test]
    fn required_outputs_have_a_name_template() {
        for spec in specs() {
            for f in spec
                .fields
                .iter()
                .filter(|f| f.role == "output_file" && f.required)
            {
                let t = f
                    .output_name
                    .as_deref()
                    .unwrap_or_else(|| panic!("{}.{}: no output_name", spec.id, f.name));
                let rest = t.replace("{stem}", "");
                assert!(
                    !rest.contains('{') && !rest.contains('}'),
                    "{}.{}: {t:?} has a placeholder other than {{stem}}",
                    spec.id,
                    f.name
                );
                assert!(
                    !rest.contains('/') && !rest.contains('\\'),
                    "{}.{}: {t:?} is a path, not a file name",
                    spec.id,
                    f.name
                );
            }
        }
    }
}
