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
--version`, e.g. `domainatoredux 1.0.0 (parity with Python domainator 1.0.0rc1 @ f18dc15)`.
Intel macOS is not built; build from source there.

The command-line download is a single self-contained binary. Put it somewhere on your
`PATH` and, on macOS or Linux, mark it executable:

```bash
chmod +x domainatoredux
./domainatoredux --version
```

The macOS and Linux bundles are **unsigned**. On macOS, Gatekeeper will refuse the first
launch — open it once via right-click → Open, or clear the quarantine flag:

```bash
xattr -dr com.apple.quarantine /Applications/Domainatoredux.app
```

## Build the command-line tool from source

You need [Rust](https://rustup.rs/) **1.89 or newer** (the workspace sets
`rust-version = "1.89"`). Nothing else.

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
cd apps/domainator-gui
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
| `cdhit` | `deduplicate_genbank --algorithm cd-hit` (the default) | you use `deduplicate_genbank --algorithm hash` |
| `usearch` | `deduplicate_genbank --algorithm usearch` | as above |
| `orphos` (0.4.0 or newer) | `gene_call`, and `domainate`/`domain_search --gene_call` | your contigs already have CDS annotations |

Each is found by the first of these that works:

1. the tool's own path option (`--diamond_path`, `--cdhit_path`, `--usearch_path`;
   `gene_call --orphos_path`);
2. the environment variable `$DOMAINATOREDUX_DIAMOND`, `$DOMAINATOREDUX_CDHIT`,
   `$DOMAINATOREDUX_USEARCH` or `$DOMAINATOREDUX_ORPHOS`;
3. a bundled copy at `external/<program>/` next to the executable;
4. the bare program name on your `PATH`.

**The desktop app can download all of them for you** — open the *External tools* panel in
the left sidebar and click Download. It installs them into the app's data directory and
sets the environment variables, so the command-line tool finds them too if you launch it
from the same session. See [the desktop app](gui.md#external-tools).

cd-hit, usearch and Orphos are GPL-licensed. Domainator never bundles or links them; it
downloads them at your request and runs them as separate processes.

Orphos publishes no build for Intel Macs. There, install it with `cargo install
orphos-cli` and point `$DOMAINATOREDUX_ORPHOS` at the result (usually `~/.cargo/bin/orphos`).

## Verify the installation

```bash
domainatoredux --version
domainatoredux --help          # lists all 35 tools
domainatoredux domainate --help
domainatoredux doctor          # which optional external programs it can find
```

If a `diamond_*` or clustering run fails with a "not found" error, see
[Troubleshooting](troubleshooting.md).
