<script lang="ts">
  import { readTextFile, writeTextFile } from "../api";
  import { ws } from "../state.svelte";
  import { ui } from "../ui.svelte";

  let { path }: { path: string } = $props();

  let text = $state("");
  let original = "";
  let loading = $state(true);
  let saving = $state(false);
  let error = $state<string | null>(null);
  let msg = $state<string | null>(null);
  const dirty = $derived(text !== original);

  // Report unsaved state so the File menu's Save item enables and the window
  // close guard can see it. Cleared on teardown -- a closed tab is not unsaved
  // work the user can still act on.
  $effect(() => {
    if (dirty) ws.markDirty(path);
    else ws.clearDirty(path);
    return () => ws.clearDirty(path);
  });

  // File → Save.
  $effect(() => {
    if (ui.saveRequest > 0 && dirty) void save();
  });

  async function load() {
    loading = true;
    error = null;
    try {
      const t = await readTextFile(path);
      text = t;
      original = t;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void path;
    load();
  });

  async function save() {
    saving = true;
    msg = null;
    try {
      await writeTextFile(path, text);
      original = text;
      msg = "saved";
      setTimeout(() => (msg = null), 2000);
    } catch (e) {
      msg = "save failed: " + String(e);
    } finally {
      saving = false;
    }
  }

  const lineCount = $derived(text ? text.split("\n").length : 0);
</script>

<div class="tx">
  {#if loading}
    <div class="msg">Loading…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else}
    <div class="toolbar">
      <button class="save" onclick={save} disabled={!dirty || saving}>{saving ? "Saving…" : "Save"}</button>
      <span class="dim">{lineCount.toLocaleString()} lines</span>
      {#if dirty}<span class="dot" title="Unsaved changes">●</span>{/if}
      {#if msg}<span class="dim">{msg}</span>{/if}
    </div>
    <textarea class="body" bind:value={text} spellcheck="false"></textarea>
  {/if}
</div>

<style>
  .tx {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
  }
  .save {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .save:disabled {
    opacity: 0.5;
    cursor: default;
    background: var(--bg-elevated);
    color: var(--text);
    border-color: var(--border);
  }
  .dim {
    color: var(--text-dim);
    font-size: 12px;
  }
  .dot {
    color: var(--accent);
  }
  .body {
    flex: 1;
    min-height: 0;
    resize: none;
    border: none;
    outline: none;
    padding: 8px 12px;
    background: var(--bg);
    color: var(--text);
    font-family: var(--mono);
    font-size: 12.5px;
    white-space: pre;
    overflow: auto;
    tab-size: 4;
  }
  .msg {
    padding: 24px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
