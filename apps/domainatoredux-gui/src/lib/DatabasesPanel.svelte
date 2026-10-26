<script lang="ts">
  import { listDatabaseFiles, openFileWindow, pickFile, revealDatabase } from "./api";
  import { ws } from "./state.svelte";
  import { ui } from "./ui.svelte";
  import { flashPanel } from "./panelFlash";
  import { openContextMenu, SEP, type CtxItem } from "./ctxmenu.svelte";
  import { copyText } from "./clipboard";
  import NameModal from "./NameModal.svelte";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import ResizableBody from "./ResizableBody.svelte";
  import PanelSeam from "./PanelSeam.svelte";
  import type { DatabaseRow, FileEntry, FileType } from "./types";

  let expanded = $state(true);
  let helpOpen = $state(false);

  const HINT =
    "Databases are sequence or HMM sets used as tool inputs — usually large, kept " +
    "outside the workspace folders, and shared by every workspace.";

  // Tools menu → expand, outline for a few seconds, and clear the request.
  let panelEl = $state<HTMLDivElement>();
  $effect(() => {
    if (ui.focusPanel === "databases") {
      expanded = true;
      ui.focusPanel = null;
      flashPanel(panelEl);
    }
  });

  async function add() {
    const path = await pickFile();
    if (path) await ws.addDatabase(path);
  }

  function typeLabel(d: DatabaseRow): string {
    if (d.format === "taxonomy") return "NCBI taxonomy";
    const mol = d.molecule === "nucleic" ? "nucleic acid" : d.molecule;
    const fmt = d.format === "hmm" ? "hmmer" : d.format;
    return `${mol} ${fmt}`;
  }

  const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;

  // ── index status ────────────────────────────────────────────────────────────
  // Only user-added databases offer to build one: managed downloads are indexed
  // by the downloader, and an HMM library has nothing to index.
  const needsIndex = (d: DatabaseRow) =>
    !d.managed && d.on_disk && (d.index === "missing" || d.index === "stale");

  const INDEX_HINT: Record<string, string> = {
    missing:
      "No .didx index. Searches still work, but must scan the whole file first; " +
      "an index lets them seek, and gives -Z 0 its target count instantly.",
    stale: "The .didx index no longer matches the data (it changed since indexing). Rebuild it.",
    unindexable:
      "Plain gzip cannot be indexed. Recompress it as BGZF with the format_db tool to index it.",
  };

  // ── context menu: the file table's row menu, for a database ─────────────────
  async function copy(text: string) {
    if (!(await copyText(text))) ws.error = "Could not copy to the clipboard.";
  }

  /// A viewer tab takes a FileEntry. Sharded databases have no single file to
  /// open, so Open is disabled for them.
  function asFile(d: DatabaseRow): FileEntry {
    return {
      // Never a taxonomy: `openable` keeps those rows from opening.
      path: d.path, name: baseName(d.path), file_type: d.format as FileType, description: "",
      size: 0, created_ms: null, modified_ms: null, created_by_job: null,
    };
  }

  let renameFor = $state<DatabaseRow | null>(null);
  let deleteFor = $state<{ db: DatabaseRow; files: string[] } | null>(null);

  async function askDelete(d: DatabaseRow) {
    deleteFor = { db: d, files: await listDatabaseFiles(d.path) };
  }

  /// Every shard of the database; its `.didx` indexes stay behind, since
  /// `format_db` rebuilds them and they are stale once the data is moved.
  async function exportDb(d: DatabaseRow) {
    const files = (await listDatabaseFiles(d.path)).filter((f) => !f.toLowerCase().endsWith(".didx"));
    if (files.length) ui.exportFor = files;
    else ws.error = `No files found for ${baseName(d.path)}.`;
  }

  function rowMenu(e: MouseEvent, d: DatabaseRow) {
    // A taxdump is a tarball no viewer reads.
    const openable = d.on_disk && d.shards <= 1 && d.format !== "taxonomy";
    const items: CtxItem[] = [
      { label: "Open", disabled: !openable, action: () => ws.openFile(asFile(d)) },
      {
        label: "Open in New Window",
        disabled: !openable,
        action: () => openFileWindow(d.path, baseName(d.path)),
      },
      SEP,
      {
        label: "Rename…",
        // The manifest owns a managed download's file name.
        disabled: d.managed || !d.on_disk,
        action: () => {
          renameFor = d;
        },
      },
      { label: "Export…", disabled: !d.on_disk, action: () => exportDb(d) },
      SEP,
      { label: "Copy Name", action: () => copy(baseName(d.path)) },
      { label: "Copy Full Path", action: () => copy(d.path) },
      { label: "Show in File Manager", disabled: !d.on_disk, action: () => revealDatabase(d.path) },
    ];
    if (ws.indexing[d.path]) {
      items.push(SEP, { label: "Cancel Indexing", action: () => ws.cancelDatabaseIndex(d.path) });
    } else if (needsIndex(d)) {
      items.push(SEP, {
        label: d.index === "stale" ? "Rebuild Index" : "Build Index",
        action: () => ws.buildDatabaseIndex(d.path),
      });
    }
    items.push(
      SEP,
      { label: "Remove from List", action: () => ws.removeDatabase(d.path) },
      { label: "Delete…", danger: true, disabled: !d.on_disk, action: () => askDelete(d) },
    );
    openContextMenu(e, items);
  }
</script>

