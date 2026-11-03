//! `domainate --foldseek`: protein sequences searched against foldseek databases by
//! their predicted 3Di sequences (upstream `foldseek.py`).
//!
//! Upstream predicts 3Di with ESM-2 3B 3Di in PyTorch, or with ProstT5 inside foldseek
//! (`--prostt5_weights`). Only the ProstT5 route is ported: foldseek turns the protein
//! FASTA into a query database itself (`createdb --prostt5-model`), so the whole search
//! is the external `foldseek` binary, driven as a subprocess like the structure tools.

use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::search::results::SearchResult;
use crate::tools::external::TempDir;
use crate::tools::structure_lib::run_command;

/// Upstream `foldseek.MAX_PROTEIN_SIZE`: longer proteins are not searched.
pub const MAX_PROTEIN_SIZE: usize = 2500;

/// The columns asked of `convertalis`, in upstream's `FoldseekHit` order.
const FORMAT_OUTPUT: &str =
    "query,target,qheader,theader,pident,alnlen,mismatch,gapopen,qstart,qend,tstart,tend,evalue,bits,qlen,tlen";

/// A `--foldseek` search: the databases, and how foldseek is run.
#[derive(Clone, Debug)]
pub struct FoldseekSearch {
    /// `(name, path)` per database, the name being the path's file name.
    pub databases: Vec<(String, String)>,
    pub binary: PathBuf,
    /// foldseek's ProstT5 weights directory (`foldseek databases ProstT5`).
    pub prostt5_weights: String,
    /// `--prostt5_device`: `cpu`, or a CUDA device string.
    pub prostt5_device: String,
    /// `--foldseek_device`: the device the search runs on.
    pub device: String,
}

impl FoldseekSearch {
    /// The search for these `--foldseek` databases, named as upstream's
    /// `read_references` names them (`Path(file).name`).
    pub fn new(
        databases: &[String],
        binary: PathBuf,
        prostt5_weights: String,
        prostt5_device: String,
        device: String,
    ) -> Self {
        let databases = databases
            .iter()
            .map(|path| {
                let name = Path::new(path)
                    .file_name()
                    .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
                (name, path.clone())
            })
            .collect();
        FoldseekSearch {
            databases,
            binary,
            prostt5_weights,
            prostt5_device,
            device,
        }
    }
}

/// Whether a 3Di predictor is run on `prot` (upstream `predictable`): at most
/// [`MAX_PROTEIN_SIZE`] residues, all of them the 20 standard amino acids.
pub fn predictable(prot: &[u8]) -> bool {
    prot.len() <= MAX_PROTEIN_SIZE && prot.iter().all(|b| b"ACDEFGHIKLMNPQRSTVWY".contains(b))
}

