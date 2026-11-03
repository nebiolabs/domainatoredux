//! `genbank_to_fasta` reads its inputs as GenBank whatever they are called (upstream's
//! `filetype_override="genbank"`): workflow engines stage inputs under fixed names such
//! as `input_seqs`, with no extension to go by.

use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("domainator-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/data/FeSOD_20.gb")
}

fn to_fasta(input: &Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_domainatoredux"))
        .args(["genbank_to_fasta", "-i", input.to_str().unwrap(), "-o", "-"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}: {}",
        input.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn genbank_input_without_an_extension_is_read_as_genbank() {
    let dir = scratch_dir("genbank-to-fasta");
    let expected = to_fasta(&fixture());
    assert_eq!(expected.matches('>').count(), 20);

    let plain = dir.join("input_seqs");
    std::fs::copy(fixture(), &plain).unwrap();
    assert_eq!(to_fasta(&plain), expected);

    // Compression is still judged by content, not by name.
    let gzipped = dir.join("input_seqs_gz");
    let mut gz = flate2::write::GzEncoder::new(
        std::fs::File::create(&gzipped).unwrap(),
        flate2::Compression::default(),
    );
    std::io::copy(&mut std::fs::File::open(fixture()).unwrap(), &mut gz).unwrap();
    gz.finish().unwrap();
    assert_eq!(to_fasta(&gzipped), expected);

    let _ = std::fs::remove_dir_all(&dir);
}
