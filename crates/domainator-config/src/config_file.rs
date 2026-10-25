//! `--config FILE` and `--print_config`, in upstream's format.
//!
//! A config file is a JSON or YAML object keyed by parameter name, the shape upstream's
//! `--print_config` writes and its manifest documents (`config_keys: parameter name`), so
//! a file works with either implementation. Rather than deserializing it into a struct
//! of its own, it is turned into command-line arguments and parsed with everything else:
//! one parser, so a bound, a choice or a type check applies to a config file exactly as
//! it does on the command line. An option given on the command line wins over the file.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};

use anyhow::{bail, Context, Result};
use clap::{Arg, ArgAction, ArgMatches, Command};
use serde_json::{Map, Value};

use crate::manifest::{is_multiple, negated, negation, parameters, typed};
use crate::registry::ToolInfo;
use crate::types::type_info;

/// A config value, from either a YAML or a JSON document.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    Null,
    Bool(bool),
    Text(String),
    List(Vec<ConfigValue>),
    /// A mapping, which no option takes.
    Mapping,
}

impl From<&Value> for ConfigValue {
    fn from(v: &Value) -> Self {
        match v {
            Value::Null => ConfigValue::Null,
            Value::Bool(b) => ConfigValue::Bool(*b),
            Value::Number(n) => ConfigValue::Text(n.to_string()),
            Value::String(s) => ConfigValue::Text(s.clone()),
            Value::Array(a) => ConfigValue::List(a.iter().map(ConfigValue::from).collect()),
            Value::Object(_) => ConfigValue::Mapping,
        }
    }
}

impl From<&serde_yaml::Value> for ConfigValue {
    fn from(v: &serde_yaml::Value) -> Self {
        use serde_yaml::Value as Y;
        match v {
            Y::Null => ConfigValue::Null,
            Y::Bool(b) => ConfigValue::Bool(*b),
            // `.nan` and `.inf` come through as text the number parsers reject.
            Y::Number(n) => ConfigValue::Text(n.to_string()),
            Y::String(s) => ConfigValue::Text(s.clone()),
            Y::Sequence(s) => ConfigValue::List(s.iter().map(ConfigValue::from).collect()),
            Y::Mapping(_) => ConfigValue::Mapping,
            Y::Tagged(t) => ConfigValue::from(&t.value),
        }
    }
}

/// Read a config file: `(key, value)` pairs in file order. YAML is a superset of
/// JSON, so one parser reads both.
pub fn read(path: &str) -> Result<Vec<(String, ConfigValue)>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading config {path}"))?;
    let doc: serde_yaml::Value =
        serde_yaml::from_str(&text).with_context(|| format!("parsing config {path}"))?;
    match doc {
        serde_yaml::Value::Null => Ok(Vec::new()),
        serde_yaml::Value::Mapping(m) => m
            .iter()
            .map(|(k, v)| match k.as_str() {
                Some(k) => Ok((k.to_string(), ConfigValue::from(v))),
                None => bail!("config {path}: keys must be option names, got {k:?}"),
            })
            .collect(),
        _ => bail!("config {path} must be a mapping of option names to values"),
    }
}

fn flag_token(arg: &Arg) -> Vec<String> {
    match (arg.get_long(), arg.get_short()) {
        (Some(l), _) => vec![format!("--{l}")],
        (None, Some(s)) => vec![format!("-{s}")],
        (None, None) => Vec::new(),
    }
}

/// `--x=value`, which keeps a value that starts with `-` a value. An option with only a
/// short flag gets `-x value`.
fn value_tokens(arg: &Arg, value: &str) -> Vec<String> {
    match arg.get_long() {
        Some(l) => vec![format!("--{l}={value}")],
        None => {
            let mut t = flag_token(arg);
            t.push(value.to_string());
            t
        }
    }
}

fn scalar(key: &str, v: &ConfigValue) -> Result<String> {
    match v {
        ConfigValue::Text(s) => Ok(s.clone()),
        ConfigValue::Bool(b) => Ok(b.to_string()),
        _ => bail!("`{key}` takes a single value, got {v:?}"),
    }
}

