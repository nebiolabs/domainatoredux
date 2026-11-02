<script lang="ts">
  // Managed reference databases (UniProt, Pfam, NCBI Taxonomy) and models (ProstT5,
  // ProtT5), the data counterpart of the External tools panel.
  //
  // Two things here exist only because of the sizes involved: a confirmation step
  // in front of every download but Swiss-Prot, and a live progress row fed by the
  // backend's `refdb-progress` event. A TrEMBL install runs for hours, so a panel
  // that only said "downloading…" would be indistinguishable from one that had hung.
  import ResizableBody from "./ResizableBody.svelte";
  import PanelSeam from "./PanelSeam.svelte";
  import { listen } from "@tauri-apps/api/event";
  import {
    cancelReferenceDb,
    checkReferenceDbVersion,
    downloadReferenceDb,
    listReferenceDbs,
    removeReferenceDb,
  } from "./api";
  import type { RefDbProgress, ReferenceDbStatus } from "./types";
  import { ws } from "./state.svelte";
  import { ui } from "./ui.svelte";
  import { flashPanel } from "./panelFlash";
  import ConfirmDialog from "./ConfirmDialog.svelte";

  let expanded = $state(false);

  // Tools menu → expand, outline for a few seconds, and clear the request.
  let panelEl = $state<HTMLDivElement>();
  $effect(() => {
    if (ui.focusPanel === "refdbs") {
      expanded = true;
      ui.focusPanel = null;
      flashPanel(panelEl);
    }
  });

  let dbs = $state<ReferenceDbStatus[]>([]);
  /// Free space on the volume the databases live on, and where that is. `null`
  /// means the platform would not say, in which case nothing is blocked on it.
  let freeBytes = $state<number | null>(null);
  let dir = $state("");
  let loading = $state(true);
  let listError = $state<string | null>(null);
  let errors = $state<Record<string, string>>({});
  let checking = $state<Record<string, boolean>>({});
  let progress = $state<Record<string, RefDbProgress>>({});
  /// The database awaiting a size confirmation, if any.
  let pending = $state<ReferenceDbStatus | null>(null);
  /// The installed database awaiting a delete confirmation, if any.
  let pendingRemove = $state<ReferenceDbStatus | null>(null);

  async function refresh() {
    try {
      const list = await listReferenceDbs();
      dbs = list.databases;
      freeBytes = list.free_bytes;
      dir = list.dir;
      listError = null;
    } catch (e) {
      listError = String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    refresh();
  });

  // Progress arrives on a backend event rather than by polling: the download runs
  // in a blocking task, and nothing else would know how far along it is.
  $effect(() => {
    const un = listen<RefDbProgress>("refdb-progress", (e) => {
      const p = e.payload;
      if (p.phase === "done" || p.phase === "failed" || p.phase === "canceled") {
        delete progress[p.id];
        refresh();
        // A finished download registers itself in the workspace; pick it up.
        if (p.phase === "done") ws.loadDatabases();
      } else {
        progress[p.id] = p;
      }
    });
    return () => {
      un.then((f) => f());
    };
  });

  function humanBytes(n: number): string {
    if (n <= 0) return "";
    const units = ["B", "KB", "MB", "GB", "TB"];
    let v = n;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
  }

  function percent(p: RefDbProgress): number | null {
    if (p.phase !== "downloading" || p.total_bytes <= 0) return null;
    return Math.min(100, (p.done_bytes / p.total_bytes) * 100);
  }

  function phaseLabel(p: RefDbProgress): string {
    if (p.phase === "indexing") return "building index…";
    if (p.phase === "registering") return "registering…";
    const pct = percent(p);
    const done = humanBytes(p.done_bytes);
    if (pct === null) return `downloading… ${done}`;
    return `downloading… ${done} of ${humanBytes(p.total_bytes)} (${Math.round(pct)}%)`;
  }

  /// Why a download is blocked. An update needs room for both copies at once — the
  /// old file is only replaced at the closing rename — which is why this can exceed
  /// the installed size the row otherwise advertises.
  function shortfall(db: ReferenceDbStatus): string {
    return `needs ≈${humanBytes(db.required_bytes)}, ${humanBytes(freeBytes ?? 0)} free`;
  }

  async function start(db: ReferenceDbStatus) {
    delete errors[db.id];
    // Show the row as busy immediately: the first progress event only arrives
    // after the release file and the first megabytes have been fetched.
    progress[db.id] = { id: db.id, phase: "downloading", done_bytes: 0, total_bytes: 0 };
    try {
      await downloadReferenceDb(db.id);
    } catch (e) {
      // A cancel comes back as an error too, and is not worth reporting as one.
      if (!String(e).includes("canceled")) errors[db.id] = String(e);
    } finally {
      delete progress[db.id];
      await refresh();
      await ws.loadDatabases();
    }
  }

  function request(db: ReferenceDbStatus) {
    if (db.needs_confirmation) pending = db;
    else start(db);
  }

  async function check(db: ReferenceDbStatus) {
    checking[db.id] = true;
    delete errors[db.id];
    try {
      await checkReferenceDbVersion(db.id);
      await refresh();
    } catch (e) {
      errors[db.id] = String(e);
    } finally {
      checking[db.id] = false;
    }
  }

  const installedCount = $derived(dbs.filter((d) => d.installed).length);
