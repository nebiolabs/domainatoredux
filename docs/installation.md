# Installation

There are two things you might install, and they are independent: the **command-line
tool** (`domainator`) and the **desktop app** (Domainator GUI). Neither needs Python,
conda, or an external HMMER — the HMMER3 implementation is pure Rust and built in.

## Prebuilt downloads

Grab them from the
[Releases page](https://github.com/nebiolabs/domainatoredux/releases).

| Platform | Desktop app | Command-line tool |
| --- | --- | --- |
| macOS (Apple silicon) | `.dmg` | `domainator` |
| Windows (x64) | NSIS `.exe` installer | `domainator.exe` |
| Linux (x64) | `.deb` and `.AppImage` | `domainator` |

Releases are cut manually, so the newest commits may be ahead of the newest release.
Release numbers are this project's own and do not follow the Python package's. Each
release says which Python `domainator` version it matches, and so does `domainator
--version`, e.g. `domainator 1.0.0 (parity with Python domainator 0.11.0 @ 9c70934)`.
Intel macOS is not built; build from source there.

The command-line download is a single self-contained binary. Put it somewhere on your
`PATH` and, on macOS or Linux, mark it executable:

```bash
chmod +x domainator
./domainator --version
```

The macOS and Linux bundles are **unsigned**. On macOS, Gatekeeper will refuse the first
launch — open it once via right-click → Open, or clear the quarantine flag:

```bash
xattr -dr com.apple.quarantine /Applications/Domainator.app
```

## Build the command-line tool from source

You need [Rust](https://rustup.rs/) **1.89 or newer** (the workspace sets
`rust-version = "1.89"`). Nothing else.

```bash
git clone https://github.com/nebiolabs/domainatoredux.git
cd domainatoredux
cargo build --release -p domainator-cli
./target/release/domainator --help
```

The binary lands at `target/release/domainator`. Copy it onto your `PATH`:

```bash
install -m755 target/release/domainator ~/.local/bin/domainator
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
| `diamond` | `seq-dist --algorithm diamond_*` (the default), `deduplicate --algorithm diamond` | you use `seq-dist --algorithm hmmer` |
| `cdhit` | `deduplicate --algorithm cd-hit` (the default) | you use `deduplicate --algorithm hash` |
| `usearch` | `deduplicate --algorithm usearch` | as above |
| `orphos` (0.4.0 or newer) | `gene-call`, and `domainate`/`domain-search --gene_call` | your contigs already have CDS annotations |

Each is found by the first of these that works:

1. the tool's own `--bin_path` flag;
2. the environment variable `$DOMAINATOR_DIAMOND`, `$DOMAINATOR_CDHIT`,
   `$DOMAINATOR_USEARCH` or `$DOMAINATOR_ORPHOS`;
3. a bundled copy at `external/<program>/` next to the executable;
4. the bare program name on your `PATH`.

**The desktop app can download all of them for you** — open the *External tools* panel in
the left sidebar and click Download. It installs them into the app's data directory and
sets the environment variables, so the command-line tool finds them too if you launch it
from the same session. See [the desktop app](gui.md#external-tools).

cd-hit, usearch and Orphos are GPL-licensed. Domainator never bundles or links them; it
downloads them at your request and runs them as separate processes.

Orphos publishes no build for Intel Macs. There, install it with `cargo install
orphos-cli` and point `$DOMAINATOR_ORPHOS` at the result (usually `~/.cargo/bin/orphos`).

## Verify the installation

```bash
domainator --version
domainator --help          # lists all 35 tools
domainator domainate --help
```

If a `diamond_*` or clustering run fails with a "not found" error, see
[Troubleshooting](troubleshooting.md).
