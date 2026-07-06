<script lang="ts">
  import { extractRecords, renderSsnHtml, resolveRecords } from "../api";
  import { cached } from "../cache";
  import { ws } from "../state.svelte";

  let { path }: { path: string } = $props();

  let html = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let selectedLabels = $state<string[]>([]);
  let opening = $state(false);
  let openNote = $state<string | null>(null);

  $effect(() => {
    void path;
    loading = true;
    error = null;
    html = null;
    selectedLabels = [];
    openNote = null;
    cached(`ssnhtml:${path}`, () => renderSsnHtml(path))
      .then((h) => (html = h))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
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

  // Resolve every selected node label to its source record, then open ONE combined
  // sequence view containing just those records (with contig / enum / summary tabs).
  async function openSelected() {
    if (!ws.root || selectedLabels.length === 0) return;
    opening = true;
    openNote = null;
    try {
      // One workspace scan resolves the whole selection (was N scans → apparent hang).
      const resolved = await resolveRecords(ws.root, selectedLabels);
      const recs: { path: string; contig_index: number }[] = [];
      for (const r of resolved) {
        if (r) recs.push({ path: r.path, contig_index: r.contig_index });
      }
      const missing = resolved.length - recs.length;
      if (recs.length === 0) {
        openNote = "None of the selected nodes were found in the workspace.";
        return;
      }
      const outPath = await extractRecords(ws.root, recs);
      ws.openPath(outPath, "genbank");
      openNote = `Opened a view of ${recs.length} sequence${recs.length === 1 ? "" : "s"}${
        missing ? `, ${missing} not found` : ""
      }.`;
    } catch (e) {
      openNote = String(e);
    } finally {
      opening = false;
    }
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
        <button onclick={openSelected} disabled={opening || !ws.root}>
          {opening ? "Opening…" : "Open sequence(s) →"}
        </button>
        {#if openNote}<span class="note">{openNote}</span>{/if}
      </div>
    {/if}
    <!-- The bundle is embedded in the shell, which self-loads via DecompressionStream. -->
    <iframe title="SSN viewer" srcdoc={html} sandbox="allow-scripts allow-same-origin"></iframe>
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
