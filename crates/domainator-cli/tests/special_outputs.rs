//! A device or FIFO output is written directly. Writing it under a temporary name and
//! renaming that over it failed on `/dev/null` (`/dev` cannot hold the temp file), which
//! upstream accepts and which 0.x workflows used to drop an unwanted output.

#![cfg(unix)]

use std::path::Path;
use std::process::Command;

#[test]
fn outputs_can_be_dev_null() {
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/data");
    let out = Command::new(env!("CARGO_BIN_EXE_domainatoredux"))
        .args(["genbank_to_fasta", "-i"])
        .arg(data.join("FeSOD_20.gb"))
        .args(["-o", "/dev/null"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(Path::new("/dev/null").exists());
}
