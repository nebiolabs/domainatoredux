# Installation

There are two things you might install, and they are independent: the **command-line
tool** (`domainatoredux`) and the **desktop app** (Domainatoredux GUI). Neither needs Python,
conda, or an external HMMER — the HMMER3 implementation is pure Rust and built in.

## Prebuilt downloads

Grab them from the
[Releases page](https://github.com/nebiolabs/domainatoredux/releases).

| Platform | Desktop app | Command-line tool |
| --- | --- | --- |
| macOS (Apple silicon) | `.dmg` | `domainatoredux` |
| Windows (x64) | NSIS `.exe` installer | `domainatoredux.exe` |
| Linux (x64) | `.deb` and `.AppImage` | `domainatoredux` |

Releases are cut manually, so the newest commits may be ahead of the newest release.
Release numbers are this project's own and do not follow the Python package's. Each
release says which Python `domainator` version it matches, and so does `domainatoredux
--version`, e.g. `domainatoredux 1.0.0-rc.1 (parity with Python domainator 1.0.0-rc.2 @ 67bad08)`.
Intel macOS is not built; build from source there.

The command-line download is a single self-contained binary. The Linux one runs on any
x86-64 distribution with glibc 2.28 or newer (RHEL/Rocky/Alma 8, Ubuntu 20.04, Debian 10
and later); on anything older, build it from source. Put it somewhere on your `PATH` and,
on macOS or Linux, mark it executable:

```bash
chmod +x domainatoredux
./domainatoredux --version
```

The macOS and Linux bundles are **unsigned**. On macOS, Gatekeeper will refuse the first
launch — open it once via right-click → Open, or clear the quarantine flag:

```bash
xattr -dr com.apple.quarantine /Applications/Domainatoredux.app
```

### GPU builds

`find_features --tmbed` can run its protein language model on a GPU. The macOS downloads
use the Apple GPU. For an NVIDIA GPU on Linux or Windows, a release may also offer `-cuda`
downloads of both the app and the command-line tool. They need:

- an NVIDIA driver for CUDA 12.4 or newer (driver 550 or later; `nvidia-smi` shows the
  CUDA version your driver supports), and
- the CUDA 12 libraries cuBLAS and cuRAND, most easily by installing the
  [CUDA Toolkit](https://developer.nvidia.com/cuda-downloads) 12.x. On Windows its
  installer puts them on `PATH`. On Linux they must be on the library path
  (`LD_LIBRARY_PATH`, or `module load cuda/12` on a cluster).

A `-cuda` download loads these libraries when it first needs the GPU, so it starts and
runs on any machine. Without them it runs on the CPU and prints a warning that names the
missing library. `domainatoredux doctor` lists the CUDA libraries it found and reports
whether the build you have can use the GPU.

## Build the command-line tool from source

You need [Rust](https://rustup.rs/) **1.94 or newer** (the workspace sets
`rust-version = "1.94"`). Nothing else.

```bash
git clone https://github.com/nebiolabs/domainatoredux.git
cd domainatoredux
cargo build --release -p domainator-cli
./target/release/domainatoredux --help
```

The binary lands at `target/release/domainatoredux`. Copy it onto your `PATH`:

```bash
install -m755 target/release/domainatoredux ~/.local/bin/domainatoredux
```

Use `--release` for any real work — the debug build is several times slower.

## Build the desktop app from source

You additionally need [Node.js](https://nodejs.org/) 20.19+ or 22.12+ (vite 8's floor; CI uses 24) and the
[Tauri v2 prerequisites](https://tauri.app/start/prerequisites/) for your platform.

```bash
cd apps/domainatoredux-gui
npm install
npm run tauri dev     # run it
npm run tauri build   # produce an installer for your platform
```

On Debian/Ubuntu the system libraries are:

```bash
sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev \
  libjavascriptcoregtk-4.1-dev librsvg2-dev patchelf build-essential file
```

## Optional external programs

Most of Domainator is self-contained. A few tools call out to an external clustering,
alignment or gene-calling program, and **only** if you ask for that algorithm:

| Program | Needed by | Not needed if |
| --- | --- | --- |
| `diamond` | `seq_dist --algorithm diamond_*` (the default), `deduplicate_genbank --algorithm diamond` | you use `seq_dist --algorithm hmmer` |
| `cdhit`, or C cd-hit's `cd-hit`/`cd-hit-est` | `deduplicate_genbank --algorithm cd-hit` (the default) | you use `deduplicate_genbank --algorithm hash` |
| `usearch` | `deduplicate_genbank --algorithm usearch` | as above |
| `orphos` (0.4.0 or newer) | `gene_call`, and `domainate`/`domain_search --gene_call` | your contigs already have CDS annotations |
| `cmbuild` | `infernal_build` | you only search with existing `.cm` models (that needs nothing) |
| `foldseek` | the `structure_*` tools (the default `--algorithm`) | you do not work with structures, or use reseek |
| `reseek` | the `structure_*` tools with `--algorithm reseek` | you use foldseek |
| `datasets` (the NCBI Datasets CLI) | `db_download` of an `ncbi_*` database with `--download_backend datasets` (the default) | you use `--download_backend direct`, or download UniProt |

Each is found by the first of these that works:

1. the tool's own path option (`--diamond_path`, `--cdhit_path`, `--usearch_path`;
   `gene_call --orphos_path`; `infernal_build --cmbuild_path`; `--foldseek_path`,
   `--reseek_path`; `db_download --datasets_path`);
2. the environment variable `$DOMAINATOREDUX_DIAMOND`, `$DOMAINATOREDUX_CDHIT`,
   `$DOMAINATOREDUX_USEARCH`, `$DOMAINATOREDUX_ORPHOS`, `$DOMAINATOREDUX_CMBUILD`,
   `$DOMAINATOREDUX_FOLDSEEK`, `$DOMAINATOREDUX_RESEEK` or `$DOMAINATOREDUX_DATASETS`;
3. a bundled copy at `external/<program>/` next to the executable;
4. the bare program name on your `PATH`. For cd-hit that is the Rust port's `cdhit`, then
   C cd-hit's `cd-hit` (bioconda's `cd-hit` package), which runs `cd-hit-est` from the
   same directory for nucleotide input.

**The desktop app can download all of them but `cmbuild` and `datasets` for you**, where a
build exists for your platform — open the *External tools* panel in
the left sidebar and click Download. It installs them into the app's data directory and
sets the environment variables, so the command-line tool finds them too if you launch it
from the same session. See [the desktop app](gui.md#external-tools).

cd-hit, usearch and Orphos are GPL-licensed. Domainator never bundles or links them; it
downloads them at your request and runs them as separate processes.

Orphos publishes no build for Intel Macs. There, install it with `cargo install
orphos-cli` and point `$DOMAINATOREDUX_ORPHOS` at the result (usually `~/.cargo/bin/orphos`).

foldseek installs with `conda install -c conda-forge -c bioconda foldseek` or from its
releases; `domainate --foldseek` needs release 10 or later. reseek's v3.01 release
binaries (Linux and Windows) need a CPU with AVX-512; elsewhere, build it from source.
The desktop app downloads both where it can. Like the programs above, both run as
separate processes and are never bundled.

`datasets` installs with `conda install -c conda-forge ncbi-datasets-cli` or from NCBI's
site. `db_download` unpacks its packages itself, so no `unzip` is needed.

`cmbuild` is Infernal's (`conda install -c bioconda infernal`, Linux and macOS). On any
platform, Windows included, `cargo install infernox` builds a Rust port whose
`infernox-cmbuild` writes the same models; point `$DOMAINATOREDUX_CMBUILD` at it. It reads
Stockholm, Pfam and SELEX alignments, not aligned FASTA. Searching with `.cm` models needs
neither: that is built in.

## Verify the installation

```bash
domainatoredux --version
domainatoredux --help          # lists all 36 tools
domainatoredux domainate --help
domainatoredux doctor          # which optional external programs it can find
```

If a `diamond_*` or clustering run fails with a "not found" error, see
[Troubleshooting](troubleshooting.md).
