<script lang="ts">
  import {
    extractRecords,
    openFileWindow,
    renderViewerHtml,
    resolveRecords,
    saveBinaryFile,
    saveTextFile,
    traceNetworkSources,
  } from "../api";
  import { cached } from "../cache";
  import { validateFilename } from "../filetypes";
  import { ws } from "../state.svelte";
  import type { SourceTrace } from "../types";
  import NameModal from "../NameModal.svelte";

  let { path }: { path: string } = $props();

  // The page as rendered (what "Save as HTML page" writes), and as the iframe shows it.
  let page = $state<string | null>(null);
  let html = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let selectedLabels = $state<string[]>([]);
  let naming = $state(false);
  let openNote = $state<string | null>(null);
  let trace = $state<SourceTrace | null>(null);
  let saveNote = $state<{ text: string; err: boolean } | null>(null);
  let frame = $state<HTMLIFrameElement | null>(null);

  // This viewer can be shown inside the main shell (has a ViewerDock for tabs) or
  // in a detached pop-out window (no dock). `ws.openPath` only shows in the shell,
  // so from a detached window we open the extract in its own new window instead.
  const inDetachedWindow = new URLSearchParams(location.search).has("view");

  // The bundle's file name without its extension.
  const stem = $derived(
    (path.split(/[\\/]/).pop() ?? "selection").replace(/\.(dvb|dsnv|ssnv|ssnview)(\.gz)?$/i, ""),
  );
  // Default extract name: "<bundle-stem>_selection.gb".
  const defaultName = $derived(`${stem}_selection.gb`);

  // Provenance-derived extraction state. Extraction resolves node labels only
  // within the sequence files this network was built from — so it's correct even
  // if the workspace holds unrelated records with the same id, and disabled when
  // those files are unknown or gone.
  const missingSources = $derived((trace?.sources ?? []).filter((s) => !s.present));
  const staleSources = $derived((trace?.sources ?? []).filter((s) => s.present && s.modified_since));
  const canExtract = $derived(
    !!trace && trace.has_provenance && missingSources.length === 0 && !!ws.currentDir,
  );
  const extractBlockReason = $derived.by(() => {
    if (!trace) return "Checking source files…";
    if (!trace.has_provenance)
      return "Extraction unavailable: this network has no recorded source sequences.";
    if (missingSources.length > 0)
      return `Extraction unavailable: source file${missingSources.length === 1 ? "" : "s"} missing (${missingSources
        .map((s) => s.path.split(/[\\/]/).pop())
        .join(", ")}).`;
    return null;
  });

  // The viewer runs in its own document inside a sandboxed iframe, so the app's
  // global `contextmenu` suppressor (main.ts) never sees events from it and the raw
  // WebKit/WebView2 menu would still appear there.
  //
  // Injected here rather than in the shell itself:
  // `crates/domainator-core/assets/viewer_shell.html` is a *generated*
  // artifact, regenerated from Python by `scripts/regen_viewer_shell.py` and
  // covered by a drift check, so it must never be hand-edited. Post-processing the
  // string in this consumer leaves those bytes untouched.
  //
  // The app cannot put its own menu inside the iframe either (a host-document menu
  // cannot read the iframe's selection or rows); the viewer's own selection UI
  // covers what one would offer.
  const IFRAME_CONTEXTMENU_SUPPRESSOR = `<script>
document.addEventListener("contextmenu", function (e) {
  var t = e.target;
  if (t && t.closest && t.closest("input, textarea, [contenteditable]")) return;
  var s = document.getSelection();
  if (s && !s.isCollapsed) return;
  e.preventDefault();
});
<\/script>`;

  // Every save/export button in the shell (Save session, Save extraction, Export
  // view PNG/SVG, Export table, the chart and legend exports) ends in a click on a
  // detached `<a download href="blob:...">`. In a sandboxed iframe WebKitGTK,
  // WKWebView and WebView2 all drop that click, and Tauri has no download handler
  // to catch it, so the buttons did nothing. This script intercepts the click and
  // hands the Blob to the host, which shows a native Save dialog (`onMessage`).
  //
  // The Blob is captured when the URL is created because the shell revokes the URL
  // right after `click()`, before an async fetch of it could run. Injected here
  // for the same reason as the suppressor above: the shell is a generated file.
  const IFRAME_DOWNLOAD_BRIDGE = `<script>
(function () {
  var blobs = new Map();
  var create = URL.createObjectURL.bind(URL);
  var revoke = URL.revokeObjectURL.bind(URL);
  URL.createObjectURL = function (obj) {
    var url = create(obj);
    if (obj instanceof Blob) blobs.set(url, obj);
    return url;
  };
  URL.revokeObjectURL = function (url) {
    blobs.delete(url);
    revoke(url);
  };
  var click = HTMLAnchorElement.prototype.click;
  HTMLAnchorElement.prototype.click = function () {
    var blob = this.hasAttribute("download") ? blobs.get(this.href) : undefined;
    if (!blob) return click.call(this);
    window.parent.postMessage(
      { type: "domainator-viewer-download", filename: this.download || "download", blob: blob },
      "*",
    );
  };
})();
<\/script>`;

  $effect(() => {
    void path;
    loading = true;
    error = null;
    page = null;
    html = null;
    selectedLabels = [];
    openNote = null;
    naming = false;
    trace = null;
    saveNote = null;
    cached(`viewerhtml:${path}`, () => renderViewerHtml(path))
      .then((h) => {
        page = h;
        html = h + IFRAME_CONTEXTMENU_SUPPRESSOR + IFRAME_DOWNLOAD_BRIDGE;
      })
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
    // Trace provenance in parallel (cached — it only reads small JSON sidecars).
    cached(`ssnsrc:${path}`, () => traceNetworkSources(path))
      .then((t) => (trace = t))
      .catch(() => (trace = { has_provenance: false, sources: [] }));
  });

  // Listen for messages from the embedded viewer iframe: node selection (the
  // shell's GUI host hook, shared by the network, tree and projection views) and
  // downloads (IFRAME_DOWNLOAD_BRIDGE). Only this viewer's own iframe counts — other
  // viewer tabs stay mounted and post to the same window.
  $effect(() => {
    const onMessage = (e: MessageEvent) => {
      if (!frame || e.source !== frame.contentWindow) return;
      if (e.data?.type === "domainator-viewer-selection" && Array.isArray(e.data.labels)) {
        selectedLabels = e.data.labels as string[];
        openNote = null;
      } else if (e.data?.type === "domainator-viewer-download" && e.data.blob instanceof Blob) {
        saveDownload(String(e.data.filename), e.data.blob as Blob);
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  });

  const FILTER_NAMES: Record<string, string> = {
    png: "PNG image",
    svg: "SVG image",
    tsv: "TSV table",
    dvb: "Domainator Viewer bundle",
    dsnv: "SSN viewer bundle",
    json: "JSON",
    html: "HTML page",
    // The tree view exports its selection as Newick and as FASTA, aligned or not.
    nwk: "Newick tree",
    newick: "Newick tree",
    afa: "Aligned FASTA",
    fasta: "FASTA",
  };

  // Base64 in chunks: a spread of a multi-megabyte array into
  // `String.fromCharCode` overflows the call stack.
  function toBase64(bytes: Uint8Array): string {
    let bin = "";
    for (let i = 0; i < bytes.length; i += 0x8000) {
      bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    }
    return btoa(bin);
  }

  // Show a native Save dialog for a file the shell tried to download.
  async function saveDownload(filename: string, blob: Blob) {
    const dot = filename.lastIndexOf(".");
    const ext = dot > 0 ? filename.slice(dot + 1).toLowerCase() : "";
    const filter = FILTER_NAMES[ext] ?? (ext ? ext.toUpperCase() : "File");
    const isText =
      blob.type.startsWith("text/") ||
      blob.type === "image/svg+xml" ||
      ["tsv", "nwk", "newick", "afa", "fasta", "html"].includes(ext);
    try {
      const saved = isText
        ? await saveTextFile(filename, ext, filter, await blob.text())
        : await saveBinaryFile(filename, ext, filter, toBase64(new Uint8Array(await blob.arrayBuffer())));
      saveNote = saved ? { text: `Saved ${saved}`, err: false } : null;
    } catch (err) {
      saveNote = { text: `Saving ${filename} failed: ${err}`, err: true };
    }
  }

  // The page with its bundle embedded, as `domainatoredux viewer -i <bundle>` writes it.
  async function savePage() {
    if (!page) return;
    try {
      const saved = await saveTextFile(`${stem}.html`, "html", "HTML page", page);
      saveNote = saved ? { text: `Saved ${saved}`, err: false } : null;
    } catch (err) {
      saveNote = { text: `Saving ${stem}.html failed: ${err}`, err: true };
    }
  }

  // Resolve every selected node label within the network's traced source files
  // (one cached scan), then write those records to a real workspace file `dir/name`.
  async function doExtract(name: string) {
    const dir = ws.currentDir;
    if (!dir) throw new Error("no workspace folder to write into");
    const files = (trace?.sources ?? []).filter((s) => s.present).map((s) => s.path);
    if (files.length === 0) throw new Error("no source files available for this network");
    const resolved = await resolveRecords(files, selectedLabels);
    const recs: { path: string; contig_index: number }[] = [];
    for (const r of resolved) {
      if (r) recs.push({ path: r.path, contig_index: r.contig_index });
    }
    if (recs.length === 0) {
      throw new Error("none of the selected nodes were found in the source files");
    }
    const missing = resolved.length - recs.length;
    const outPath = await extractRecords(dir, name, recs);
    // Show the extract: a new window when detached, a viewer tab in the main shell.
    if (inDetachedWindow) {
      await openFileWindow(outPath, name);
    } else {
      await ws.refreshFolder();
      ws.openPath(outPath, "genbank");
    }
    naming = false;
    openNote = `Extracted ${recs.length} sequence${recs.length === 1 ? "" : "s"} to ${name}${
      missing ? ` (${missing} not found)` : ""
    }.`;
  }
</script>

<div class="viewer">
  {#if loading}
    <div class="msg">Loading the viewer…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if html}
    {#if selectedLabels.length > 0}
      <div class="selbar">
        <span>{selectedLabels.length} node{selectedLabels.length === 1 ? "" : "s"} selected</span>
        <button onclick={() => (naming = true)} disabled={!canExtract}>Extract selected →</button>
        {#if extractBlockReason}
          <span class="note warn">{extractBlockReason}</span>
        {:else if staleSources.length > 0}
          <span class="note warn" title={staleSources.map((s) => s.path).join("\n")}>
            ⚠ {staleSources.length} source file{staleSources.length === 1 ? "" : "s"} changed since this
            bundle was built — extracted sequences may not match.
          </span>
        {/if}
        {#if openNote}<span class="note">{openNote}</span>{/if}
      </div>
    {/if}
    <div class="toolbar">
      <button
        onclick={() => savePage()}
        title="Save this viewer, with its bundle, as one HTML file that opens in any browser"
      >Save as HTML page…</button>
    </div>
    {#if saveNote}
      <div class="selbar">
        <span class="note" class:err={saveNote.err}>{saveNote.text}</span>
        <button onclick={() => (saveNote = null)}>Dismiss</button>
      </div>
    {/if}
    <!-- The bundle is embedded in the shell, which self-loads via DecompressionStream.
         `allow-modals` lets the shell's own alert/confirm prompts show. -->
    <iframe
      bind:this={frame}
      title="Domainator Viewer"
      srcdoc={html}
      sandbox="allow-scripts allow-same-origin allow-modals"
    ></iframe>
  {:else}
    <!-- No HTML and no error: don't leave the pane blank with no explanation. -->
    <div class="msg">Nothing to show for this bundle.</div>
  {/if}

  {#if naming}
    <NameModal
      title="Extract {selectedLabels.length} selected sequence{selectedLabels.length === 1 ? '' : 's'}"
      defaultName={defaultName}
      submitLabel="Extract"
      validate={validateFilename}
      onsubmit={(name) => doExtract(name)}
      oncancel={() => (naming = false)}
    />
  {/if}
</div>

<style>
  .viewer {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .toolbar {
    display: flex;
    justify-content: flex-end;
    padding: 3px 12px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
  }
  .selbar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 5px 12px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    font-size: 12.5px;
  }
  .selbar .note {
    color: var(--text-dim);
  }
  .selbar .note.err {
    color: #e5484d;
  }
  .selbar .note.warn {
    color: #b7791f;
  }
  iframe {
    flex: 1;
    width: 100%;
    height: 100%;
    border: none;
    background: #fff;
  }
  .msg {
    padding: 24px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
