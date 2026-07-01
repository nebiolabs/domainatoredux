# Domainator GenBank Summary WASM

A browser-only version of the GenBank Summary app. It runs the summarization
logic in WebAssembly, reads a user-selected local GenBank file with the browser
File API, previews TSV output, and downloads the TSV without installing a native
app.

This is intended as a short-term distribution path for environments where native
macOS apps cannot run until signing/notarization is resolved.

## Development

From this directory:

```bash
npm install
npm run dev
```

Build a static site:

```bash
npm run build
```

The build emits a file-openable static site in `dist/`. The WebAssembly module,
JavaScript, and CSS are all packed into `dist/index.html`. The `dist` directory
can be zipped and shared; recipients can unzip it and open `index.html` directly
without running a local server.

Create the shareable zip:

```bash
npm run package:zip
```

This writes `domainator-genbank-summary-wasm.zip` next to this README.

## Scope

This WASM app is intentionally narrower than the Tauri app. It supports plain
GenBank text files selected in the browser. It does not use native file dialogs,
does not read paths directly, and does not handle compressed files.