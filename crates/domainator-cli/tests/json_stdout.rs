//! `--json -` is the agent contract (CLAUDE.md, "Agent-facing `--json`"): the JSON goes
//! to stdout and stdout carries nothing else, so a caller can parse it directly.

use std::process::Command;

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("domainator-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn matrix_report_json_dash_is_parseable_stdout() {
    let dir = scratch_dir("json-stdout");
    let matrix = dir.join("m.tsv");
    std::fs::write(
        &matrix,
        "\ta\tb\tc\na\t10.0\t3.0\t0.0\nb\t3.0\t10.0\t1.0\nc\t0.0\t1.0\t10.0\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_domainatoredux"))
        .current_dir(&dir)
        .args([
            "matrix_report",
            "-i",
            matrix.to_str().unwrap(),
            "--json",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not JSON ({e}): {}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_eq!(json["nodes"], 3);
    assert!(
        !dir.join("-").exists(),
        "wrote a file named '-' instead of stdout"
    );
    std::fs::remove_dir_all(&dir).ok();
}
