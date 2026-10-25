//! `domainatoredux describe` against upstream's `domainator describe`.
//!
//! Upstream's manifest (`scripts/parity/upstream_manifest.json`, refreshed by
//! `scripts/parity/refresh_manifest.py` at each sync) is the complete, explicit
//! interface of every upstream tool. Every way this port's interface differs from it --
//! a missing tool or option, a different flag, type, default, bound, choice or role --
//! must be listed in `scripts/parity/manifest_allowlist.tsv`, and every row there must
//! still be a real difference. So a new upstream option fails this test at the next
//! sync, and a fixed difference cannot linger as a stale row (the same two-way rule as
//! the parity sweep's `known_gaps.tsv`).
//!
//! Rows carry the compared values on both sides, so an upstream change to an
//! allowlisted value fails again. `UPDATE_MANIFEST_ALLOWLIST=1 cargo test -p
//! domainator-cli --test upstream_manifest` rewrites the file from the current
//! differences, keeping the hand-written `class` and `note` of rows that remain.
//!
//! Classes: `deferred:` (not ported, on purpose), `port-only:` (ours alone),
//! `upstream:` (a difference upstream should fix), `todo:` (allowed only while the
//! 1.0 migration is in progress).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::{json, Map, Value};

const HEADER: &str = "\
# Every difference between `domainatoredux describe` and upstream's `domainator describe`
# (scripts/parity/upstream_manifest.json). Enforced both ways by
# crates/domainator-cli/tests/upstream_manifest.rs; regenerate membership with
#   UPDATE_MANIFEST_ALLOWLIST=1 cargo test -p domainator-cli --test upstream_manifest
# which keeps the class and note columns. `ours`/`upstream` are compact JSON.
# Classes: deferred: (not ported, on purpose)  port-only: (ours alone)
#          upstream: (upstream should change)   todo: (the 1.0 migration; none may remain)
tool\tparameter\taspect\tours\tupstream\tclass\tnote
";

/// Upstream tools this port does not have, and why.
const DEFERRED_TOOLS: &[(&str, &str)] = &[
    ("build_projection", "deferred:unported-tool"),
    ("build_tree", "deferred:unported-tool"),
    ("db_download", "deferred:unported-tool"),
    ("find_features", "deferred:unported-tool"),
    ("infernal_build", "deferred:infernal"),
    ("kmer_dist", "deferred:unported-tool"),
    ("plot_contigs", "deferred:unported-tool"),
    ("split_by_cluster", "deferred:unported-tool"),
    ("structure_dist", "deferred:structure"),
    ("structure_domainate", "deferred:structure"),
    ("structure_search", "deferred:structure"),
    ("structure_to_genbank", "deferred:structure"),
    ("rotate_contigs", "todo:new-tool"),
];

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Numbers by value (`1` == `1.0`), recursively.
fn norm(v: &Value) -> Value {
    match v {
        Value::Number(n) => match n.as_f64() {
            Some(f) if f.fract() == 0.0 && f.abs() < 1e15 => {
                // Whole and well inside i64's range, so exact.
                #[allow(clippy::cast_possible_truncation)]
                let i = f as i64;
                json!(i)
            }
            Some(f) => json!(f),
            None => v.clone(),
        },
        Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), norm(v))).collect()),
        _ => v.clone(),
    }
}

fn compact(v: &Value) -> String {
    serde_json::to_string(&norm(v)).unwrap()
}

/// A list compared as a set: sorted.
fn as_set(v: &Value) -> Value {
    match v.as_array() {
        Some(a) => {
            let mut items: Vec<Value> = a.iter().map(norm).collect();
            items.sort_by_key(compact);
            Value::Array(items)
        }
        None => v.clone(),
    }
}

/// A parameter without its prose, for `missing`/`extra` rows.
fn bare(p: &Value) -> Value {
    let mut p = p.clone();
    if let Some(o) = p.as_object_mut() {
        o.remove("help");
        if let Some(Value::Array(cols)) = o.get_mut("columns") {
            for c in cols {
                if let Some(c) = c.as_object_mut() {
                    c.remove("help");
                }
            }
        }
    }
    norm(&p)
}

type Key = (String, String, String);

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    tool: String,
    parameter: String,
    aspect: String,
    ours: String,
    upstream: String,
}

impl Row {
    fn key(&self) -> Key {
        (
            self.tool.clone(),
            self.parameter.clone(),
            self.aspect.clone(),
        )
    }
}

