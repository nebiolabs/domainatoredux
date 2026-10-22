<script lang="ts">
  // Settings that belong to the app rather than to any one workspace.
  //
  // Thread count lives here, not in the workspace manifest, because it is a
  // property of the *machine*: a workspace copied from a 64-core node to a laptop
  // must not bring `default_cpu = 64` with it.
  import {
    appDiagnostics,
    checkForUpdates,
    getPrefs,
    openAppDataDir,
    revealInFileManager,
    setPrefs,
  } from "./api";
  import ColumnBuilder from "./ColumnBuilder.svelte";
  import { toColumns, toRows, type ColumnRow, type ColumnType } from "./columns";
  import { defaultColumns, loadEnumColumnTypes } from "./enumColumns";
  import { ENUM_BYS, type EnumBy } from "./selection";
  import { applyTheme } from "./theme";
  import { ws } from "./state.svelte";
  import { ui } from "./ui.svelte";
  import type { AppDiagnostics } from "./api";
  import type { Prefs, Theme, UpdateCheck } from "./types";

  let { onclose }: { onclose: () => void } = $props();

  let prefs = $state<Prefs | null>(null);
  let err = $state<string | null>(null);
  let info = $state<AppDiagnostics | null>(null);
  let dialogEl = $state<HTMLDivElement>();

  // Update state. Deliberately local to this dialog: a failed check is shown
  // here and never on the app-wide error bar.
  let checking = $state(false);
  let update = $state<UpdateCheck | null>(null);

  const cores = typeof navigator !== "undefined" ? (navigator.hardwareConcurrency ?? 0) : 0;

  // Enum Report default columns, edited one granularity at a time with the
  // same builder the report uses. The draft is reloaded only on an explicit
  // change (granularity, save, reset), so saving another field mid-edit does
  // not throw the edit away.
  let erBy = $state<EnumBy>("cds");
  let erTypes = $state<ColumnType[]>([]);
  let erDraft = $state<ColumnRow[]>([]);
  let erError = $state<string | null>(null);
  let erMsg = $state<string | null>(null);
  function loadErDraft() {
    if (!prefs || !erTypes.length) return;
    erDraft = toRows(defaultColumns(erBy, prefs), erTypes);
    erError = null;
  }
  async function saveEr(cols: Record<string, unknown>[] | null) {
    if (!prefs) return;
    if (!(await save({ enum_report_columns: { ...prefs.enum_report_columns, [erBy]: cols } }))) return;
    loadErDraft();
    erMsg = cols ? "Saved." : "Back to the built-in columns.";
    setTimeout(() => (erMsg = null), 2500);
  }
  function saveErDraft() {
    try {
      void saveEr(toColumns(erDraft, erTypes));
    } catch (e) {
      erError = e instanceof Error ? e.message : String(e);
    }
  }

  $effect(() => {
    loadEnumColumnTypes()
      .then((t) => {
        erTypes = t;
        loadErDraft();
      })
      .catch((e) => (erError = String(e)));
  });

  $effect(() => {
    getPrefs()
      .then((p) => {
        prefs = p;
        loadErDraft();
      })
      .catch((e) => (err = String(e)));
    appDiagnostics()
      .then((d) => (info = d))
      .catch(() => {
        /* the Locations section just stays empty; not worth blocking on */
      });
  });

  $effect(() => {
    dialogEl?.focus();
  });

  // Opened from Help → Check for Updates…: run the check straight away rather
  // than making the user find the button.
  $effect(() => {
    if (ui.updateCheckRequest > 0) void runCheck();
  });

  /// Save one field. The patch shape means an unnamed field is never cleared.
  /// Returns whether it saved.
  async function save(patch: Partial<Prefs>): Promise<boolean> {
    try {
      prefs = await setPrefs(patch);
      if (patch.theme) applyTheme(patch.theme);
      return true;
    } catch (e) {
      err = String(e);
      return false;
    }
  }

  async function runCheck() {
    checking = true;
    update = null;
    try {
      update = await checkForUpdates();
    } catch (e) {
      // Still inline: being offline must never look like an app failure.
      update = { current: prefs ? "" : "", latest: null, newer: false, note: String(e) };
    } finally {
      checking = false;
    }
  }

  function lastChecked(ms: number | null): string {
    if (!ms) return "never";
    return new Date(ms).toLocaleString();
  }

  /// Clear the per-machine layout keys. Listed here because they are real state
  /// the user may want to reset, and they are invisible otherwise.
  function resetLayout() {
    for (const k of ["shell.leftWidth", "shell.tableHeight", "ft.cols", "ft.sort"]) {
      try {
        localStorage.removeItem(k);
      } catch {
        /* storage unavailable; nothing to reset */
      }
    }
    err = "Panel sizes and columns reset — they apply on the next launch.";
  }
</script>

