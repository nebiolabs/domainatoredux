//! Shared helpers for tools that shell out to external binaries (DIAMOND for
//! `seq_dist`/`deduplicate_genbank`, USEARCH for `deduplicate_genbank`, cmbuild for
//! `infernal_build`, foldseek and reseek for the structure tools): a self-cleaning temp
//! directory, a binary locator (env var → bundled → PATH), a FASTA writer for
//! `(id, residues)` records, and a friendly "not found" error.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    let _ = command;
}

/// Locate an external executable `name` (e.g. `"diamond"`): first the `env_vars`
/// in order, then a bundled `external/<bundled_dir>/<name>` searched upward from
/// the running executable and in the current directory, then bare `name` on PATH.
pub(crate) fn find_binary(name: &str, env_vars: &[&str], bundled_dir: &str) -> PathBuf {
    let exe_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    for var in env_vars {
        if let Ok(p) = std::env::var(var) {
            if !p.is_empty() {
                return PathBuf::from(p);
            }
        }
    }
    let bundled = |base: &Path| base.join("external").join(bundled_dir).join(&exe_name);
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent();
        while let Some(d) = dir {
            let cand = bundled(d);
            if cand.is_file() {
                return cand;
            }
            dir = d.parent();
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let cand = bundled(&cwd);
        if cand.is_file() {
            return cand;
        }
    }
    PathBuf::from(exe_name)
}

/// Turn a "binary not found" spawn failure into an actionable error with `hint`.
pub(crate) fn spawn_error(e: std::io::Error, bin: &Path, hint: &str) -> std::io::Error {
    if e.kind() == std::io::ErrorKind::NotFound {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "could not run '{}': executable not found. {hint}",
                bin.display()
            ),
        )
    } else {
        e
    }
}

/// Write `(id, residues)` records as FASTA (60-column wrap).
pub(crate) fn write_fasta_ids(records: &[(String, Vec<u8>)], path: &Path) -> std::io::Result<()> {
    let mut w = std::io::BufWriter::new(std::fs::File::create(path)?);
    for (id, seq) in records {
        writeln!(w, ">{id}")?;
        for chunk in seq.chunks(60) {
            w.write_all(chunk)?;
            w.write_all(b"\n")?;
        }
    }
    w.flush()
}