fn aspects(ours: &Map<String, Value>, theirs: &Map<String, Value>) -> Vec<(String, Value, Value)> {
    let get = |m: &Map<String, Value>, k: &str| m.get(k).cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();
    let mut push = |name: &str, a: Value, b: Value| {
        if norm(&a) != norm(&b) {
            out.push((name.to_string(), a, b));
        }
    };
    for k in [
        "flags", "kind", "type", "role", "count", "sets", "stdin", "stdout",
    ] {
        push(k, get(ours, k), get(theirs, k));
    }
    let flag = |m: &Map<String, Value>, k: &str| get(m, k).as_bool().unwrap_or(false);
    push(
        "multiple",
        json!(flag(ours, "multiple")),
        json!(flag(theirs, "multiple")),
    );
    if ours.get("kind") != Some(&json!("flag")) || theirs.get("kind") != Some(&json!("flag")) {
        push(
            "required",
            json!(flag(ours, "required")),
            json!(flag(theirs, "required")),
        );
    }
    let bounds = |m: &Map<String, Value>| json!({"min": get(m, "min"), "max": get(m, "max"), "exclusive_min": get(m, "exclusive_min")});
    push("bounds", bounds(ours), bounds(theirs));
    push(
        "choices",
        as_set(&get(ours, "choices")),
        as_set(&get(theirs, "choices")),
    );
    push(
        "negative_flags",
        get(ours, "negative_flags"),
        get(theirs, "negative_flags"),
    );
    let dynamic = |m: &Map<String, Value>| flag(m, "default_dynamic");
    if dynamic(ours) || dynamic(theirs) {
        push(
            "default_dynamic",
            json!(dynamic(ours)),
            json!(dynamic(theirs)),
        );
    } else {
        // An empty list and no default mean the same for a list option.
        let default = |m: &Map<String, Value>| match get(m, "default") {
            Value::Array(a) if a.is_empty() => Value::Null,
            d => d,
        };
        push("default", default(ours), default(theirs));
    }
    let columns =
        |m: &Map<String, Value>| bare(&json!({"columns": get(m, "columns")}))["columns"].clone();
    push("columns", columns(ours), columns(theirs));
    out
}

fn differences(ours: &Value, theirs: &Value) -> Vec<Row> {
    let by_name = |doc: &Value| -> BTreeMap<String, Value> {
        doc["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| (t["name"].as_str().unwrap().to_string(), t.clone()))
            .collect()
    };
    let (ours_t, theirs_t) = (by_name(ours), by_name(theirs));
    let mut rows = Vec::new();
    let mut row = |tool: &str, parameter: &str, aspect: &str, o: &Value, u: &Value| {
        rows.push(Row {
            tool: tool.into(),
            parameter: parameter.into(),
            aspect: aspect.into(),
            ours: compact(o),
            upstream: compact(u),
        });
    };
    let names: BTreeSet<&String> = ours_t.keys().chain(theirs_t.keys()).collect();
    for name in names {
        let (Some(o), Some(u)) = (ours_t.get(name), theirs_t.get(name)) else {
            row(
                name,
                "-",
                "tool",
                &json!(ours_t.contains_key(name)),
                &json!(theirs_t.contains_key(name)),
            );
            continue;
        };
        for k in ["category", "required_outputs"] {
            if norm(&o[k]) != norm(&u[k]) {
                row(name, "-", k, &o[k], &u[k]);
            }
        }
        let exclusive = |t: &Value| {
            as_set(&Value::Array(
                t["mutually_exclusive"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(as_set)
                    .collect(),
            ))
        };
        if exclusive(o) != exclusive(u) {
            row(
                name,
                "-",
                "mutually_exclusive",
                &exclusive(o),
                &exclusive(u),
            );
        }
        let params = |t: &Value| -> BTreeMap<String, Map<String, Value>> {
            t["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p["name"].as_str().unwrap().to_string(),
                        p.as_object().unwrap().clone(),
                    )
                })
                .collect()
        };
        let (op, up) = (params(o), params(u));
        let pnames: BTreeSet<&String> = op.keys().chain(up.keys()).collect();
        for p in pnames {
            match (op.get(p), up.get(p)) {
                (Some(a), Some(b)) => {
                    for (aspect, x, y) in aspects(a, b) {
                        row(name, p, &aspect, &x, &y);
                    }
                }
                (None, Some(b)) => row(
                    name,
                    p,
                    "missing",
                    &Value::Null,
                    &bare(&Value::Object(b.clone())),
                ),
                (Some(a), None) => row(
                    name,
                    p,
                    "extra",
                    &bare(&Value::Object(a.clone())),
                    &Value::Null,
                ),
                (None, None) => unreachable!(),
            }
        }
    }
    rows.sort();
    rows
}

fn seed_class(r: &Row) -> String {
    if r.aspect == "tool" {
        if let Some((_, c)) = DEFERRED_TOOLS.iter().find(|(t, _)| *t == r.tool) {
            return (*c).to_string();
        }
        if r.ours == "true" {
            return "port-only:".to_string();
        }
    }
    if r.tool == "gene_call" {
        return "port-only:".to_string();
    }
    if r.parameter.starts_with("html_") {
        return "deferred:html".to_string();
    }
    if r.aspect == "missing" {
        let unsupported = domainator_config::registry::tool_info(&r.tool)
            .is_some_and(|t| t.unsupported.iter().any(|(n, _)| *n == r.parameter));
        if unsupported {
            return "deferred:unsupported".to_string();
        }
    }
    "todo:1.0-cli".to_string()
}

