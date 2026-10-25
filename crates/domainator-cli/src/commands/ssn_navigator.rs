use std::io::Write;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use domainator_config::args::SsnNavigatorArgs;
use domainator_core::io::python_json;
use domainator_core::tools::ssn_bundle::load_bundle;
use domainator_core::tools::ssn_navigator::{
    ssn_navigator, NavMode, SsnNavigatorOptions, JSON_SCHEMA_VERSION,
};
use serde_json::{json, Map, Value};

use crate::output::Output;

pub fn run(args: &SsnNavigatorArgs) -> Result<()> {
    let count =
        |value: i64, name: &str| usize::try_from(value).map_err(|_| anyhow!("{name} must be >= 0"));
    let opts = SsnNavigatorOptions {
        query: NavMode::parse(&args.query).map_err(anyhow::Error::msg)?,
        threshold: args.threshold.clone(),
        cluster_id: args.cluster,
        node: args.node.clone(),
        // Every cluster has at least one member, so a negative minimum is no minimum.
        min_size: usize::try_from(args.min_size).unwrap_or(0),
        top_n: count(args.top_n, "top_n")?,
        members: args.members,
        max_merge_events: count(args.max_merge_events, "max_merge_events")?,
    };
    let bundle = load_bundle(Path::new(&args.input))
        .with_context(|| format!("reading bundle {}", args.input))?;
    let result = ssn_navigator(&bundle, &opts).map_err(anyhow::Error::msg)?;

    let mut document = Map::new();
    document.insert("schema_version".into(), json!(JSON_SCHEMA_VERSION));
    document.insert("query".into(), json!(args.query));
    if let Value::Object(fields) = result {
        document.extend(fields);
    }
    let path = args
        .json_out
        .as_deref()
        .expect("json_out is a required output");
    let mut out = Output::create(path)?;
    writeln!(out, "{}", python_json::to_string(&Value::Object(document)))?;
    out.finish()
}
