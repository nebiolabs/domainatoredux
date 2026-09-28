<script lang="ts">
  // "Move to Folder…": pick a workspace folder for the selected files, or make a
  // new one first. The tree is drawn fully expanded as an indented list -- a
  // picker, not a browser, so there is nothing to collapse.
  import { createFolder } from "./api";
  import { ws } from "./state.svelte";
  import type { FolderNode } from "./types";

  let {
    paths,
    onclose,
  }: {
    paths: string[];
    onclose: () => void;
  } = $props();

  type Row = { path: string; name: string; depth: number };
  function flatten(node: FolderNode, depth: number, out: Row[]): Row[] {
    out.push({ path: node.path, name: node.name, depth });
    for (const c of node.children) flatten(c, depth + 1, out);
    return out;
  }
  const rows = $derived(ws.tree ? flatten(ws.tree, 0, []) : []);
  // The files all come from the table, i.e. the selected folder: moving there is a no-op.
  const here = $derived(ws.selectedFolder);

  let target = $state<string | null>(null);
  let busy = $state(false);
  let err = $state<string | null>(null);
  let naming = $state(false);
  let newName = $state("");

  const title = $derived(paths.length === 1 ? `Move “${paths[0].split(/[\\/]/).pop()}”` : `Move ${paths.length} files`);

  async function makeFolder() {
    const parent = target ?? ws.root;
    const name = newName.trim();
    if (!parent || !name) return;
    err = null;
    try {
      // Not `ws.createSubfolder`: that also selects the new folder, which would
      // swap the file table out from under the files being moved.
      const created = await createFolder(parent, name);
      await ws.refreshTree();
      target = created;
      naming = false;
      newName = "";
    } catch (e) {
      err = String(e);
    }
  }

  async function move() {
    if (!target || busy) return;
    busy = true;
    await ws.moveFiles(paths, target);
    busy = false;
    onclose();
  }
</script>

<div class="overlay" role="presentation" onclick={onclose}>
  <div
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === "Escape" && !naming && onclose()}
  >
    <h3>{title}</h3>
    <div class="list" role="listbox" aria-label="Destination folder">
      {#each rows as r (r.path)}
        <button
          class="folder"
          class:chosen={target === r.path}
          role="option"
          aria-selected={target === r.path}
          disabled={r.path === here}
          title={r.path === here ? "The files are already here" : r.path}
          style="padding-left: {r.depth * 14 + 8}px"
          onclick={() => (target = r.path)}
          ondblclick={() => {
            target = r.path;
            void move();
          }}
        >
          <span class="icon">🗀</span>{r.name}
        </button>
      {/each}
    </div>
    {#if naming}
      <div class="newrow">
        <!-- svelte-ignore a11y_autofocus -->
        <input
          type="text"
          placeholder="New folder name"
          bind:value={newName}
          autofocus
          onkeydown={(e) => {
            if (e.key === "Enter") void makeFolder();
            if (e.key === "Escape") {
              e.stopPropagation();
              naming = false;
            }
          }}
        />
        <button onclick={makeFolder} disabled={!newName.trim()}>Create</button>
      </div>
    {/if}
    {#if err}<div class="err">{err}</div>{/if}
    <div class="actions">
      <button class="left" onclick={() => (naming = !naming)} title="Create a folder inside the highlighted one">
        ＋ New folder…
      </button>
      <button onclick={onclose} disabled={busy}>Cancel</button>
      <button class="primary" onclick={move} disabled={!target || busy}>{busy ? "Moving…" : "Move"}</button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 120;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .modal {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4);
    padding: 16px;
    width: min(420px, 90vw);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: 14px;
  }
  .list {
    max-height: 50vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg);
    padding: 4px 0;
  }
  .folder {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    height: 24px;
    border: none;
    border-radius: 0;
    background: none;
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
    text-align: left;
    white-space: nowrap;
    cursor: default;
  }
  .folder:hover:not(:disabled) {
    background: var(--row-hover);
  }
  .folder.chosen {
    background: var(--row-selected);
  }
  .folder:disabled {
    color: var(--text-dim);
    opacity: 0.6;
  }
  .icon {
    font-size: 12px;
    opacity: 0.8;
  }
  .newrow {
    display: flex;
    gap: 6px;
  }
  .newrow input {
    flex: 1;
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg);
    color: var(--text);
  }
  .err {
    color: #e5484d;
    font-size: 12px;
    user-select: text;
    white-space: pre-wrap;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .left {
    margin-right: auto;
  }
  .primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
</style>
