use std::path::Path;

// Only parsed here, to fail the build on a bad spec file; the app reads the fields.
#[allow(dead_code)]
#[path = "src/tool_spec_types.rs"]
mod tool_spec_types;

fn main() {
    embed_tool_specs();
    tauri_build::build()
}

/// Parse every `../tools/*.yml` and write `$OUT_DIR/tool_specs.rs`, a list of
/// `(file name, include_str!(..))` that `src/tool_specs.rs` includes.
fn embed_tool_specs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tools");
    // A directory: cargo rescans it, so an added or removed file reruns this too.
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|e| e.expect("reading a tools/ entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "yml"))
        .collect();
    paths.sort();
    let mut code = String::from("pub const TOOL_SPEC_FILES: &[(&str, &str)] = &[\n");
    for p in &paths {
        let text =
            std::fs::read_to_string(p).unwrap_or_else(|e| panic!("reading {}: {e}", p.display()));
        if let Err(e) = serde_yaml::from_str::<tool_spec_types::ToolSpec>(&text) {
            panic!("{}: {e}", p.display());
        }
        let name = p.file_name().unwrap().to_string_lossy();
        let abs = p.canonicalize().unwrap();
        code.push_str(&format!(
            "    ({name:?}, include_str!({:?})),\n",
            abs.display().to_string()
        ));
    }
    code.push_str("];\n");
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("tool_specs.rs");
    std::fs::write(out, code).unwrap();
}