<div class="overlay" role="presentation" onclick={onclose}>
  <div
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-label="Preferences"
    tabindex="-1"
    bind:this={dialogEl}
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === "Escape" && onclose()}
  >
    <h3>Preferences</h3>
    {#if err}<div class="err">{err}</div>{/if}

    {#if !prefs}
      <p class="dim">Loading…</p>
    {:else}
      <div class="sec">General</div>
      <label class="row">
        <input
          type="checkbox"
          checked={prefs.reopen_last_workspace}
          onchange={(e) => save({ reopen_last_workspace: e.currentTarget.checked })}
        />
        <span>Reopen the last workspace on launch</span>
      </label>

      <div class="row">
        <span class="lbl">Theme</span>
        {#each ["system", "light", "dark"] as t (t)}
          <label class="radio">
            <input
              type="radio"
              name="theme"
              value={t}
              checked={prefs.theme === t}
              onchange={() => save({ theme: t as Theme })}
            />
            <span>{t}</span>
          </label>
        {/each}
      </div>

      <div class="sec">Performance</div>
      <div class="row">
        <span class="lbl">Default threads</span>
        <input
          class="num"
          type="number"
          min="0"
          value={prefs.default_cpu}
          onchange={(e) => save({ default_cpu: Math.max(0, Number(e.currentTarget.value) || 0) })}
        />
        <span class="dim">0 = all cores{cores ? ` (detected ${cores})` : ""}</span>
      </div>
      <p class="hint">
        Applies only to tools that accept a thread count, and only when you have not set
        one in the tool's own form.
      </p>
      <div class="row">
        <span class="lbl">Jobs at once</span>
        <input
          class="num"
          type="number"
          min="1"
          value={prefs.max_concurrent_jobs}
          onchange={(e) =>
            save({ max_concurrent_jobs: Math.max(1, Math.floor(Number(e.currentTarget.value)) || 1) })}
        />
        <span class="dim">more wait in the Jobs panel's queue</span>
      </div>
      <p class="hint">
        Each job runs in its own process and uses the thread count above, so two jobs at
        once already share the cores between them.
      </p>

      <div class="sec">Enum Report columns</div>
      <div class="row">
        <span class="lbl">Granularity</span>
        <select bind:value={erBy} onchange={loadErDraft}>
          {#each ENUM_BYS as b (b)}<option value={b}>{b}</option>{/each}
        </select>
        <span class="dim">{prefs.enum_report_columns[erBy] ? "custom default" : "built-in default"}</span>
      </div>
      {#if erTypes.length}
        <ColumnBuilder types={erTypes} bind:rows={erDraft} />
      {:else if !erError}
        <p class="dim">Loading column types…</p>
      {/if}
      {#if erError}<div class="err">{erError}</div>{/if}
      <div class="row">
        <button onclick={saveErDraft} disabled={!erTypes.length}>Save as default</button>
        <button onclick={() => saveEr(null)} disabled={!prefs.enum_report_columns[erBy]}>Use built-in</button>
        {#if erMsg}<span class="dim">{erMsg}</span>{/if}
      </div>
      <p class="hint">
        The columns an Enum Report starts with at this granularity. Each file remembers the
        columns you last applied to it, and those win; the report's <b>Defaults</b> button
        goes back to this setting.
      </p>

      <div class="sec">Updates</div>
      <label class="row">
        <input
          type="checkbox"
          checked={prefs.check_updates_on_launch}
          onchange={(e) => save({ check_updates_on_launch: e.currentTarget.checked })}
        />
        <span>Check for a newer release at launch</span>
      </label>
      <div class="row">
        <button onclick={runCheck} disabled={checking}>
          {checking ? "Checking…" : "Check now"}
        </button>
        <span class="dim">Last checked: {lastChecked(prefs.last_update_check_ms)}</span>
      </div>
      {#if update}
        <p class="hint" class:warn={update.newer}>
          {#if update.newer && update.latest}
            {update.latest.tag} is available (you have {update.current}).
          {:else if update.note}
            {update.note}
          {:else}
            You're up to date ({update.current}).
          {/if}
        </p>
      {/if}

      <div class="sec">Locations</div>
      {#if info}
        <dl class="facts">
          <dt>App data</dt>
          <dd class="path row">
            <span class="grow">{info.app_data_dir}</span>
            <button class="mini" onclick={() => openAppDataDir().catch((e) => (err = String(e)))}>
              Show
            </button>
          </dd>
          <dt>Preferences</dt>
          <dd class="path">{info.prefs_path}</dd>
          <dt>Managed binaries</dt>
          <dd class="path">{info.managed_tools_dir}</dd>
          {#if ws.root}
            <dt>Workspace data</dt>
            <dd class="path row">
              <span class="grow">{ws.root}/.domainatoredux</span>
              <button
                class="mini"
                onclick={() =>
                  revealInFileManager(ws.root, `${ws.root}/.domainatoredux`).catch(
                    (e) => (err = String(e)),
                  )}
              >
                Show
              </button>
            </dd>
          {/if}
        </dl>
        <p class="hint">
          <b>.domainatoredux</b> inside a workspace holds this app's own records — the file
          manifest, the database registry, job history and provenance. Deleting it loses
          that history but never your data files.
        </p>
      {/if}

      <div class="sec">Layout</div>
      <div class="row">
        <button onclick={resetLayout}>Reset panel sizes and columns</button>
      </div>
    {/if}

    <div class="actions">
      <button class="primary" onclick={onclose}>Close</button>
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 100;
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
    width: min(600px, 92vw);
    max-height: 88vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  h3 {
    margin: 0;
    font-size: 15px;
  }
  .sec {
    margin-top: 8px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
    padding-bottom: 3px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  .lbl {
    min-width: 8.5em;
    color: var(--text-dim);
  }
  .radio {
    display: flex;
    align-items: center;
    gap: 3px;
    text-transform: capitalize;
  }
  .num {
    width: 5em;
    padding: 3px 5px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--text);
    font-size: 12.5px;
  }
  .hint {
    margin: 0;
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.45;
  }
  .hint.warn {
    color: var(--accent);
  }
  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 2px 12px;
    margin: 0;
    font-size: 12.5px;
  }
  .facts dt {
    color: var(--text-dim);
  }
  .facts dd {
    margin: 0;
    min-width: 0;
    user-select: text;
  }
  .path {
    font-family: var(--mono);
    font-size: 11.5px;
    overflow-wrap: anywhere;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .mini {
    flex: none;
    padding: 1px 6px;
    font-size: 11px;
  }
  .dim {
    color: var(--text-dim);
    font-size: 12px;
  }
  .err {
    color: #e5484d;
    font-size: 12px;
    user-select: text;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 8px;
  }
  .primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
</style>
