//! `format_db` (`domainator_format_db`): shard, BGZF-compress, and/or `.didx`-index
//! a `domain_search` database. A port of `domainator.domainator_format_db`.
//!
//! - `shards = Some(n)` splits each input into `n` files balanced by record count
//!   (`mydb.gb` -> `mydb.0.gb`, `mydb.1.gb`, ...).
//! - `compress` writes BGZF output (gunzip-readable, seekable, ~3-4x smaller).
//! - `index` writes a `.didx` offset index per output file (so `domain_search`
//!   skips its per-run offset scan).
//!
//! At least one of `shards`, `compress`, `index` must be requested. Shard content
//! is a byte-exact range copy of the (decompressed) record stream, so records are
//! preserved exactly. The on-disk artifacts (`.didx`, shard names, BGZF blocks)
//! are the same hard contracts `domain_search` already reads back.
//!
//! Compared to the Python tool this supports uncompressed and BGZF inputs (plain
//! non-BGZF gzip is rejected — the same restriction the rest of the Rust port's
//! seekable I/O layer imposes).

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::io::bgzf as io_bgzf;
use crate::io::didx;
use crate::io::partition::scan_offsets_uncached;
use crate::io::shards::shard_path;
use crate::io::{detect_compression, detect_filetype, Compression, FileType};

const COPY_CHUNK: usize = 1 << 20;
const COMPRESSION_EXTS: [&str; 3] = ["gz", "bgz", "bgzf"];

/// Options for [`format_db`].
pub struct FormatDbOptions {
    /// Split each input into this many shards (balanced by record count).
    pub shards: Option<usize>,
    /// Write BGZF output.
    pub compress: bool,
    /// Write a `.didx` index per output file.
    pub index: bool,
    /// Output directory (default: alongside each input).
    pub output_dir: Option<String>,
    /// Base name for the output database (requires a single input).
    pub name: Option<String>,
    /// BGZF/deflate compression level (1-12).
    pub level: u8,
    /// Overwrite existing outputs / rebuild indexes instead of skipping.
    pub force: bool,
}

impl Default for FormatDbOptions {
    fn default() -> Self {
        FormatDbOptions {
            shards: None,
            compress: false,
            index: false,
            output_dir: None,
            name: None,
            level: 6,
            force: false,
        }
    }
}

/// The status of one produced output.
pub enum Status {
    Written,
    Indexed,
    Skipped,
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match self {
            Status::Written => "written",
            Status::Indexed => "indexed",
            Status::Skipped => "skipped",
        }
    }
}

/// `(dir, stem, fmt, comp)` split of a database path (mirrors `db_index._split_db_name`).
fn split_db_name(path: &Path) -> (PathBuf, String, String, String) {
    let dir = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let base = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    let mut parts: Vec<&str> = base.split('.').collect();
    let mut comp = String::new();
    if parts.len() >= 2
        && COMPRESSION_EXTS.contains(&parts.last().unwrap().to_ascii_lowercase().as_str())
    {
        comp = parts.pop().unwrap().to_string();
    }
    let (fmt, stem) = if parts.len() >= 2 {
        let fmt = parts.pop().unwrap().to_string();
        (fmt, parts.join("."))
    } else {
        (
            String::new(),
            parts.first().copied().unwrap_or("").to_string(),
        )
    };
    (dir, stem, fmt, comp)
}

/// Copy bytes `[start, end)` of `src` to `dst` (streamed).
fn copy_byte_range(src: &Path, dst: &Path, start: u64, end: u64) -> io::Result<()> {
    let mut fh = File::open(src)?;
    use std::io::Seek;
    fh.seek(io::SeekFrom::Start(start))?;
    let mut out = File::create(dst)?;
    let mut remaining = (end - start) as usize;
    let mut buf = vec![0u8; COPY_CHUNK.min(remaining.max(1))];
    while remaining > 0 {
        let want = remaining.min(buf.len());
        let n = fh.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
        remaining -= n;
    }
    out.flush()
}

