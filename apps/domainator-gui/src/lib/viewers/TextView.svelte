<script lang="ts">
  import { readTextFile, writeTextFile } from "../api";
  import { ws } from "../state.svelte";

  let { path }: { path: string } = $props();

  let text = $state("");
  let original = "";
  // The file's own line terminator. The buffer here is always LF -- a `<textarea>`
  // normalizes its value to LF whatever you put in it -- so without carrying this
  // back to the save, opening a CRLF file and pressing Save rewrote every line
  // ending in it.
  let eol = "\n";
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

  // File → Save, and the "Save" button on the unsaved-changes guard, both call
  // this hook. It used to be a counter on the `ui` store that this component
  // `$effect`ed on as `saveRequest > 0 && dirty` -- which short-circuited past
  // `dirty` while the counter was 0, leaving it untracked, then picked it up as a
  // dependency on the first real save. From then on every keystroke re-ran the
  // effect and silently wrote the file. A direct call has no such state.
  $effect(() => ws.registerEditor(path, save));

  async function load() {
    loading = true;
    error = null;
    try {
      const f = await readTextFile(path);
      text = f.text;
      original = f.text;
      eol = f.eol;
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
      await writeTextFile(path, text, eol);
      original = text;
      msg = "saved";
      setTimeout(() => (msg = null), 2000);
    } catch (e) {
      msg = "save failed: " + String(e);
    } finally {
      saving = false;
    }
  }

  // Counted with `indexOf` rather than `split("\n").length`: splitting allocates
  // an array of every line in the file on every keystroke, which is what made a
  // multi-megabyte file crawl.
  const lineCount = $derived.by(() => {
    if (!text) return 0;
    let n = 1;
    for (let i = text.indexOf("\n"); i !== -1; i = text.indexOf("\n", i + 1)) n++;
    return n;
  });
  const eolLabel = $derived(eol === "\r\n" ? "CRLF" : eol === "\r" ? "CR" : "LF");
  // Formatting a byte count is cheap; it tells the user why a huge file is slow.
  const sizeLabel = $derived.by(() => {
    const mb = text.length / (1024 * 1024);
    return mb >= 1 ? `${mb.toFixed(1)} MB` : `${Math.round(text.length / 1024)} kB`;
  });
</script>

<div class="tx">
  {#if loading}
    <div class="msg">Loading…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else}
    <div class="toolbar">
      <button class="save" onclick={save} disabled={!dirty || saving}>{saving ? "Saving…" : "Save"}</button>
      <span class="dim">{lineCount.toLocaleString()} lines · {sizeLabel} · {eolLabel}</span>
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