fn load_allowlist(path: &PathBuf) -> Vec<(Row, String, String)> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty() && !l.starts_with("tool\t"))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert!(f.len() >= 6, "manifest_allowlist.tsv: malformed row: {l}");
            (
                Row {
                    tool: f[0].into(),
                    parameter: f[1].into(),
                    aspect: f[2].into(),
                    ours: f[3].into(),
                    upstream: f[4].into(),
                },
                f[5].to_string(),
                f.get(6).copied().unwrap_or_default().to_string(),
            )
        })
        .collect()
}

/// The CLI's `Command` is one large derive, whose construction overflows a test
/// thread's 2 MB stack in a debug build (the GUI runs the CLI on a big stack too).
fn with_big_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e));
}

#[test]
fn describe_matches_upstream_up_to_the_allowlist() {
    with_big_stack(check_against_the_allowlist);
}

fn check_against_the_allowlist() {
    let snapshot = repo().join("scripts/parity/upstream_manifest.json");
    let theirs: Value = serde_json::from_str(&std::fs::read_to_string(&snapshot).unwrap()).unwrap();
    let ours = domainator_config::manifest::manifest(&domainator_cli::command(), None);
    assert_eq!(ours["manifest_version"], theirs["manifest_version"]);
    let actual = differences(&ours, &theirs);

    let path = repo().join("scripts/parity/manifest_allowlist.tsv");
    let listed = load_allowlist(&path);
    if std::env::var_os("UPDATE_MANIFEST_ALLOWLIST").is_some() {
        let carried: BTreeMap<Key, (String, String)> = listed
            .iter()
            .map(|(r, c, n)| (r.key(), (c.clone(), n.clone())))
            .collect();
        let mut text = HEADER.to_string();
        for r in &actual {
            let (class, note) = carried
                .get(&r.key())
                .cloned()
                .unwrap_or_else(|| (seed_class(r), String::new()));
            text.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{class}\t{note}\n",
                r.tool, r.parameter, r.aspect, r.ours, r.upstream
            ));
        }
        std::fs::write(&path, text).unwrap();
        eprintln!("wrote {} rows to {}", actual.len(), path.display());
        return;
    }

    for (r, class, _) in &listed {
        assert!(
            ["deferred:", "port-only:", "upstream:", "todo:"]
                .iter()
                .any(|p| class.starts_with(p)),
            "manifest_allowlist.tsv: {}/{}/{} has class {class:?}",
            r.tool,
            r.parameter,
            r.aspect
        );
    }
    check_unsupported(&theirs, &listed);
    let listed: BTreeSet<Row> = listed.into_iter().map(|(r, _, _)| r).collect();
    let actual: BTreeSet<Row> = actual.into_iter().collect();
    let new: Vec<&Row> = actual.difference(&listed).collect();
    let stale: Vec<&Row> = listed.difference(&actual).collect();
    let show = |rows: &[&Row]| {
        rows.iter()
            .map(|r| {
                format!(
                    "  {}\t{}\t{}\t{}\t{}",
                    r.tool, r.parameter, r.aspect, r.ours, r.upstream
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        new.is_empty() && stale.is_empty(),
        "describe disagrees with scripts/parity/manifest_allowlist.tsv \
         (UPDATE_MANIFEST_ALLOWLIST=1 rewrites it, keeping classes).\n\
         {} unlisted difference(s):\n{}\n{} stale row(s):\n{}",
        new.len(),
        show(&new),
        stale.len(),
        show(&stale)
    );
}

/// A tool lacks an upstream option only as a registry `unsupported` entry,
/// which accepts upstream's default from a config file; so every `missing` row of a
/// tool is one, its default is upstream's, and its class is `deferred:`.
fn check_unsupported(theirs: &Value, listed: &[(Row, String, String)]) {
    use domainator_config::registry::TOOLS;
    for info in TOOLS.iter().filter(|t| t.in_manifest) {
        let upstream: &[Value] = theirs["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == info.name)
            .and_then(|t| t["parameters"].as_array())
            .map_or(&[], Vec::as_slice);
        for (name, default) in info.unsupported {
            let p = upstream
                .iter()
                .find(|p| p["name"] == *name)
                .unwrap_or_else(|| panic!("{}: unsupported {name} is not upstream's", info.name));
            let default: Value = serde_json::from_str(default).unwrap();
            assert_eq!(
                p["default"], default,
                "{}: unsupported {name}: upstream's default",
                info.name
            );
            let row = listed.iter().find(|(r, _, _)| {
                r.tool == info.name && r.parameter == *name && r.aspect == "missing"
            });
            assert!(
                row.is_some_and(|(_, class, _)| class.starts_with("deferred:")),
                "{}: unsupported {name} needs a deferred: `missing` row",
                info.name
            );
        }
        for (r, _, _) in listed {
            if r.tool == info.name && r.aspect == "missing" {
                assert!(
                    info.unsupported.iter().any(|(n, _)| *n == r.parameter),
                    "{}: upstream's {} is missing; add it, or list it in the registry's \
                     `unsupported` so upstream configs still load",
                    info.name,
                    r.parameter
                );
            }
        }
    }
}