/// Decompress a BGZF input to an uncompressed temp file (streamed).
fn decompress_to_temp(input: &Path, tmp: &Path) -> io::Result<()> {
    let path = input
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "non-UTF8 path"))?;
    let mut reader = io_bgzf::open_at(path, 0, true)?;
    let mut out = File::create(tmp)?;
    let mut buf = vec![0u8; COPY_CHUNK];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.write_all(&buf[..n])?;
    }
    out.flush()
}

/// Compress `src` to `dst` as standard BGZF (streamed).
fn compress_to_bgzf(src: &Path, dst: &Path, level: u8) -> io::Result<()> {
    let cl = bgzf::CompressionLevel::new(level).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("bad compression level {level}: {e:?}"),
        )
    })?;
    let out = File::create(dst)?;
    let mut writer = bgzf::Writer::new(io::BufWriter::new(out), cl);
    let mut fh = File::open(src)?;
    let mut buf = vec![0u8; COPY_CHUNK];
    loop {
        let n = fh.read(&mut buf)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n])?;
    }
    let mut w = writer.finish()?;
    w.flush()
}

/// Split `num_records` into at most `n_shards` contiguous groups balanced by count.
/// Returns `(start_index, count)` per shard.
fn record_boundaries(num_records: usize, n_shards: usize) -> Vec<(usize, usize)> {
    let n_shards = n_shards.clamp(1, num_records.max(1));
    let base = num_records / n_shards;
    let extra = num_records % n_shards;
    let mut out = Vec::new();
    let mut start = 0;
    for s in 0..n_shards {
        let count = base + if s < extra { 1 } else { 0 };
        out.push((start, count));
        start += count;
    }
    out
}

/// Write a `.didx` for `path` from a fresh scan of it (any existing index is ignored,
/// and replaced atomically).
fn write_index_for(path: &Path, filetype: FileType, out_bgzf: bool) -> Result<PathBuf, String> {
    let compression = if out_bgzf {
        Compression::Bgzf
    } else {
        Compression::None
    };
    let name = path.to_str().ok_or("non-UTF8 output path")?;
    let (offsets, cds_counts) = scan_offsets_uncached(name, filetype, compression)
        .map_err(|e| format!("scanning offsets of {name}: {e}"))?;
    didx::write_index(path, &offsets, &cds_counts, filetype, compression)
        .map_err(|e| format!("writing index for {}: {e}", path.display()))
}

/// Temporary files beside an output, removed on drop unless they were renamed away.
struct Staged(Vec<PathBuf>);

