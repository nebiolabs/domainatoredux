<script lang="ts">
  import { ws } from "./state.svelte";
  import type { FileEntry } from "./types";

  const TYPE_LABEL: Record<string, string> = {
    genbank: "GenBank",
    fasta: "FASTA",
    hmm: "HMM",
    hdf5: "Matrix",
    cm: "CM",
    ssn_viewer: "SSN",
  };

  function fmtSize(n: number): string {
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }

  function activate(f: FileEntry) {
    ws.openFile(f);
  }
</script>

<div class="table" role="table">
  <div class="head" role="row">
    <div class="c-name" role="columnheader">Name</div>
    <div class="c-type" role="columnheader">Type</div>
    <div class="c-desc" role="columnheader">Description</div>
    <div class="c-size" role="columnheader">Size</div>
  </div>
  {#if ws.loadingFiles}
    <div class="note">Loading…</div>
  {:else if !ws.selectedFolder}
    <div class="note">Select a folder.</div>
  {:else if ws.files.length === 0}
    <div class="note">No recognized files in this folder.</div>
  {:else}
    {#each ws.files as f (f.path)}
      <div
        class="row"
        class:active={ws.activeTabId === f.path}
        role="row"
        tabindex="0"
        ondblclick={() => activate(f)}
        onclick={() => activate(f)}
        onkeydown={(e) => e.key === "Enter" && activate(f)}
      >
        <div class="c-name" role="cell" title={f.name}>{f.name}</div>
        <div class="c-type" role="cell">
          <span class="badge badge-{f.file_type}">{TYPE_LABEL[f.file_type] ?? f.file_type}</span>
        </div>
        <div class="c-desc" role="cell" title={f.description}>{f.description}</div>
        <div class="c-size" role="cell">{fmtSize(f.size)}</div>
      </div>
    {/each}
  {/if}
</div>

<style>
  .table {
    display: flex;
    flex-direction: column;
    min-width: 0;
    font-size: 12.5px;
  }
  .head,
  .row {
    display: grid;
    grid-template-columns: minmax(160px, 2fr) 90px minmax(160px, 3fr) 80px;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
  }
  .head {
    position: sticky;
    top: 0;
    height: 28px;
    background: var(--bg-panel);
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
    font-weight: 600;
    z-index: 1;
  }
  .row {
    height: 26px;
    cursor: default;
    user-select: none;
    border-bottom: 1px solid transparent;
  }
  .row:hover {
    background: var(--row-hover);
  }
  .row.active {
    background: var(--row-selected);
  }
  .c-name,
  .c-desc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .c-size {
    text-align: right;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .badge {
    display: inline-block;
    padding: 1px 7px;
    border-radius: 10px;
    font-size: 11px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .badge-genbank {
    color: #06842b;
    border-color: #06842b55;
  }
  .badge-fasta {
    color: #2f6feb;
    border-color: #2f6feb55;
  }
  .badge-ssn_viewer {
    color: #b5179e;
    border-color: #b5179e55;
  }
  .note {
    padding: 16px 12px;
    color: var(--text-dim);
  }
</style>
