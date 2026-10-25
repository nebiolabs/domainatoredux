//! `domainatoredux describe`: the tool manifest, in upstream's format (manifest
//! version 1, `docs/tool_manifest.md` upstream).
//!
//! Built from the tools' clap arguments plus [`crate::registry`], the way upstream
//! builds its manifest from its argparse parsers, so it cannot drift from the command
//! line. A cargo test diffs it against upstream's own `domainator describe`
//! (`scripts/parity/upstream_manifest.json`), which turns an interface difference into a
//! failing test.

use clap::{Arg, ArgAction, Command};
use serde_json::{json, Map, Value};

use crate::registry::{ToolInfo, CATEGORIES, TOOLS};
use crate::types::type_info;

pub const MANIFEST_VERSION: u32 = 1;

/// Every tool takes these, so `describe` leaves them out (upstream: `invocation`
/// says how to pass a config file).
const SKIPPED: &[&str] = &["help", "version", "config", "print_config"];

/// Inputs not named `*_file` (upstream `manifest.INPUT_PATHS`).
const INPUT_PATHS: &[&str] = &[
    "input",
    "reference",
    "metadata",
    "color_table",
    "coords",
    "foldseek",
    "esm2_3di_weights",
    "matrix",
    "ncbi_summary",
];
/// Directories not named `*_dir` (upstream `manifest.DIRECTORIES`).
const DIRECTORIES: &[&str] = &[
    "output_dir",
    "tmp_dir",
    "model_dir",
    "tmbed_model_dir",
    "datasets_workdir",
    "ncbi_taxonomy_path",
];

/// `input`, `output`, `directory` or `program` for an option that holds a path, by
/// upstream's naming rules: an option is an output exactly when its name ends in
/// `_out`.
pub fn path_role(name: &str) -> Option<&'static str> {
    if name.ends_with("_out") {
        Some("output")
    } else if DIRECTORIES.contains(&name) || name.ends_with("_dir") {
        Some("directory")
    } else if name.ends_with("_path") {
        Some("program")
    } else if INPUT_PATHS.contains(&name) || name.ends_with("_file") {
        Some("input")
    } else {
        None
    }
}

fn is_switch(arg: &Arg) -> bool {
    matches!(arg.get_action(), ArgAction::SetTrue | ArgAction::SetFalse)
}

/// The arguments `describe` and `--print_config` cover, in command-line order: every
/// option except `--help`, `--config` and `--print_config`, and the `--no_x` half of
/// a switch.
pub fn parameters(cmd: &Command) -> impl Iterator<Item = &Arg> {
    cmd.get_arguments().filter(move |a| {
        let id = a.get_id().as_str();
        !SKIPPED.contains(&id) && !a.is_positional() && negated(cmd, a).is_none()
    })
}

/// For the `--no_x` argument of a switch, the `--x` argument it turns off.
pub fn negated<'a>(cmd: &'a Command, arg: &Arg) -> Option<&'a Arg> {
    let base = arg.get_id().as_str().strip_prefix("no_")?;
    if !is_switch(arg) {
        return None;
    }
    cmd.get_arguments()
        .find(|a| a.get_id().as_str() == base && is_switch(a))
}

/// The `--no_x` argument that turns `arg` off, if it has one.
pub fn negation<'a>(cmd: &'a Command, arg: &Arg) -> Option<&'a Arg> {
    let id = format!("no_{}", arg.get_id().as_str());
    cmd.get_arguments()
        .find(|a| a.get_id().as_str() == id && is_switch(a))
}

pub fn flags(arg: &Arg) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(s) = arg.get_short() {
        out.push(format!("-{s}"));
    }
    if let Some(l) = arg.get_long() {
        out.push(format!("--{l}"));
    }
    out
}

/// Whether the option takes a list of values.
pub fn is_multiple(arg: &Arg) -> bool {
    matches!(arg.get_action(), ArgAction::Append)
        || arg.get_num_args().is_some_and(|r| r.max_values() > 1)
}