impl Drop for Staged {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// One planned output file.
struct Task {
    /// Source of record bytes (the working file), and the byte range to copy.
    working: PathBuf,
    byte_start: u64,
    byte_end: u64,
    final_path: PathBuf,
    out_bgzf: bool,
    needs_write: bool,
    do_index: bool,
    filetype: FileType,
}

/// Plan the outputs for one input. Returns `(tasks, working_temp_or_None)`.
fn plan_input(
    input_path: &Path,
    opts: &FormatDbOptions,
    tmp_dir: &Path,
    log: &mut dyn Write,
) -> Result<(Vec<Task>, Option<PathBuf>), String> {
    let filetype = detect_filetype(input_path).ok_or_else(|| {
        format!(
            "Input '{}' is not a GenBank or FASTA file.",
            input_path.display()
        )
    })?;
    let in_comp = detect_compression(input_path);
    if in_comp == Compression::Gzip {
        return Err(format!(
            "Input '{}' is plain (non-BGZF) gzip, which is not seekable. Decompress it or recompress as BGZF.",
            input_path.display()
        ));
    }

    let out_dir: PathBuf = match &opts.output_dir {
        Some(d) => PathBuf::from(d),
        None => {
            let p = input_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_default();
            if p.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                p
            }
        }
    };
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("creating output dir: {e}"))?;

    let base = match &opts.name {
        Some(name) => {
            let (_d, _stem, fmt, comp) = split_db_name(input_path);
            let mut new_basename = if fmt.is_empty() {
                name.clone()
            } else {
                format!("{name}.{fmt}")
            };
            if !comp.is_empty() {
                new_basename = format!("{new_basename}.{comp}");
            }
            out_dir.join(new_basename)
        }
        None => out_dir.join(input_path.file_name().unwrap_or_default()),
    };

    // Preserve BGZF; uncompressed inputs become BGZF only with --compress.
    let out_bgzf = opts.compress || in_comp == Compression::Bgzf;

    // Fast path: a single output that is the input file itself (no shard, no
    // format/location change) just gets an index in place -- no copy, no scan.
    if opts.shards.is_none() {
        let final_path = shard_path(&base, None, Some(out_bgzf));
        let same = final_path == input_path
            || std::fs::canonicalize(&final_path).ok() == std::fs::canonicalize(input_path).ok();
        // Same file only if no recompression is needed (uncompressed input staying
        // uncompressed, or a BGZF input staying BGZF).
        let recompress = out_bgzf && in_comp != Compression::Bgzf;
        if same && !recompress {
            return Ok((
                vec![Task {
                    working: input_path.to_path_buf(),
                    byte_start: 0,
                    byte_end: 0,
                    final_path,
                    out_bgzf,
                    needs_write: false,
                    do_index: opts.index,
                    filetype,
                }],
                None,
            ));
        }
    }

    // We copy record bytes -> need an uncompressed working file.
    let (working, working_temp): (PathBuf, Option<PathBuf>) = if in_comp == Compression::None {
        (input_path.to_path_buf(), None)
    } else {
        let tmp = tmp_dir.join("working.plain");
        writeln!(
            log,
            "Decompressing {} -> temporary working file",
            input_path.display()
        )
        .ok();
        decompress_to_temp(input_path, &tmp).map_err(|e| format!("decompressing input: {e}"))?;
        (tmp.clone(), Some(tmp))
    };
    let working_size = std::fs::metadata(&working)
        .map_err(|e| e.to_string())?
        .len();

    let mut tasks = Vec::new();
    match opts.shards {
        None => {
            let final_path = shard_path(&base, None, Some(out_bgzf));
            tasks.push(Task {
                working: working.clone(),
                byte_start: 0,
                byte_end: working_size,
                final_path,
                out_bgzf,
                needs_write: true,
                do_index: opts.index,
                filetype,
            });
        }
        Some(shards) => {
            if shards < 1 {
                return Err("--shards must be >= 1".into());
            }
            let working_str = working.to_str().ok_or("non-UTF8 working path")?;
            let (offsets, _cds) = scan_offsets_uncached(working_str, filetype, Compression::None)
                .map_err(|e| format!("scanning offsets of {working_str}: {e}"))?;
            let num_records = offsets.len();
            if num_records == 0 {
                return Err(format!(
                    "No sequence records found in '{}'.",
                    input_path.display()
                ));
            }
            let boundaries = record_boundaries(num_records, shards);
            if boundaries.len() < shards {
                crate::log::warn(format_args!(
                    "'{}' has only {} records; producing {} shard(s) instead of {}.",
                    input_path.display(),
                    num_records,
                    boundaries.len(),
                    shards
                ));
            }
            for (i, (start_idx, count)) in boundaries.iter().enumerate() {
                let b_start = offsets[*start_idx];
                let end_idx = start_idx + count;
                let b_end = if end_idx < num_records {
                    offsets[end_idx]
                } else {
                    working_size
                };
                let final_path = shard_path(&base, Some(i), Some(out_bgzf));
                tasks.push(Task {
                    working: working.clone(),
                    byte_start: b_start,
                    byte_end: b_end,
                    final_path,
                    out_bgzf,
                    needs_write: true,
                    do_index: opts.index,
                    filetype,
                });
            }
            // Warn if shards would sit alongside the original with the same stem.
            let (_d, in_stem, _f, _c) = split_db_name(input_path);
            let same_name = opts.name.as_deref().map(|n| n == in_stem).unwrap_or(true);
            let in_dir = input_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("."));
            let in_dir = if in_dir.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                in_dir
            };
            if same_name && out_dir == in_dir && input_path.exists() {
                crate::log::warn(format_args!(
                    "Wrote shards alongside the original '{}'. Remove the original (or use \
                     --output_dir) so domain_search does not see both the unsharded file and its \
                     shards.",
                    input_path.display()
                ));
            }
        }
    }
    Ok((tasks, working_temp))
}