/// foldseek's options and environment for running on `device` (upstream `_gpu_options`):
/// `cpu` needs none; a CUDA device string adds `--gpu 1`, and `cuda:N` selects the GPU
/// through `CUDA_VISIBLE_DEVICES`, which is how foldseek picks one.
fn gpu_options(device: &str) -> (Vec<String>, Vec<(&'static str, String)>) {
    if device == "cpu" {
        return (Vec::new(), Vec::new());
    }
    let env = match device.split_once(':') {
        Some((_, index)) => vec![("CUDA_VISIBLE_DEVICES", index.to_string())],
        None => Vec::new(),
    };
    (vec!["--gpu".to_string(), "1".to_string()], env)
}

/// The queries as a foldseek database, its 3Di sequences predicted by ProstT5. The
/// prediction depends only on the queries, and it is by far the costliest step, so one
/// is built per run and searched against every `--foldseek` database ([`search_db`]).
pub struct QueryDb {
    /// Holds the database's files; removed on drop.
    _dir: TempDir,
    /// The database's path prefix, as foldseek names it.
    base: String,
}

/// Build the query database for `peptides` (named `"{contig},{cds}"`): the
/// [`predictable`] ones, through `foldseek createdb --prostt5-model` (upstream
/// `build_query_db`). `None` when no peptide is predictable. `cpu` is the resolved thread
/// count.
pub fn build_query_db(
    search: &FoldseekSearch,
    peptides: &[(String, Vec<u8>)],
    cpu: usize,
) -> io::Result<Option<QueryDb>> {
    let selected: Vec<&(String, Vec<u8>)> =
        peptides.iter().filter(|(_, p)| predictable(p)).collect();
    if selected.is_empty() {
        return Ok(None);
    }
    let dir = TempDir::new("domainatoredux_foldseek")?;
    let protein_fasta = dir.path().join("protein.fasta");
    let base = dir.path().join("output").to_string_lossy().into_owned();
    {
        let mut out = BufWriter::new(File::create(&protein_fasta)?);
        for (name, prot) in &selected {
            // Upstream writes `>{name} {description}`; the description is empty.
            writeln!(out, ">{name} ")?;
            out.write_all(prot)?;
            out.write_all(b"\n")?;
        }
        out.flush()?;
    }

    let (prostt5_gpu, prostt5_env) = gpu_options(&search.prostt5_device);
    let mut createdb = vec![
        search.binary.to_string_lossy().into_owned(),
        "createdb".into(),
        protein_fasta.to_string_lossy().into_owned(),
        base.clone(),
        "--prostt5-model".into(),
        search.prostt5_weights.clone(),
    ];
    createdb.extend(threads(cpu));
    createdb.extend(prostt5_gpu);
    run_command(&createdb, "foldseek createdb", None, &prostt5_env)?;
    Ok(Some(QueryDb { _dir: dir, base }))
}

/// foldseek's `--threads` option for `cpu` threads (none for `0`).
fn threads(cpu: usize) -> Vec<String> {
    if cpu > 0 {
        vec!["--threads".to_string(), cpu.to_string()]
    } else {
        Vec::new()
    }
}

/// Search `query` against one database; the hits as `(contig, cds, result)`, in the order
/// foldseek reports them, within `[min_evalue, evalue]` (upstream `search_query_db` +
/// `foldseek_hits_to_search_results`). `cpu` is the resolved thread count.
pub fn search_db(
    search: &FoldseekSearch,
    query: &QueryDb,
    db_name: &str,
    db_path: &str,
    cpu: usize,
    evalue: f64,
    min_evalue: f64,
) -> io::Result<Vec<(usize, usize, SearchResult)>> {
    // The alignment and foldseek's scratch space last only as long as this database's
    // search, so a run over many databases holds one at a time.
    let tmp = TempDir::new("domainatoredux_foldseek_search")?;
    let path = |name: &str| tmp.path().join(name).to_string_lossy().into_owned();
    let aln = path("aln");
    let tmp_folder = path("foldseek_tmpfolder");
    let results = format!("{tmp_folder}/results.tsv");
    let binary = search.binary.to_string_lossy().into_owned();

    let (search_gpu, search_env) = gpu_options(&search.device);
    let mut argv = vec![
        binary.clone(),
        "search".into(),
        query.base.clone(),
        db_path.to_string(),
        aln.clone(),
        tmp_folder.clone(),
        "-e".into(),
        crate::matrix::text::py_float_str(evalue),
    ];
    argv.extend(threads(cpu));
    argv.extend(search_gpu);
    run_command(&argv, "foldseek search", None, &search_env)?;

    let convertalis = vec![
        binary,
        "convertalis".into(),
        "--format-output".into(),
        FORMAT_OUTPUT.into(),
        query.base.clone(),
        db_path.to_string(),
        aln,
        results.clone(),
    ];
    run_command(&convertalis, "convertalis", None, &[])?;

    let mut hits = Vec::new();
    for line in BufReader::new(File::open(&results)?).lines() {
        let line = line?;
        if let Some(hit) = parse_hit(line.trim(), db_name, evalue, min_evalue)? {
            hits.push(hit);
        }
    }
    Ok(hits)
}

/// One `convertalis` line as `(contig, cds, result)`, or `None` when its E-value is
/// outside `[min_evalue, evalue]`: foldseek's own `-e` already bounds it, but
/// `--min_evalue` has no foldseek equivalent.
fn parse_hit(
    line: &str,
    db_name: &str,
    evalue: f64,
    min_evalue: f64,
) -> io::Result<Option<(usize, usize, SearchResult)>> {
    let bad = || io::Error::other(format!("Unexpected foldseek output line: {line}"));
    let fields: Vec<&str> = line.split('\t').collect();
    if fields.len() != 16 {
        return Err(bad());
    }
    let float = |i: usize| fields[i].parse::<f64>().map_err(|_| bad());
    let int = |i: usize| fields[i].parse::<i64>().map_err(|_| bad());
    let (contig, cds) = fields[0].split_once(',').ok_or_else(bad)?;
    let contig: usize = contig.parse().map_err(|_| bad())?;
    let cds: usize = cds.parse().map_err(|_| bad())?;
    let hit_evalue = float(12)?;
    if !(hit_evalue <= evalue && hit_evalue >= min_evalue) {
        return Ok(None);
    }
    Ok(Some((
        contig,
        cds,
        SearchResult {
            name: fields[1].to_string(),
            desc: fields[3].to_string(),
            acc: String::new(),
            evalue: hit_evalue,
            score: float(13)?,
            start: int(8)? - 1,
            end: int(9)?,
            database: db_name.to_string(),
            identity: float(4)?,
            rstart: int(10)?,
            rend: int(11)?,
            rlen: int(15)?,
            program: "foldseek".to_string(),
            strand: 1,
            tmscore: None,
            lddt: None,
            rmsd: None,
            prob: None,
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ProstT5 is the expensive step and depends only on the queries, so a search over
    /// several databases runs `createdb` once and only `search`/`convertalis` per
    /// database. A stand-in foldseek logs each call and reports one hit per search.
    #[cfg(unix)]
    #[test]
    fn prostt5_runs_once_for_many_databases() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("domainator_fake_foldseek_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("calls.log");
        let script = dir.join("foldseek");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\n\
                 echo \"$*\" >> '{}'\n\
                 for a; do last=$a; done\n\
                 case \"$1\" in\n\
                 createdb) : > \"$3\" ;;\n\
                 convertalis) mkdir -p \"$(dirname \"$last\")\"; \
                 printf '0,1\\tT\\t0,1 \\tT desc\\t50.0\\t10\\t5\\t0\\t1\\t10\\t1\\t10\\t1.0E-5\\t40\\t10\\t10\\n' \
                 > \"$last\" ;;\n\
                 esac\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let search = FoldseekSearch::new(
            &["/dbs/A".to_string(), "/dbs/B".to_string()],
            script,
            "weights".to_string(),
            "cpu".to_string(),
            "cpu".to_string(),
        );
        let peptides = vec![
            ("0,1".to_string(), b"MKVLA".to_vec()),
            ("0,2".to_string(), b"MKVXA".to_vec()), // not predictable: never sent
        ];
        let query = build_query_db(&search, &peptides, 2).unwrap().unwrap();
        let fasta = std::fs::read_to_string(query._dir.path().join("protein.fasta")).unwrap();
        assert_eq!(fasta, ">0,1 \nMKVLA\n");
        let mut found = Vec::new();
        for (name, path) in &search.databases {
            for (contig, cds, hit) in search_db(&search, &query, name, path, 2, 1e-3, 0.0).unwrap()
            {
                found.push((contig, cds, hit.database));
            }
        }
        assert_eq!(found, [(0, 1, "A".to_string()), (0, 1, "B".to_string())]);

        let calls = std::fs::read_to_string(&log).unwrap();
        let programs: Vec<&str> = calls
            .lines()
            .map(|l| l.split(' ').next().unwrap())
            .collect();
        assert_eq!(
            programs,
            ["createdb", "search", "convertalis", "search", "convertalis"]
        );
        assert!(calls
            .lines()
            .next()
            .unwrap()
            .contains("--prostt5-model weights"));
        assert!(calls.lines().nth(1).unwrap().contains(" /dbs/A "));
        assert!(calls.lines().nth(3).unwrap().contains(" /dbs/B "));

        let none = build_query_db(&search, &peptides[1..], 2).unwrap();
        assert!(none.is_none(), "nothing predictable: no createdb");
        drop(query);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn predictable_matches_upstream_selection() {
        assert!(predictable(b"MKV"));
        assert!(!predictable(b"MKVX"));
        assert!(!predictable(b"MKV*"));
        assert!(!predictable(b"mkv"));
        assert!(predictable(&vec![b'M'; MAX_PROTEIN_SIZE]));
        assert!(!predictable(&vec![b'M'; MAX_PROTEIN_SIZE + 1]));
    }

    #[test]
    fn gpu_options_map_cuda_devices_like_upstream() {
        assert_eq!(gpu_options("cpu"), (Vec::new(), Vec::new()));
        let (opts, env) = gpu_options("cuda");
        assert_eq!(opts, ["--gpu", "1"]);
        assert!(env.is_empty());
        let (opts, env) = gpu_options("cuda:1");
        assert_eq!(opts, ["--gpu", "1"]);
        assert_eq!(env, [("CUDA_VISIBLE_DEVICES", "1".to_string())]);
    }

    #[test]
    fn parse_hit_reads_convertalis_columns() {
        let line = "3,7\tFeSOD_R7J7P3\t3,7 \tFeSOD_R7J7P3 unreviewed Superoxide\t39.7\t190\t\
                    100\t3\t1\t193\t1\t189\t1.0E-19\t882\t196\t190";
        let (contig, cds, r) = parse_hit(line, "FeSOD", 0.001, 0.0).unwrap().unwrap();
        assert_eq!((contig, cds), (3, 7));
        assert_eq!(r.name, "FeSOD_R7J7P3");
        assert_eq!(r.desc, "FeSOD_R7J7P3 unreviewed Superoxide");
        assert_eq!(
            (r.start, r.end, r.rstart, r.rend, r.rlen),
            (0, 193, 1, 189, 190)
        );
        assert_eq!((r.score, r.identity, r.evalue), (882.0, 39.7, 1.0e-19));
        assert_eq!(r.database, "FeSOD");
        assert!(parse_hit(line, "FeSOD", 1e-20, 0.0).unwrap().is_none());
        assert!(parse_hit(line, "FeSOD", 0.001, 1e-10).unwrap().is_none());
        assert!(parse_hit("1,2\tx", "FeSOD", 0.001, 0.0).is_err());
    }
}