</script>

<div class="refdbs" bind:this={panelEl}>
  <PanelSeam key="refdbs" active={expanded} />
  <div class="header">
    <button class="twisty" onclick={() => (expanded = !expanded)} aria-label="Toggle reference databases">
      {expanded ? "▾" : "▸"}
    </button>
    <span class="title">Reference databases</span>
    <span class="count">{installedCount}/{dbs.length}</span>
  </div>

  {#if expanded && freeBytes !== null}
    <div class="free" title={dir}>{humanBytes(freeBytes)} free on this volume</div>
  {/if}

  {#if expanded}
    <ResizableBody key="refdbs" defaultMax={320}>
    {#if loading}
      <div class="empty">Checking…</div>
    {:else if listError}
      <div class="err">{listError}</div>
    {/if}

    {#each dbs as db (db.id)}
      <div class="db">
        <div class="row1">
          <span class="name" title={db.description}>{db.name}</span>
          {#if progress[db.id]}
            <button class="cancel" onclick={() => cancelReferenceDb(db.id)}>Cancel</button>
          {:else if db.installed}
            <button
              class="rm"
              title="Delete the downloaded files"
              aria-label="Remove {db.name}"
              onclick={() => (pendingRemove = db)}>×</button
            >
          {:else}
            <button
              class="dl"
              disabled={!db.enough_space}
              title={db.enough_space ? "" : shortfall(db)}
              onclick={() => request(db)}>Download</button
            >
          {/if}
        </div>

        <div class="row2">
          {#if db.installed}
            <span class="ver" title={db.installed.path}>
              {db.installed.version} · {humanBytes(db.installed.bytes)}
            </span>
          {:else}
            <span class="size">≈{humanBytes(db.download_bytes)} download</span>
          {/if}
          {#if !db.enough_space}
            <span class="nospace">{shortfall(db)}</span>
          {/if}
          {#if !progress[db.id] && !db.pinned}
            <button class="check" disabled={checking[db.id]} onclick={() => check(db)}>
              {checking[db.id] ? "checking…" : "Check"}
            </button>
          {/if}
        </div>

        {#if db.update_available && !progress[db.id]}
          <div class="update">
            {db.latest_version} available
            <button
              class="dl"
              disabled={!db.enough_space}
              title={db.enough_space ? "" : shortfall(db)}
              onclick={() => request(db)}>Update</button
            >
          </div>
        {:else if db.latest_version && !db.installed && !progress[db.id]}
          <div class="latest">latest release {db.latest_version}</div>
        {/if}

        {#if progress[db.id]}
          <div class="prog">
            <div class="bar">
              {#if percent(progress[db.id]) !== null}
                <div class="fill" style="width: {percent(progress[db.id])}%"></div>
              {:else}
                <div class="fill indeterminate"></div>
              {/if}
            </div>
            <div class="phase">{phaseLabel(progress[db.id])}</div>
          </div>
        {/if}

        {#if errors[db.id]}<div class="err">{errors[db.id]}</div>{/if}
      </div>
    {/each}
    </ResizableBody>
  {/if}
</div>

{#if pending}
  <ConfirmDialog
    title={`Download ${pending.name}?`}
    body={`This downloads about ${humanBytes(pending.download_bytes)} and uses about ${humanBytes(
      pending.disk_bytes,
    )} of disk once installed${
      pending.installed
        ? `, and about ${humanBytes(pending.required_bytes)} free while it runs — the copy you have now is only replaced at the end`
        : ""
    }.${
      freeBytes === null ? "" : `\n\nYou have ${humanBytes(freeBytes)} free on this volume.`
    }\n\nIt can take a long time, and can be canceled at any point.`}
    confirmLabel="Download"
    onconfirm={() => {
      const db = pending;
      pending = null;
      if (db) start(db);
    }}
    oncancel={() => (pending = null)}
  />
{/if}

{#if pendingRemove}
  <ConfirmDialog
    title={`Remove ${pendingRemove.name}?`}
    body={`Deletes ${pendingRemove.installed?.path ?? pendingRemove.file_name}${
      pendingRemove.kind === "fasta" ? " and its index" : ""
    }. Downloading it again means fetching about ${humanBytes(
      pendingRemove.download_bytes,
    )} over the network.`}
    confirmLabel="Remove"
    danger={true}
    onconfirm={async () => {
      const db = pendingRemove;
      pendingRemove = null;
      if (!db) return;
      try {
        await removeReferenceDb(db.id);
      } catch (e) {
        errors[db.id] = String(e);
      }
      await refresh();
      await ws.loadDatabases();
    }}
    oncancel={() => (pendingRemove = null)}
  />
{/if}

<style>
  .refdbs {
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
    margin-left: auto;
  }
  .db {
    padding: 4px 8px 5px 12px;
    border-bottom: 1px solid var(--border);
  }
  .row1 {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .row2 {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin-top: 1px;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ver {
    font-family: var(--mono);
    font-size: 11px;
    color: #06842b;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size,
  .latest {
    color: var(--text-dim);
    font-size: 11px;
  }
  .free {
    padding: 2px 8px 3px 12px;
    color: var(--text-dim);
    font-size: 11px;
  }
  .nospace {
    color: #e5484d;
    font-size: 11px;
  }
  .dl:disabled {
    opacity: 0.5;
  }
  .update {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    color: var(--accent);
    margin-top: 2px;
  }
  .dl,
  .cancel {
    margin-left: auto;
    padding: 1px 8px;
    font-size: 11px;
  }
  .update .dl {
    margin-left: 0;
  }
  .check {
    margin-left: auto;
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: 11px;
    padding: 0 2px;
    text-decoration: underline;
  }
  .check:hover {
    background: none;
    color: var(--accent);
  }
  .rm {
    margin-left: auto;
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
  .prog {
    margin-top: 4px;
  }
  .bar {
    height: 4px;
    background: var(--border);
    border-radius: 2px;
    overflow: hidden;
  }
  .fill {
    height: 100%;
    background: var(--accent);
    transition: width 0.2s linear;
  }
  /* No total to divide by (no Content-Length, or the indexing pass, which has no
     cheap way to report a fraction). A moving stripe says "working" honestly
     rather than inventing a percentage. */
  .indeterminate {
    width: 35%;
    animation: slide 1.4s ease-in-out infinite;
  }
  @keyframes slide {
    0% {
      margin-left: 0;
    }
    50% {
      margin-left: 65%;
    }
    100% {
      margin-left: 0;
    }
  }
  .phase {
    color: var(--text-dim);
    font-size: 11px;
    margin-top: 2px;
  }
  .err {
    color: #e5484d;
    font-size: 11px;
    margin-top: 2px;
    overflow-wrap: anywhere;
  }
  .empty {
    padding: 6px 12px;
    color: var(--text-dim);
    line-height: 1.4;
  }
</style>