/// Execute one planned task: write the file (if needed) and its index (if requested).
fn run_task(task: &Task, opts: &FormatDbOptions, log: &mut dyn Write) -> Result<Status, String> {
    let index_path = didx::index_path_for(&task.final_path);
    if !opts.force && task.final_path.exists() && (!task.do_index || index_path.exists()) {
        return Ok(Status::Skipped);
    }

    if task.needs_write {
        // Write the shard (and its index) under hidden temporary names in the output
        // directory, which no shard pattern matches, and rename them into place: the
        // index first, so a new shard never appears without it, then the shard. A shard
        // therefore only appears complete, and a replaced one gets a new inode, so a
        // search already reading the old one finishes normally (upstream's mkstemp +
        // os.replace). The rename keeps the size and mtime the index fingerprints.
        let tmp = crate::io::temp_path_beside(&task.final_path);
        let mut staged = Staged(vec![tmp.clone()]);
        if task.out_bgzf {
            // Copy the byte range to a plain temp beside the output, then BGZF it.
            let plain_tmp = tmp.with_extension("plain.tmp");
            staged.0.push(plain_tmp.clone());
            copy_byte_range(&task.working, &plain_tmp, task.byte_start, task.byte_end)
                .map_err(|e| format!("copying range: {e}"))?;
            compress_to_bgzf(&plain_tmp, &tmp, opts.level)
                .map_err(|e| format!("compressing {}: {e}", task.final_path.display()))?;
            let _ = std::fs::remove_file(&plain_tmp);
        } else {
            copy_byte_range(&task.working, &tmp, task.byte_start, task.byte_end)
                .map_err(|e| format!("copying range: {e}"))?;
        }
        let rename = |from: &Path, to: &Path| {
            std::fs::rename(from, to)
                .map_err(|e| format!("moving {} into place: {e}", to.display()))
        };
        if task.do_index {
            let tmp_index = write_index_for(&tmp, task.filetype, task.out_bgzf)?;
            staged.0.push(tmp_index.clone());
            rename(&tmp_index, &index_path)?;
        }
        rename(&tmp, &task.final_path)?;
    } else if task.do_index {
        write_index_for(&task.final_path, task.filetype, task.out_bgzf)?;
    }
    let status = if task.needs_write {
        Status::Written
    } else {
        Status::Indexed
    };
    writeln!(log, "{}\t{}", status.as_str(), task.final_path.display()).ok();
    Ok(status)
}

/// Shard / compress / index `inputs`. Returns the list of final output paths.
pub fn format_db(
    inputs: &[String],
    opts: &FormatDbOptions,
    log: &mut dyn Write,
) -> Result<Vec<PathBuf>, String> {
    if opts.shards.is_none() && !opts.compress && !opts.index {
        return Err(
            "Nothing to do: request at least one of --shards, --compress, or --index.".into(),
        );
    }
    if opts.name.is_some() {
        if inputs.len() > 1 {
            return Err(
                "--name cannot be used with multiple inputs (each would collide on the same name)."
                    .into(),
            );
        }
        if opts.shards.is_none() && !opts.compress {
            crate::log::warn(
                "--name on an index-only run copies the input to the new name before indexing \
                 (an index alone cannot rename a database); --name is intended for \
                 --shards/--compress.",
            );
        }
    }

    let tmp = crate::tools::external::TempDir::new("domainatoredux_format_db")
        .map_err(|e| e.to_string())?;

    let mut out_paths = Vec::new();
    for input in inputs {
        let input_path = PathBuf::from(input);
        // Give each input its own temp working subdir to avoid clobbering.
        let sub = tmp.path().join(sanitize(input));
        std::fs::create_dir_all(&sub).map_err(|e| format!("creating temp subdir: {e}"))?;
        let (tasks, _working_temp) = plan_input(&input_path, opts, &sub, log)?;
        for task in &tasks {
            run_task(task, opts, log)?;
            out_paths.push(task.final_path.clone());
        }
    }
    Ok(out_paths)
}