/// The command-line arguments that set `key` to `value`.
fn arguments(cmd: &Command, key: &str, value: &ConfigValue) -> Result<Vec<String>> {
    let unsupported = crate::registry::tool_info(cmd.get_name())
        .and_then(|t| t.unsupported.iter().find(|(name, _)| *name == key));
    if let Some((_, default)) = unsupported {
        let default: Value = serde_json::from_str(default).context("registry default")?;
        if *value == ConfigValue::Null || *value == ConfigValue::from(&default) {
            return Ok(Vec::new());
        }
        bail!(
            "`{key}` is an option of upstream domainator's {} that domainatoredux does not \
             support; remove it, or set it to {default}",
            cmd.get_name()
        );
    }
    let arg = parameters(cmd)
        .find(|a| a.get_id().as_str() == key)
        .with_context(|| {
            format!(
                "unknown option `{key}` for {name}; `domainatoredux describe --tool {name}` \
                 lists the options",
                name = cmd.get_name()
            )
        })?;
    if *value == ConfigValue::Null {
        return Ok(Vec::new());
    }
    if matches!(arg.get_action(), ArgAction::SetTrue | ArgAction::SetFalse) {
        let ConfigValue::Bool(on) = value else {
            bail!("`{key}` is a switch: give true or false, not {value:?}");
        };
        let sets = matches!(arg.get_action(), ArgAction::SetTrue);
        return Ok(if *on == sets {
            flag_token(arg)
        } else {
            negation(cmd, arg).map(flag_token).unwrap_or_default()
        });
    }
    let items = match value {
        ConfigValue::List(items) => {
            if !is_multiple(arg) && arg.get_num_args().is_none_or(|r| r.max_values() <= 1) {
                bail!("`{key}` takes a single value, got a list");
            }
            items.clone()
        }
        one => vec![one.clone()],
    };
    let values = items
        .iter()
        .map(|v| scalar(key, v))
        .collect::<Result<Vec<_>>>()?;
    if values.is_empty() {
        return Ok(Vec::new());
    }
    // An option that takes N values at once (`--x a b c`) gets them in one go; the rest
    // are repeated, which appends.
    if arg.get_num_args().is_some_and(|r| r.min_values() > 1) {
        let mut out = flag_token(arg);
        out.extend(values);
        return Ok(out);
    }
    Ok(values.iter().flat_map(|v| value_tokens(arg, v)).collect())
}

/// The command-line arguments for a set of parameters (a parsed config file, or the
/// GUI's dialog values), skipping the ones in `given`.
pub fn to_arguments(
    cmd: &Command,
    params: &[(String, ConfigValue)],
    given: &HashSet<String>,
) -> Result<Vec<OsString>> {
    let mut out = Vec::new();
    for (key, value) in params {
        if given.contains(key) {
            continue;
        }
        out.extend(arguments(cmd, key, value)?.into_iter().map(OsString::from));
    }
    Ok(out)
}

/// The same, for a JSON object of parameters.
pub fn json_to_arguments(cmd: &Command, params: &Map<String, Value>) -> Result<Vec<OsString>> {
    let params: Vec<(String, ConfigValue)> = params
        .iter()
        .map(|(k, v)| (k.clone(), ConfigValue::from(v)))
        .collect();
    to_arguments(cmd, &params, &HashSet::new())
}

/// The ids of the parameters set on a command line (`--no_x` counts as setting `x`).
fn given_ids(cmd: &Command, args: &[OsString]) -> HashSet<String> {
    let mut out = HashSet::new();
    for tok in args.iter().filter_map(|a| a.to_str()) {
        let arg = if let Some(long) = tok.strip_prefix("--") {
            let name = long.split('=').next().unwrap_or(long);
            cmd.get_arguments().find(|a| a.get_long() == Some(name))
        } else if let Some(c) = tok
            .strip_prefix('-')
            .and_then(|s| s.chars().next())
            .filter(char::is_ascii_alphabetic)
        {
            cmd.get_arguments().find(|a| a.get_short() == Some(c))
        } else {
            None
        };
        if let Some(arg) = arg {
            let arg = negated(cmd, arg).unwrap_or(arg);
            out.insert(arg.get_id().to_string());
        }
    }
    out
}

/// Replace each `--config FILE` in a tool's arguments with the options the file sets,
/// leaving out any the command line sets itself. Later files win over earlier ones.
pub fn expand(cmd: &Command, args: &[OsString]) -> Result<Vec<OsString>> {
    let mut rest = Vec::new();
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == OsStr::new("--config") {
            let path = it.next().context("--config requires a file name")?;
            files.push(path.to_string_lossy().into_owned());
        } else if let Some(path) = a.to_str().and_then(|s| s.strip_prefix("--config=")) {
            files.push(path.to_string());
        } else {
            rest.push(a.clone());
        }
    }
    if files.is_empty() {
        return Ok(rest);
    }
    let mut params: Vec<(String, ConfigValue)> = Vec::new();
    for path in &files {
        for (key, value) in read(path)? {
            params.retain(|(k, _)| *k != key);
            params.push((key, value));
        }
    }
    let given = given_ids(cmd, &rest);
    let mut out = to_arguments(cmd, &params, &given)
        .with_context(|| format!("in config {}", files.join(", ")))?;
    out.extend(rest);
    Ok(out)
}