/// `value` as JSON of the option's declared type (`3` for an integer, not `"3"`).
pub fn typed(json_type: &str, value: &str) -> Value {
    let parsed = match json_type {
        "integer" => value.parse::<i64>().ok().map(Value::from),
        "number" => value
            .parse::<f64>()
            .ok()
            .and_then(|f| serde_json::Number::from_f64(f).map(Value::Number)),
        "boolean" => value.parse::<bool>().ok().map(Value::Bool),
        _ => None,
    };
    parsed.unwrap_or_else(|| Value::String(value.to_string()))
}

fn bound(json_type: &str, v: f64) -> Value {
    if json_type == "integer" {
        // Bounds are small whole numbers (0, 1); the cast cannot lose anything.
        #[allow(clippy::cast_possible_truncation)]
        Value::from(v as i64)
    } else {
        json!(v)
    }
}

fn help_text(arg: &Arg) -> String {
    arg.get_long_help()
        .or_else(|| arg.get_help())
        .map(|h| h.to_string())
        .unwrap_or_default()
}

/// One parameter object (upstream `describe_parameter`).
pub fn describe_parameter(cmd: &Command, arg: &Arg, info: &ToolInfo) -> Value {
    let name = arg.get_id().as_str();
    let mut p = Map::new();
    p.insert("name".into(), json!(name));
    p.insert("flags".into(), json!(flags(arg)));
    p.insert("help".into(), json!(help_text(arg)));
    if is_switch(arg) {
        let default =
            info.switches_on.contains(&name) || matches!(arg.get_action(), ArgAction::SetFalse);
        p.insert("kind".into(), json!("flag"));
        p.insert("type".into(), json!("boolean"));
        p.insert("default".into(), json!(default));
        p.insert(
            "sets".into(),
            json!(matches!(arg.get_action(), ArgAction::SetTrue)),
        );
        if let Some(neg) = negation(cmd, arg) {
            p.insert("negative_flags".into(), json!(flags(neg)));
        }
        return Value::Object(p);
    }
    let ty = type_info(arg.get_value_parser());
    let role = path_role(name);
    let columns = info.columns.filter(|_| name == "columns");
    let kind = match (columns, role) {
        (Some(_), _) => "columns",
        (None, Some(_)) => "path",
        (None, None) => "value",
    };
    p.insert("kind".into(), json!(kind));
    p.insert("type".into(), json!(ty.json_type));
    for (key, v) in [
        ("min", ty.min),
        ("max", ty.max),
        ("exclusive_min", ty.exclusive_min),
    ] {
        if let Some(v) = v {
            p.insert(key.into(), bound(ty.json_type, v));
        }
    }
    if let Some(role) = role {
        p.insert("role".into(), json!(role));
    }
    let multiple = is_multiple(arg);
    if multiple {
        p.insert("multiple".into(), json!(true));
    } else if let Some(n) = arg
        .get_num_args()
        .filter(|r| r.min_values() == r.max_values() && r.min_values() > 1)
    {
        p.insert("count".into(), json!(n.min_values()));
    }
    p.insert("required".into(), json!(arg.is_required_set()));
    if info.dynamic_defaults.contains(&name) {
        p.insert("default_dynamic".into(), json!(true));
    } else {
        let defaults: Vec<Value> = arg
            .get_default_values()
            .iter()
            .map(|v| typed(ty.json_type, &v.to_string_lossy()))
            .collect();
        let default = match (multiple, defaults.len()) {
            (_, 0) => Value::Null,
            (true, _) => Value::Array(defaults),
            (false, _) => defaults.into_iter().next().unwrap_or(Value::Null),
        };
        p.insert("default".into(), default);
    }
    let choices: Vec<Value> = arg
        .get_possible_values()
        .iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| typed(ty.json_type, v.get_name()))
        .collect();
    if !choices.is_empty() {
        p.insert("choices".into(), Value::Array(choices));
    }
    if let Some(catalog) = columns {
        // Upstream lists name, args and repeat_last; `help` is ours, for agents and
        // the GUI's column builder.
        let entries: Vec<Value> = catalog
            .iter()
            .map(|c| {
                json!({
                    "name": c.name,
                    "args": c.args,
                    "repeat_last": c.repeat_last,
                    "help": c.help,
                })
            })
            .collect();
        p.insert("columns".into(), Value::Array(entries));
    }
    if role == Some("input") && name == "input" {
        p.insert("stdin".into(), json!(info.stdin));
    }
    if role == Some("output") {
        p.insert("stdout".into(), json!(!info.no_stdout.contains(&name)));
    }
    Value::Object(p)
}