/// A filesystem-safe token from a path (for temp subdir naming).
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_balance_records() {
        // 10 records into 3 shards -> 4,3,3.
        let b = record_boundaries(10, 3);
        assert_eq!(b, vec![(0, 4), (4, 3), (7, 3)]);
    }

    #[test]
    fn boundaries_clamp_to_record_count() {
        // 2 records, 4 shards -> only 2 shards.
        let b = record_boundaries(2, 4);
        assert_eq!(b, vec![(0, 1), (1, 1)]);
    }

    #[test]
    fn boundaries_single_shard() {
        assert_eq!(record_boundaries(5, 1), vec![(0, 5)]);
    }

    #[test]
    fn split_name_handles_compression_suffix() {
        let (_d, stem, fmt, comp) = split_db_name(Path::new("mydb.0.gb.bgz"));
        assert_eq!(stem, "mydb.0");
        assert_eq!(fmt, "gb");
        assert_eq!(comp, "bgz");
        let (_d, stem, fmt, comp) = split_db_name(Path::new("mydb.gb"));
        assert_eq!(stem, "mydb");
        assert_eq!(fmt, "gb");
        assert_eq!(comp, "");
    }
    /// A rebuild replaces each shard by rename, never by truncating it in place: a
    /// search holding the old shard open keeps reading the old bytes in full, the new
    /// shard and its index agree, and no temporary file is left for the shard glob.
    #[test]
    fn a_forced_rebuild_replaces_shards_without_touching_open_ones() {
        use crate::io::shards::resolve_database_shards;
        let dir = std::env::temp_dir().join(format!("domainator_format_db_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../test/data/FeSOD_20.gb");
        let input = dir.join("db.gb");
        std::fs::copy(&fixture, &input).unwrap();
        let out = dir.join("out");
        let opts = FormatDbOptions {
            shards: Some(2),
            compress: true,
            index: true,
            output_dir: Some(out.to_string_lossy().into_owned()),
            ..FormatDbOptions::default()
        };
        let input_arg = [input.to_string_lossy().into_owned()];
        let first = format_db(&input_arg, &opts, &mut io::sink()).unwrap();
        assert_eq!(first.len(), 2);
        let old_bytes = std::fs::read(&first[0]).unwrap();
        let mut held = File::open(&first[0]).unwrap();

        // Rebuild from a smaller input, with --force.
        let text = std::fs::read_to_string(&fixture).unwrap();
        let half: String = text.split_inclusive("//\n").take(6).collect();
        std::fs::write(&input, half).unwrap();
        let opts = FormatDbOptions {
            force: true,
            ..opts
        };
        let second = format_db(&input_arg, &opts, &mut io::sink()).unwrap();
        assert_eq!(second, first);

        let mut still = Vec::new();
        held.read_to_end(&mut still).unwrap();
        assert_eq!(
            still, old_bytes,
            "an open shard must not be truncated by a rebuild"
        );
        assert_ne!(std::fs::read(&first[0]).unwrap(), old_bytes);

        let names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.contains("tmp")), "{names:?}");
        let db = out.join("db.gb.bgz");
        let shards = resolve_database_shards(&db).unwrap();
        assert_eq!(shards, second);
        let mut records = 0;
        for shard in &shards {
            let (offsets, _) = didx::read_index(shard, FileType::Genbank, Compression::Bgzf)
                .unwrap()
                .unwrap();
            records += offsets.len();
        }
        assert_eq!(records, 6);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A temporary file being written beside the shards is not one of them.
    #[test]
    fn hidden_temporaries_are_not_shards() {
        use crate::io::shards::resolve_database_shards;
        let dir =
            std::env::temp_dir().join(format!("domainator_format_db_tmp_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.0.gb.bgz"), b"").unwrap();
        std::fs::write(crate::io::temp_path_beside(&dir.join("x.1.gb.bgz")), b"").unwrap();
        let shards = resolve_database_shards(&dir.join("x.gb.bgz")).unwrap();
        assert_eq!(shards, vec![dir.join("x.0.gb.bgz")]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
