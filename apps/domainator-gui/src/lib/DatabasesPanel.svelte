<script lang="ts">
  import { pickFile } from "./api";
  import { ws } from "./state.svelte";
  import { ui } from "./ui.svelte";
  import type { DatabaseEntry } from "./types";

  let expanded = $state(true);

  // Tools menu → expand and clear the request.
  $effect(() => {
    if (ui.focusPanel === "databases") {
      expanded = true;
      ui.focusPanel = null;
    }
  });

  async function add() {
    const path = await pickFile();
    if (path) await ws.addDatabase(path);
  }

  function typeLabel(d: DatabaseEntry): string {
    const mol = d.molecule === "nucleic" ? "nucleic acid" : d.molecule;
    const fmt = d.format === "hmm" ? "hmmer" : d.format;
    return `${mol} ${fmt}`;
  }
</script>

<div class="dbpanel">
  <div class="header">
    <button class="twisty" onclick={() => (expanded = !expanded)} aria-label="Toggle databases">
      {expanded ? "▾" : "▸"}
    </button>
    <span class="title">Databases</span>
    <span class="count">{ws.databases.length}</span>
    <button class="add" onclick={add} disabled={!ws.root} title="Add database…">＋</button>
  </div>
  {#if expanded}
    {#if ws.dbError}<div class="err">{ws.dbError}</div>{/if}
    {#if ws.databases.length === 0}
      <div class="empty">No databases. Add a formatted FASTA/GenBank/HMM set.</div>
    {:else}
      {#each ws.databases as db (db.path)}
        <div class="db" title={db.path}>
          <span class="dot dot-{db.molecule}"></span>
          <span class="dbname">{db.name}</span>
          <span class="dbtype">{typeLabel(db)}{db.shards > 1 ? ` ·${db.shards}` : ""}</span>
          <button class="rm" onclick={() => ws.removeDatabase(db.path)} aria-label="Remove">×</button>
        </div>
      {/each}
    {/if}
  {/if}
</div>

<style>
  .dbpanel {
    border-top: 1px solid var(--border);
    font-size: 12.5px;
  }
  .header {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    background: var(--bg-panel);
  }
  .twisty {
    border: none;
    background: none;
    color: var(--text-dim);
    padding: 0 2px;
    font-size: 10px;
  }
  .twisty:hover {
    background: none;
  }
  .title {
    font-weight: 600;
  }
  .count {
    color: var(--text-dim);
  }
  .add {
    margin-left: auto;
    border: none;
    background: none;
    font-size: 15px;
    line-height: 1;
    color: var(--accent);
    padding: 0 4px;
  }
  .add:hover {
    background: none;
  }
  .empty,
  .err {
    padding: 6px 12px;
    color: var(--text-dim);
    line-height: 1.4;
  }
  .err {
    color: #e5484d;
  }
  .db {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px 3px 12px;
    white-space: nowrap;
  }
  .db:hover {
    background: var(--row-hover);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex: none;
  }
  .dot-protein {
    background: #2f6feb;
  }
  .dot-nucleic {
    background: #06842b;
  }
  .dot-unknown {
    background: var(--text-dim);
  }
  .dbname {
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
  }
  .dbtype {
    color: var(--text-dim);
    font-size: 11px;
  }
  .rm {
    border: none;
    background: none;
    color: var(--text-dim);
    padding: 0 4px;
    font-size: 13px;
  }
  .rm:hover {
    background: none;
    color: #e5484d;
  }
</style>
