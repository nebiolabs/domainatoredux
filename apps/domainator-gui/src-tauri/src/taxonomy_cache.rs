//! The NCBI taxonomy last loaded, kept for the next run that asks for it.
//!
//! Parsing the full NCBI taxdump takes about 9 s and peaks near 2 GB (release
//! build, 2026). The Enum Report tab reruns its report every time its columns
//! change, and each run with a taxonomy column would otherwise pay that again.
//! One slot, not a map: the app has one taxonomy database in practice, and holding
//! two parsed copies of a 2.7-million-node tree is the cost this must not grow into.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use domainator_core::seq::taxonomy::NCBITaxonomy;

/// What makes a loaded taxonomy still current: the same path, unchanged on disk.
#[derive(PartialEq)]
struct Key {
    path: PathBuf,
    len: u64,
    modified: Option<SystemTime>,
}

static SLOT: Mutex<Option<(Key, Arc<NCBITaxonomy>)>> = Mutex::new(None);

/// The taxonomy at `path` (a taxdump archive or a directory, as
/// [`NCBITaxonomy::load`] takes), from the slot when it is the one last loaded.
pub fn load(path: &str) -> Result<Arc<NCBITaxonomy>, String> {
    let p = Path::new(path);
    let meta = std::fs::metadata(p).map_err(|e| format!("taxonomy database {path}: {e}"))?;
    let key = Key { path: p.to_path_buf(), len: meta.len(), modified: meta.modified().ok() };
    // Held across the load, so two requests for the same taxonomy parse it once:
    // the second waits, then finds it in the slot.
    let mut slot = SLOT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, tax)) = slot.as_ref() {
        if *k == key {
            return Ok(tax.clone());
        }
    }
    // Let go of the old one first, so two are never held at once.
    *slot = None;
    let tax = Arc::new(NCBITaxonomy::load(p).map_err(|e| format!("loading NCBI taxonomy: {e}"))?);
    *slot = Some((key, tax.clone()));
    Ok(tax)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn taxdump(dir: &Path, species: &str) {
        std::fs::create_dir_all(dir).unwrap();
        for (file, body) in [
            ("nodes.dmp", "1\t|\t1\t|\tno rank\t|\t\n562\t|\t1\t|\tspecies\t|\t\n".to_string()),
            ("names.dmp", format!("1\t|\troot\t|\t\t|\tscientific name\t|\t\n562\t|\t{species}\t|\t\t|\tscientific name\t|\t\n")),
            ("merged.dmp", String::new()),
            ("delnodes.dmp", String::new()),
        ] {
            std::fs::write(dir.join(file), body).unwrap();
        }
    }

    /// A second request is served from memory, and a different taxonomy replaces
    /// it rather than being answered with the stale one.
    #[test]
    fn a_repeat_load_is_shared_and_a_new_path_replaces_it() {
        let root = std::env::temp_dir().join(format!("domainator-taxcache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (a, b) = (root.join("a"), root.join("b"));
        taxdump(&a, "Escherichia coli");
        taxdump(&b, "Shigella flexneri");

        let first = load(a.to_str().unwrap()).unwrap();
        let again = load(a.to_str().unwrap()).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "the same taxonomy must not be parsed twice");

        let other = load(b.to_str().unwrap()).unwrap();
        assert_eq!(other.name(562), Some("Shigella flexneri"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
