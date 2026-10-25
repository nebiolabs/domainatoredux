//! The shape of a GUI tool spec file (`apps/domainator-gui/tools/<id>.yml`).
//!
//! Shared with `build.rs` through `#[path]`, which parses every spec file before
//! embedding it, so a misspelled key or a malformed file fails the build rather
//! than the dialog. That is also why this module depends on nothing but serde.
//! A spec is GUI policy layered over the tool's manifest entry (`domainatoredux
//! describe`): the fields marked "resolved" are filled from the manifest when the
//! specs load (`tool_specs::specs`), and the checks that need the manifest (every
//! parameter listed, defaults that fit, ...) are tests in `tool_specs.rs`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolSpec {
    pub id: String,
    pub label: String,
    /// Resolved: the Tools-menu group, `cmd_tools::CATEGORY_LABELS`' label for the
    /// tool's manifest category.
    #[serde(skip)]
    pub category: String,
    pub description: String,
    /// Resolved: the tool's manifest entry.
    #[serde(skip)]
    pub manifest: Value,
    /// Every parameter of the tool's manifest entry except `-v`/`-q`, in dialog order.
    pub fields: Vec<FieldSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    pub name: String,
    pub label: String,
    /// The hint under the field. Unset, the dialog shows the parameter's manifest
    /// `help` -- the same text `domainatoredux describe` gives agents.
    #[serde(default)]
    pub help: Option<String>,
    /// The control the dialog draws, one of `cmd_tools::ROLES`, derived from the
    /// manifest parameter (`tool_specs::derived_role`).
    #[serde(skip)]
    pub role: String,
    /// Inputs only: the file types the dropdown offers. Written as one value
    /// (`file_type: hmm`) or a list (`file_type: [sequence, hmm]`).
    #[serde(default, deserialize_with = "one_or_many")]
    pub file_type: Vec<String>,
    /// Marks the field required in the dialog. Resolved: also true when the manifest
    /// says so (a spec can tighten, never loosen).
    #[serde(default)]
    pub required: bool,
    /// `false` for a CLI option the dialog does not offer; it is never sent, so
    /// the CLI default applies.
    #[serde(default = "shown")]
    pub show: bool,
    /// What the dialog seeds instead of the manifest `default`.
    #[serde(default)]
    pub gui_default: Option<Value>,
    /// Input roles only: which group of the dropdown comes first.
    #[serde(default)]
    pub sources: Option<Sources>,
    /// Input roles only: what the dropdown starts on when no selected file was
    /// placed in it.
    #[serde(default)]
    pub select: Option<Select>,
    /// Input roles only: selected files fill inputs in descending priority, ties
    /// in spec order. Unset is 0.
    #[serde(default)]
    pub prefill_priority: Option<i32>,
    /// Output roles only: the seeded file name, `{stem}` being the primary
    /// input's name without its final extension.
    #[serde(default)]
    pub output_name: Option<String>,
    /// `columns` role only: GUI overrides for the column builder, keyed by column
    /// name. The columns and their arguments come from the manifest parameter's
    /// `columns`; this only relabels, hides, and seeds them.
    #[serde(default)]
    pub column_types: BTreeMap<String, ColumnTypeSpec>,
    /// Resolved: the manifest parameter.
    #[serde(skip)]
    pub param: Value,
}

/// The builder's view of one column (a manifest `columns` entry).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnTypeSpec {
    /// Replaces the column's name in the dropdown.
    #[serde(default)]
    pub label: Option<String>,
    /// Replaces the column's `help` as the row's hint.
    #[serde(default)]
    pub help: Option<String>,
    /// `false` leaves the type out of the dropdown.
    #[serde(default = "shown")]
    pub show: bool,
    #[serde(default)]
    pub params: BTreeMap<String, ColumnParamSpec>,
}

/// The builder's view of one argument of a column.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ColumnParamSpec {
    /// The text beside the box (default: the argument name).
    #[serde(default)]
    pub label: Option<String>,
    /// The box's tooltip and placeholder.
    #[serde(default)]
    pub help: Option<String>,
    /// What a newly added column of this type starts with.
    #[serde(default)]
    pub default: Option<Value>,
    /// Suggested values: one-click chips for a list parameter, a suggestion list
    /// for a text one. Values outside them can still be typed.
    #[serde(default)]
    pub choices: Vec<Value>,
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

fn shown() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sources {
    DatabasesFirst,
    WorkspaceFirst,
}

impl Sources {
    pub fn as_str(self) -> &'static str {
        match self {
            Sources::DatabasesFirst => "databases_first",
            Sources::WorkspaceFirst => "workspace_first",
        }
    }
}

/// An input dropdown's initial choice when no selected file was placed in it.
/// A `Database` that is not on offer falls back to the first entry.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum Select {
    First,
    None,
    /// The managed reference database with this id (`reference_dbs`).
    Database(String),
}

impl TryFrom<String> for Select {
    type Error = String;
    fn try_from(s: String) -> Result<Self, String> {
        Ok(match s.as_str() {
            "first" => Select::First,
            "none" => Select::None,
            _ => match s.strip_prefix("database:") {
                Some(id) if !id.is_empty() => Select::Database(id.to_string()),
                _ => {
                    return Err(format!(
                        "select {s:?}: expected first, none or database:<id>"
                    ))
                }
            },
        })
    }
}

impl std::fmt::Display for Select {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Select::First => f.write_str("first"),
            Select::None => f.write_str("none"),
            Select::Database(id) => write!(f, "database:{id}"),
        }
    }
}
