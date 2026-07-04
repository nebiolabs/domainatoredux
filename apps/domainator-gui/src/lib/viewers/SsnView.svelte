<script lang="ts">
  import { openSequenceWindow, renderSsnHtml, resolveRecord } from "../api";
  import { cached } from "../cache";
  import { ws } from "../state.svelte";

  let { path }: { path: string } = $props();

  let html = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let selectedLabels = $state<string[]>([]);
  let opening = $state(false);
  let openNote = $state<string | null>(null);

  const MAX_OPEN = 12;

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

  // Resolve each selected node label to its source sequence and open it,
  // focusing the last one's contig.
  async function openSelected() {
    if (!ws.root || selectedLabels.length === 0) return;
    opening = true;
    openNote = null;
    let opened = 0;
    let missing = 0;
    try {
      for (const label of selectedLabels.slice(0, MAX_OPEN)) {
        const rec = await resolveRecord(ws.root, label);
        if (rec) {
          await openSequenceWindow(rec.path, rec.contig_index, rec.contig_id || label);
          opened++;
        } else {
          missing++;
        }
      }
      const extra = selectedLabels.length > MAX_OPEN ? ` (first ${MAX_OPEN})` : "";
      openNote = `Opened ${opened} window${opened === 1 ? "" : "s"}${missing ? `, ${missing} not found in workspace` : ""}${extra}.`;
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