<div class="dbpanel" bind:this={panelEl}>
  <PanelSeam key="databases" active={expanded} />
  <div class="header">
    <button class="twisty" onclick={() => (expanded = !expanded)} aria-label="Toggle databases">
      {expanded ? "▾" : "▸"}
    </button>
    <span class="title" title={HINT}>Databases</span>
    <button
      class="help"
      class:on={helpOpen}
      title={HINT}
      aria-label="What is a database?"
      aria-expanded={helpOpen}
      onclick={() => (helpOpen = !helpOpen)}>?</button
    >
    <span class="count">{ws.databases.length}</span>
    <button class="add" onclick={add} title="Add database…">＋</button>
  </div>
  {#if helpOpen}
    <div class="helpbox" role="note">
      <p>
        A <b>database</b> is a sequence or HMM set you search against or annotate with
        — usually large, often split into shards with a <code>.didx</code> index. It is
        never copied: it stays where it is on disk and is registered by path, and the
        list is shared by <b>every workspace</b>. Databases appear in tool input
        dropdowns alongside the files in the current folder.
      </p>
      <p>
        <b>Regular files</b> live inside a workspace's folders, appear in the file
        table, and belong to that workspace.
      </p>
      <p>
        <b>Remove from List</b> forgets a database and leaves its files alone;
        <b>Delete…</b> removes the files from disk.
      </p>
      <button class="helpclose" onclick={() => (helpOpen = false)}>Got it</button>
    </div>
  {/if}
  {#if expanded}
    <ResizableBody key="databases" defaultMax={220}>
      {#if ws.dbError}<div class="err">{ws.dbError}</div>{/if}
      {#if ws.databases.length === 0}
        <div class="empty">No databases. Add a formatted FASTA/GenBank/HMM set.</div>
      {:else}
        {#each ws.databases as db (db.path)}
          <div class="db" title={db.path} role="presentation" oncontextmenu={(e) => rowMenu(e, db)}>
            <span class="dot dot-{db.molecule}"></span>
            <span class="dbname" class:gone={!db.on_disk}>{db.name}</span>
            {#if !db.on_disk}
              <span class="tag warn" title="None of its files are on disk (moved, deleted, or an unplugged drive)">missing</span>
            {:else if ws.indexing[db.path]}
              <span class="tag">indexing…</span>
              <button class="idx" title="Stop indexing (the partial index is removed)" onclick={() => ws.cancelDatabaseIndex(db.path)}>
                Cancel
              </button>
            {:else if needsIndex(db)}
              <button class="idx" title={INDEX_HINT[db.index]} onclick={() => ws.buildDatabaseIndex(db.path)}>
                {db.index === "stale" ? "Rebuild index" : "Build index"}
              </button>
            {:else if db.index === "unindexable" && !db.managed}
              <span class="tag" title={INDEX_HINT.unindexable}>gzip</span>
            {/if}
            <span class="dbtype">{typeLabel(db)}{db.shards > 1 ? ` ·${db.shards}` : ""}</span>
            <button class="rm" onclick={() => ws.removeDatabase(db.path)} title="Remove from list (keeps the files)" aria-label="Remove from list">×</button>
          </div>
        {/each}
      {/if}
    </ResizableBody>
  {/if}
</div>

{#if renameFor}
  {@const d = renameFor}
  <NameModal
    title="Rename database"
    defaultName={baseName(d.path)}
    submitLabel="Rename"
    onsubmit={async (name) => {
      await ws.renameDatabase(d.path, name);
      renameFor = null;
    }}
    oncancel={() => (renameFor = null)}
  />
{/if}

{#if deleteFor}
  {@const pending = deleteFor}
  <ConfirmDialog
    title="Delete {pending.db.name} from disk?"
    body={`This permanently deletes ${pending.files.length === 1 ? "this file" : `these ${pending.files.length} files`}, and removes the database from every workspace:\n\n${pending.files.join("\n")}\n\nTo stop listing it but keep the files, use Remove from List instead.`}
    confirmLabel="Delete"
    danger
    onconfirm={async () => {
      await ws.deleteDatabase(pending.db.path);
      deleteFor = null;
    }}
    oncancel={() => (deleteFor = null)}
  />
{/if}

<style>
  .dbpanel {
    border-top: 1px solid var(--border);
    font-size: 12.5px;
    flex: none;
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
    padding: 0;
    width: 18px;
    font-size: 14px;
    line-height: 1;
    text-align: center;
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
  .help {
    border: 1px solid var(--border);
    border-radius: 50%;
    background: none;
    color: var(--text-dim);
    width: 15px;
    height: 15px;
    padding: 0;
    font-size: 10px;
    line-height: 13px;
    text-align: center;
    flex: none;
  }
  .help:hover,
  .help.on {
    color: var(--accent);
    border-color: var(--accent);
    background: none;
  }
  .helpbox {
    margin: 4px 8px 6px;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-elevated);
    line-height: 1.45;
  }
  .helpbox p {
    margin: 0 0 6px;
  }
  .helpclose {
    font-size: 11.5px;
    padding: 2px 8px;
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
  .gone {
    color: var(--text-dim);
    text-decoration: line-through;
  }
  .tag {
    color: var(--text-dim);
    font-size: 10.5px;
    border: 1px solid var(--border);
    border-radius: 3px;
    padding: 0 4px;
    flex: none;
  }
  .tag.warn {
    color: #e5484d;
    border-color: #e5484d;
  }
  .idx {
    font-size: 10.5px;
    padding: 0 5px;
    line-height: 16px;
    flex: none;
  }
  .helpbox code {
    font-family: var(--mono);
    font-size: 11px;
  }
</style>
