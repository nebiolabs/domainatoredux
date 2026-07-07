<script lang="ts">
  import {
    extractRecords,
    openFileWindow,
    renderSsnHtml,
    resolveRecords,
    traceNetworkSources,
  } from "../api";
  import { cached } from "../cache";
  import { validateFilename } from "../filetypes";
  import { ws } from "../state.svelte";
  import type { SourceTrace } from "../types";
  import NameModal from "../NameModal.svelte";

  let { path }: { path: string } = $props();

  let html = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let selectedLabels = $state<string[]>([]);
  let naming = $state(false);
  let openNote = $state<string | null>(null);
  let trace = $state<SourceTrace | null>(null);

  // This viewer can be shown inside the main shell (has a ViewerDock for tabs) or
  // in a detached pop-out window (no dock). `ws.openPath` only shows in the shell,
  // so from a detached window we open the extract in its own new window instead.
  const inDetachedWindow = new URLSearchParams(location.search).has("view");

  // Default extract name: "<ssn-stem>_selection.gb".
  const defaultName = $derived.by(() => {
    const base = (path.split(/[\\/]/).pop() ?? "selection").replace(/\.(ssnv|ssnview)(\.gz)?$/i, "");
    return `${base}_selection.gb`;
  });

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

  $effect(() => {
    void path;
    loading = true;
    error = null;
    html = null;
    selectedLabels = [];
    openNote = null;
    naming = false;
    trace = null;
    cached(`ssnhtml:${path}`, () => renderSsnHtml(path))
      .then((h) => (html = h))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
    // Trace provenance in parallel (cached — it only reads small JSON sidecars).
    cached(`ssnsrc:${path}`, () => traceNetworkSources(path))
      .then((t) => (trace = t))
      .catch(() => (trace = { has_provenance: false, sources: [] }));
  });

  // Listen for node-selection messages from the embedded SSN viewer iframe.
  $effect(() => {
    const onMessage = (e: MessageEvent) => {
      if (e.data?.type === "domainator-ssn-selection" && Array.isArray(e.data.labels)) {
        selectedLabels = e.data.labels as string[];
        openNote = null;
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  });

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

<div class="ssn">
  {#if loading}
    <div class="msg">Loading SSN viewer…</div>
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
            network was built — extracted sequences may not match.
          </span>
        {/if}
        {#if openNote}<span class="note">{openNote}</span>{/if}
      </div>
    {/if}
    <!-- The bundle is embedded in the shell, which self-loads via DecompressionStream. -->
    <iframe title="SSN viewer" srcdoc={html} sandbox="allow-scripts allow-same-origin"></iframe>
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
  .ssn {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
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