/// The value a switch ends up with: `--no_x` beats `--x`, and neither leaves the
/// default.
pub fn switch_value(cmd: &Command, m: &ArgMatches, arg: &Arg, info: &ToolInfo) -> bool {
    let id = arg.get_id().as_str();
    let set = |a: &Arg| {
        m.value_source(a.get_id().as_str()) == Some(clap::parser::ValueSource::CommandLine)
    };
    match negation(cmd, arg) {
        Some(neg) if set(neg) => false,
        _ if set(arg) => matches!(arg.get_action(), ArgAction::SetTrue),
        _ => info.switches_on.contains(&id) || matches!(arg.get_action(), ArgAction::SetFalse),
    }
}

/// `--print_config`: every parameter with its value, in `describe` order, as YAML that
/// `--config` reads back -- the shape upstream prints.
pub fn print_config(cmd: &Command, m: &ArgMatches, info: &ToolInfo) -> Result<String> {
    let mut out = Map::new();
    for arg in parameters(cmd) {
        let id = arg.get_id().as_str();
        let value = if matches!(arg.get_action(), ArgAction::SetTrue | ArgAction::SetFalse) {
            Value::Bool(switch_value(cmd, m, arg, info))
        } else {
            let ty = type_info(arg.get_value_parser()).json_type;
            let raw: Vec<Value> = m
                .get_raw(id)
                .map(|vals| {
                    vals.map(|v| typed(ty, &v.to_string_lossy()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            match (is_multiple(arg), raw.len()) {
                (_, 0) => Value::Null,
                (true, _) => Value::Array(raw),
                (false, _) => raw.into_iter().next().unwrap_or(Value::Null),
            }
        };
        out.insert(id.to_string(), value);
    }
    serde_yaml::to_string(&Value::Object(out)).context("writing the config")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::SortContigsArgs;
    use crate::registry::tool_info;
    use clap::Args;

    fn sort_contigs() -> Command {
        SortContigsArgs::augment_args(Command::new("sort_contigs"))
    }

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    fn scratch(name: &str, text: &str) -> String {
        let dir = std::env::temp_dir().join(format!("domainator-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn a_config_file_becomes_arguments_and_the_command_line_wins() {
        let path = scratch(
            "sc.yml",
            "input: [a.gb, b.gb]\nmax_hits: 3\ngenbank_out: x.gb\nverbose: false\n\
             domains_file: null\n",
        );
        let args = expand(
            &sort_contigs(),
            &os(&["--config", &path, "-o", "y.gb", "--max_hits=5"]),
        )
        .unwrap();
        assert_eq!(
            args,
            os(&["--input=a.gb", "--input=b.gb", "-o", "y.gb", "--max_hits=5"])
        );
    }

    #[test]
    fn an_unknown_key_is_an_error_that_names_it() {
        let path = scratch("bad.json", r#"{"input": ["a.gb"], "output": "x.gb"}"#);
        let err = expand(&sort_contigs(), &os(&["--config", &path])).unwrap_err();
        assert!(
            format!("{err:#}").contains("unknown option `output`"),
            "{err:#}"
        );
    }

    #[test]
    fn an_unsupported_upstream_option_is_accepted_only_at_its_default() {
        let cmd = crate::args::tool_command("enum_report").unwrap();
        let ok = scratch(
            "unsupported-ok.yml",
            "html_out: null\ntaxonomy_update: false\n",
        );
        assert!(expand(&cmd, &os(&["--config", &ok])).unwrap().is_empty());
        let bad = scratch("unsupported-bad.yml", "html_out: r.html\n");
        let e = format!("{:#}", expand(&cmd, &os(&["--config", &bad])).unwrap_err());
        assert!(
            e.contains("`html_out`") && e.contains("does not support"),
            "{e}"
        );
    }

    #[test]
    fn print_config_reads_back_as_the_same_command_line() {
        let cmd = sort_contigs();
        let info = tool_info("sort_contigs").unwrap();
        let m = cmd
            .clone()
            .try_get_matches_from(["sort_contigs", "-i", "a.gb", "--domains", "x", "y"])
            .unwrap();
        let yaml = print_config(&cmd, &m, info).unwrap();
        assert!(
            yaml.starts_with("verbose: false\nquiet: false\ninput:\n- a.gb\n"),
            "{yaml}"
        );
        let path = scratch("round.yml", &yaml);
        let again = cmd
            .clone()
            .try_get_matches_from(
                std::iter::once(OsString::from("sort_contigs"))
                    .chain(expand(&cmd, &os(&["--config", &path])).unwrap()),
            )
            .unwrap();
        assert_eq!(print_config(&cmd, &again, info).unwrap(), yaml);
    }
}