/// A uniquely-named temporary directory removed (recursively) on drop. Avoids a
/// `tempfile` dependency; uniqueness comes from pid + nanosecond clock + a retry
/// counter.
pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new(prefix: &str) -> std::io::Result<Self> {
        Self::new_in(None, prefix)
    }

    /// A temp directory under `parent` (a `--tmp_dir`), or the system's when `None`.
    pub(crate) fn new_in(parent: Option<&Path>, prefix: &str) -> std::io::Result<Self> {
        let base = parent.map_or_else(std::env::temp_dir, Path::to_path_buf);
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for n in 0u32.. {
            let cand = base.join(format!("{prefix}_{pid}_{nanos}_{n}"));
            match std::fs::create_dir(&cand) {
                Ok(()) => return Ok(TempDir { path: cand }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        unreachable!("u32 range exhausted creating a temp dir")
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// An optional external program some tools run, and how it is found.
pub struct ExternalProgram {
    /// The executable's name.
    pub name: &'static str,
    /// Environment variables naming it, checked in order.
    pub env_vars: &'static [&'static str],
    /// A bundled copy lives in `external/<bundled_dir>/`.
    pub bundled_dir: &'static str,
    /// The commands that need it.
    pub needed_by: &'static str,
    /// How to get it.
    pub install: &'static str,
}

impl ExternalProgram {
    /// What a tool runs: an environment variable's path, else a bundled copy, else the
    /// bare name, which the OS looks up on `PATH`.
    pub fn find(&self) -> PathBuf {
        find_binary(self.name, self.env_vars, self.bundled_dir)
    }

    /// The file a tool would run, if there is one.
    pub fn locate(&self) -> Option<PathBuf> {
        let path = self.find();
        if path.components().count() > 1 {
            return path.is_file().then_some(path);
        }
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(&path))
            .find(|candidate| candidate.is_file())
    }
}

/// Every external program, in the order `doctor` lists them.
pub const EXTERNAL_PROGRAMS: &[ExternalProgram] = &[
    DIAMOND, CDHIT, USEARCH, ORPHOS, CMBUILD, FOLDSEEK, RESEEK, DATASETS,
];

pub const DIAMOND: ExternalProgram = ExternalProgram {
    name: super::deduplicate_genbank::DIAMOND_BIN_NAME,
    env_vars: &[super::deduplicate_genbank::DIAMOND_ENV_VAR, "DIAMOND"],
    bundled_dir: "diamond",
    needed_by: "seq_dist --algorithm diamond_* (the default), deduplicate_genbank --algorithm \
                diamond",
    install: "conda install -c bioconda diamond, or https://github.com/bbuchfink/diamond/releases",
};

pub const CDHIT: ExternalProgram = ExternalProgram {
    name: super::deduplicate_genbank::CDHIT_BIN_NAME,
    env_vars: &[super::deduplicate_genbank::CDHIT_ENV_VAR, "CDHIT"],
    bundled_dir: "cdhit",
    needed_by: "deduplicate_genbank --algorithm cd-hit (the default)",
    install: "https://github.com/seanrjohnson-neb/cdhit-rust/releases (or C cd-hit, as \
              cd-hit/cd-hit-est)",
};

pub const USEARCH: ExternalProgram = ExternalProgram {
    name: super::deduplicate_genbank::USEARCH_BIN_NAME,
    env_vars: &[super::deduplicate_genbank::USEARCH_ENV_VAR, "USEARCH"],
    bundled_dir: "usearch",
    needed_by: "deduplicate_genbank --algorithm usearch",
    install: "https://github.com/rcedgar/usearch12/releases",
};

pub const ORPHOS: ExternalProgram = ExternalProgram {
    name: super::gene_call::ORPHOS_BIN_NAME,
    env_vars: &[super::gene_call::ORPHOS_ENV_VAR, "ORPHOS"],
    bundled_dir: "orphos",
    needed_by: "gene_call, domainate/domain_search/rotate_contigs --gene_call",
    install: "https://github.com/FullHuman/orphos/releases (0.4.0 or newer), or cargo install \
              orphos-cli",
};

pub const CMBUILD: ExternalProgram = ExternalProgram {
    name: super::infernal_build::CMBUILD_BIN_NAME,
    env_vars: &[super::infernal_build::CMBUILD_ENV_VAR, "CMBUILD"],
    bundled_dir: "infernal",
    needed_by: "infernal_build",
    install: "conda install -c bioconda infernal, or cargo install infernox (then point \
              $DOMAINATOREDUX_CMBUILD at its infernox-cmbuild)",
};

pub const FOLDSEEK: ExternalProgram = ExternalProgram {
    name: super::structure_lib::FOLDSEEK_BIN_NAME,
    env_vars: &[super::structure_lib::FOLDSEEK_ENV_VAR, "FOLDSEEK"],
    bundled_dir: "foldseek",
    needed_by: "structure_to_genbank, structure_domainate, structure_search, structure_dist",
    install: "conda install -c conda-forge -c bioconda foldseek, or \
              https://github.com/steineggerlab/foldseek/releases",
};

pub const DATASETS: ExternalProgram = ExternalProgram {
    name: "datasets",
    env_vars: &["DOMAINATOREDUX_DATASETS"],
    bundled_dir: "datasets",
    needed_by: "db_download --download_backend datasets (the default for the ncbi databases)",
    install: "conda install -c conda-forge ncbi-datasets-cli, or \
              https://www.ncbi.nlm.nih.gov/datasets/docs/v2/command-line-tools/download-and-install/",
};

pub const RESEEK: ExternalProgram = ExternalProgram {
    name: super::structure_lib::RESEEK_BIN_NAME,
    env_vars: &[super::structure_lib::RESEEK_ENV_VAR, "RESEEK"],
    bundled_dir: "reseek",
    needed_by: "structure_domainate, structure_search, structure_dist (--algorithm reseek)",
    install: "build it from https://github.com/rcedgar/reseek (the v3.01 release binary \
              requires AVX-512)",
};

/// The executable for `program` (upstream `resolve_binary`): the user's explicit path,
/// which must be an executable file or a name on PATH, or else the program found the
/// usual way (environment variable, bundled copy, PATH).
pub fn resolve_binary(
    program: &ExternalProgram,
    path: Option<&str>,
    hint: &str,
) -> std::io::Result<PathBuf> {
    if let Some(path) = path {
        if is_executable_file(Path::new(path)) || on_path(path).is_some() {
            return Ok(PathBuf::from(path));
        }
        return Err(std::io::Error::other(format!(
            "'{path}' is not an executable file."
        )));
    }
    program.locate().ok_or_else(|| {
        std::io::Error::other(format!(
            "Could not find '{name}' on PATH. {hint} Alternatively pass an explicit path with \
             --{name}_path.",
            name = program.name
        ))
    })
}

/// A file the user may run: on Unix, one with an execute bit set.
pub(crate) fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn on_path(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}
