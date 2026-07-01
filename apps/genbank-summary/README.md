# Domainator GenBank Summary

A small Tauri desktop app for summarizing a GenBank file into a TSV table.

The app groups contig records by either `taxid` or `Definition`, optionally
filters records to those containing all requested domain names, and ignores
domain hits whose evalue is higher than the selected threshold. Output has three
columns: selected grouping key, `count`, and comma-separated record names.

## Development

From this directory:

```bash
npm install
npm run tauri dev
```

To validate only the web frontend:

```bash
npm run build
```

To validate the shared Rust summary logic from the repository root:

```bash
cargo test -p domainator-core genbank_summary
```

## Linux prerequisites

Tauri needs the platform WebView and windowing development packages. On Debian or
Ubuntu-like systems, install the Tauri Linux prerequisites before running the
desktop app build/check commands:

```bash
sudo apt install -y \
	pkg-config \
	libwayland-client0 libwayland-dev \
	libdbus-1-dev libglib2.0-dev \
	libgtk-3-dev libcairo2-dev libgdk-pixbuf-2.0-dev libatk1.0-dev \
	libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev
```

From the repository root, the Rust-side Tauri check is:

```bash
cargo check -p domainator-genbank-summary
```

## Release builds

Build native bundles on the target operating system:

```bash
npm run tauri build
```

For Windows GNU cross-builds from Linux, install the Rust target and packaging
tools:

```bash
rustup target add x86_64-pc-windows-gnu
sudo apt install -y mingw-w64 nsis
npm run tauri build -- --target x86_64-pc-windows-gnu
```

This produces the unsigned executable and NSIS installer under the repository
`target/x86_64-pc-windows-gnu/release/` directory. Tauri warns that
cross-platform compilation is experimental and that Windows signing is skipped
from non-Windows hosts.

For macOS ARM, install the Rust target:

```bash
rustup target add aarch64-apple-darwin
npm run tauri build -- --target aarch64-apple-darwin
```

On Linux, this currently reaches the macOS Objective-C build step and fails
without an Apple compiler/SDK toolchain. Use an Apple Silicon macOS machine or
runner for the release bundle. Signing and notarization are not configured in
this first app scaffold.