/// One tool object (upstream `describe_tool`).
pub fn describe_tool(cmd: &Command, info: &ToolInfo) -> Value {
    let parameters: Vec<Value> = parameters(cmd)
        .map(|a| describe_parameter(cmd, a, info))
        .collect();
    let exclusive: Vec<Value> = cmd
        .get_groups()
        .filter(|g| {
            let mut g = (*g).clone();
            !g.is_multiple()
        })
        .map(|g| json!(g.get_args().map(|a| a.as_str()).collect::<Vec<_>>()))
        .collect();
    let description = cmd
        .get_long_about()
        .or_else(|| cmd.get_about())
        .map(|s| s.to_string())
        .unwrap_or_default();
    json!({
        "name": info.name,
        "category": info.category,
        "summary": info.summary,
        "description": description,
        "parameters": parameters,
        "required_outputs": info.required_outputs,
        "mutually_exclusive": exclusive,
    })
}

/// The whole manifest for the subcommands of `root`, or only `only`'s.
pub fn manifest(root: &Command, only: Option<&[String]>) -> Value {
    let tools: Vec<Value> = TOOLS
        .iter()
        .filter(|t| t.in_manifest)
        .filter(|t| only.is_none_or(|o| o.iter().any(|n| n == t.name)))
        .filter_map(|t| root.find_subcommand(t.name).map(|c| describe_tool(c, t)))
        .collect();
    json!({
        "manifest_version": MANIFEST_VERSION,
        // The upstream release whose interface this one follows, so a program written
        // against upstream's manifest can check it; the port's own version beside it.
        "domainator_version": domainator_core::version::UPSTREAM_VERSION,
        "domainatoredux_version": domainator_core::version::VERSION,
        "invocation": {
            "command": ["domainatoredux", "{tool}"],
            "config_option": "--config",
            "config_keys": "parameter name",
            "stdio_path": "-",
        },
        "categories": CATEGORIES
            .iter()
            .map(|(name, title)| json!({"name": name, "title": title}))
            .collect::<Vec<_>>(),
        "tools": tools,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_follow_upstreams_naming_rules() {
        assert_eq!(path_role("genbank_out"), Some("output"));
        assert_eq!(path_role("domains_file"), Some("input"));
        assert_eq!(path_role("input"), Some("input"));
        assert_eq!(path_role("ncbi_taxonomy_path"), Some("directory"));
        assert_eq!(path_role("diamond_path"), Some("program"));
        assert_eq!(path_role("output_dir"), Some("directory"));
        assert_eq!(path_role("max_hits"), None);
    }

    #[test]
    fn a_switch_and_its_negation_are_one_parameter() {
        let cmd = Command::new("t")
            .arg(Arg::new("dedup").long("dedup").action(ArgAction::SetTrue))
            .arg(
                Arg::new("no_dedup")
                    .long("no_dedup")
                    .action(ArgAction::SetTrue),
            );
        let info = ToolInfo {
            switches_on: &["dedup"],
            ..TOOLS[0]
        };
        let ps: Vec<Value> = parameters(&cmd)
            .map(|a| describe_parameter(&cmd, a, &info))
            .collect();
        assert_eq!(ps.len(), 1);
        assert_eq!(ps[0]["negative_flags"], json!(["--no_dedup"]));
        assert_eq!(ps[0]["default"], json!(true));
    }
}
