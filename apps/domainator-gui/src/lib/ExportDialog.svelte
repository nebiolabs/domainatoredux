<script lang="ts">
  // "Export…": copy files out of the workspace into a folder the user picks,
  // optionally gzipped one by one or bundled into a zip, and optionally split so
  // every sequence record / HMM profile becomes its own file
  // (`app_support::export`). A folder export that would replace existing files
  // writes nothing until the user confirms here.
  import { exportFiles, pickDirectory, pickSavePath } from "./api";
  import { fileTypeOf } from "./filetypes";
  import type { ExportOptions, ExportPackaging } from "./types";

  let {
    paths,
    onclose,
  }: {
    paths: string[];
    onclose: () => void;
  } = $props();

  const baseName = (p: string) => p.split(/[\\/]/).pop() ?? p;
  const names = $derived(paths.map(baseName));
  const SHOWN = 5;

  // Only these split; anything else is exported whole even with the box checked.
  const splittable = $derived(
    names.some((n) => ["genbank", "fasta", "hmm"].includes(fileTypeOf(n) ?? "")),
  );

  let split = $state(false);
  let packaging = $state<ExportPackaging>("plain");
  // A single file, unsplit, is one output: packaging has nothing to package.
  const multi = $derived(paths.length > 1 || (split && splittable));
  const options = $derived<ExportOptions>({
    packaging: multi ? packaging : "plain",
    split_records: split && splittable,
  });

  let busy = $state(false);
  let err = $state<string | null>(null);
  /// A folder export that hit existing files: where, and which.
  let conflict = $state<{ dest: string; names: string[] } | null>(null);
  let done = $state<string | null>(null);

  const title = $derived(paths.length === 1 ? `Export “${names[0]}”` : `Export ${paths.length} files`);

  /// `foo.gb.gz` → `foo`; for several files, their shared folder's name.
  function defaultZipName(): string {
    if (paths.length === 1) {
      const stem = names[0].replace(/\.(gz|bgz|bgzf)$/i, "").replace(/\.[^.]+$/, "");
      return `${stem || "export"}.zip`;
    }
    const dirs = new Set(paths.map((p) => p.slice(0, Math.max(0, p.length - baseName(p).length - 1))));
    const dir = dirs.size === 1 ? baseName([...dirs][0]) : "";
    return `${dir || "export"}.zip`;
  }

  async function run(dest: string, overwrite: boolean) {
    busy = true;
    err = null;
    try {
      const outcome = await exportFiles(paths, options, dest, overwrite);
      if (outcome.kind === "conflicts") {
        conflict = { dest, names: outcome.names };
      } else {
        conflict = null;
        const what = outcome.written === 1 ? "1 file" : `${outcome.written} files`;
        done = `Exported ${what} to ${outcome.dest}.`;
      }
    } catch (e) {
      err = String(e);
    } finally {
      busy = false;
    }
  }

  async function start() {
    if (busy) return;
    err = null;
    try {
      // The native save dialog asks about replacing an existing archive itself.
      const dest =
        options.packaging === "zip"
          ? await pickSavePath(defaultZipName(), "Zip archive", ["zip"])
          : await pickDirectory();
      if (dest) await run(dest, options.packaging === "zip");
    } catch (e) {
      err = String(e);
    }
  }

  function close() {
    if (!busy) onclose();
  }
</script>

<div class="overlay" role="presentation" onclick={close}>
  <div
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === "Escape" && close()}
  >
    <h3>{title}</h3>
    {#if paths.length > 1}
      <ul class="names">
        {#each names.slice(0, SHOWN) as n, i (i)}<li>{n}</li>{/each}
        {#if names.length > SHOWN}<li class="dim">and {names.length - SHOWN} more</li>{/if}
      </ul>
    {/if}

    {#if done}
      <p class="ok">{done}</p>
      <div class="actions">
        <button class="primary" onclick={onclose}>Close</button>
      </div>
    {:else if conflict}
      <p class="warn">
        {conflict.names.length === 1 ? "1 file already exists" : `${conflict.names.length} files already exist`} in
        {conflict.dest}:
      </p>
      <ul class="names">
        {#each conflict.names.slice(0, SHOWN) as n, i (i)}<li>{n}</li>{/each}
        {#if conflict.names.length > SHOWN}<li class="dim">and {conflict.names.length - SHOWN} more</li>{/if}
      </ul>
      <p class="hint">Overwrite {conflict.names.length === 1 ? "it" : "them"}?</p>
      {#if err}<div class="err">{err}</div>{/if}
      <div class="actions">
        <button onclick={() => (conflict = null)} disabled={busy}>Back</button>
        <button class="primary danger" onclick={() => conflict && run(conflict.dest, true)} disabled={busy}>
          {busy ? "Exporting…" : "Overwrite"}
        </button>
      </div>
    {:else}
      {#if splittable}
        <label class="row">
          <input type="checkbox" bind:checked={split} disabled={busy} />
          <span>Write each record as its own file</span>
        </label>
        <p class="hint">
          One file per sequence or HMM profile, named after its id. Other files are exported whole.
        </p>
      {/if}
      {#if multi}
        <div class="row">
          <span class="lbl">Packaging</span>
          {#each [["plain", "None"], ["gzip", "Gzip each file"], ["zip", "Zip archive"]] as [value, label] (value)}
            <label class="radio">
              <input type="radio" name="packaging" {value} bind:group={packaging} disabled={busy} />
              <span>{label}</span>
            </label>
          {/each}
        </div>
      {/if}
      {#if err}<div class="err">{err}</div>{/if}
      <div class="actions">
        <button onclick={close} disabled={busy}>Cancel</button>
        <button class="primary" onclick={start} disabled={busy}>{busy ? "Exporting…" : "Export…"}</button>
      </div>
    {/if}
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
    width: min(460px, 90vw);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: 14px;
    overflow-wrap: anywhere;
  }
  .names {
    margin: 0;
    padding-left: 18px;
    font-size: 12px;
    max-height: 30vh;
    overflow: auto;
    overflow-wrap: anywhere;
  }
  .row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    font-size: 12.5px;
  }
  .lbl {
    margin-right: 4px;
  }
  .radio {
    display: flex;
    align-items: center;
    gap: 3px;
  }
  .hint {
    margin: 0;
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.45;
  }
  .dim {
    color: var(--text-dim);
  }
  .warn,
  .ok {
    margin: 0;
    font-size: 12.5px;
    overflow-wrap: anywhere;
    user-select: text;
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
  .primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .primary.danger {
    background: #e5484d;
    border-color: #e5484d;
  }
</style>
