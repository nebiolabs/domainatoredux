<script lang="ts">
  // About Domainatoredux: what version this is, what it is built on, where its files
  // live, and a copyable blob for bug reports.
  //
  // All of it comes from one `app_diagnostics` call rather than being assembled
  // here, so the text on screen and the text the user pastes into an issue can
  // never describe different builds.
  import { appDiagnostics, saveTextFile, type AppDiagnostics } from "./api";
  import { copyText } from "./clipboard";
  import { openExternalUrl, openAppDataDir } from "./api";

  let { onclose }: { onclose: () => void } = $props();

  let info = $state<AppDiagnostics | null>(null);
  let err = $state<string | null>(null);
  // "copied" / "saved" / a failure message, shown next to the buttons.
  let note = $state<string | null>(null);
  let blobEl = $state<HTMLTextAreaElement>();
  let dialogEl = $state<HTMLDivElement>();

  $effect(() => {
    appDiagnostics()
      .then((d) => (info = d))
      .catch((e) => (err = String(e)));
  });

  // Focus the dialog itself so Escape works without the user clicking first.
  $effect(() => {
    dialogEl?.focus();
  });

  function flash(msg: string) {
    note = msg;
    setTimeout(() => (note = null), 2500);
  }

  /// Copy the diagnostics blob. `copyText` handles the webview clipboard
  /// fallbacks; if even that fails, leave the text selected so it can be copied
  /// by hand, and point at Save.
  async function copyBlob() {
    if (await copyText(info?.diagnostics ?? "")) {
      flash("copied");
      return;
    }
    // The textarea below is selectable and Save always works, so say which.
    blobEl?.focus();
    blobEl?.select();
    flash("could not copy — the text is selected, or use Save");
  }

  async function saveBlob() {
    try {
      const p = await saveTextFile(
        "domainator-diagnostics.txt",
        "txt",
        "Text file",
        info?.diagnostics ?? "",
      );
      if (p) flash("saved");
    } catch (e) {
      flash(String(e));
    }
  }
</script>

<div class="overlay" role="presentation" onclick={onclose}>
  <div
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-label="About Domainatoredux"
    tabindex="-1"
    bind:this={dialogEl}
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === "Escape" && onclose()}
  >
    {#if err}
      <h3>About</h3>
      <div class="err">{err}</div>
    {:else if !info}
      <h3>About</h3>
      <p class="dim">Loading…</p>
    {:else}
      <div class="head">
        <h3>{info.product}</h3>
        <span class="ver">{info.version}</span>
      </div>

      <dl class="facts">
        <dt>Parity with</dt>
        <dd>Python domainator {info.upstream_version} @ {info.upstream_commit}</dd>
        <dt>License</dt>
        <dd>{info.license}</dd>
        <dt>Tauri</dt>
        <dd>{info.tauri_version}</dd>
        <dt>Webview</dt>
        <dd>{info.webview_version}</dd>
        <dt>Platform</dt>
        <dd>{info.os} {info.arch} ({info.profile})</dd>
        <dt>Project</dt>
        <dd class="path row">
          <span class="grow">{info.project_url}</span>
          <button class="mini" onclick={() => openExternalUrl(info!.project_url).catch((e) => flash(String(e)))}>
            Open
          </button>
        </dd>
        <dt>Report a bug</dt>
        <dd class="path row">
          <span class="grow">{info.issues_url}</span>
          <button class="mini" onclick={() => openExternalUrl(info!.issues_url).catch((e) => flash(String(e)))}>
            Open
          </button>
        </dd>
      </dl>

      <div class="sec">External tools</div>
      {#if info.external_tools.length === 0}
        <p class="dim">None.</p>
      {:else}
        <ul class="tools">
          {#each info.external_tools as t (t.name)}
            <li>
              <span class="tname">{t.name}</span>
              <span class="dim">{t.version}</span>
              <span class="state" class:on={t.installed}>
                {t.installed ? "installed" : "not installed"}
              </span>
            </li>
          {/each}
        </ul>
      {/if}

      <div class="sec">Locations</div>
      <dl class="facts">
        <dt>App data</dt>
        <dd class="path row">
          <span class="grow">{info.app_data_dir}</span>
          <button class="mini" onclick={() => openAppDataDir().catch((e) => flash(String(e)))}>
            Show
          </button>
        </dd>
        <dt>Preferences</dt>
        <dd class="path">{info.prefs_path}</dd>
        <dt>Managed binaries</dt>
        <dd class="path">{info.managed_tools_dir}</dd>
      </dl>

      <div class="sec">Diagnostics <span class="dim">— paste this into a bug report</span></div>
      <textarea class="blob" bind:this={blobEl} readonly value={info.diagnostics}></textarea>

      <div class="actions">
        {#if note}<span class="note">{note}</span>{/if}
        <button onclick={copyBlob}>Copy</button>
        <button onclick={saveBlob}>Save…</button>
        <button class="primary" onclick={onclose}>Close</button>
      </div>
    {/if}
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
    width: min(620px, 92vw);
    max-height: 88vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  h3 {
    margin: 0;
    font-size: 15px;
  }
  .ver {
    font-family: var(--mono);
    color: var(--text-dim);
    font-size: 12px;
  }
  .sec {
    margin-top: 6px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
    padding-bottom: 3px;
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
    /* Paths and URLs are the whole point of this box being copyable. */
    user-select: text;
    overflow-wrap: anywhere;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .grow {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .mini {
    flex: none;
    padding: 1px 6px;
    font-size: 11px;
  }
  .path {
    font-family: var(--mono);
    font-size: 11.5px;
  }
  .tools {
    list-style: none;
    margin: 0;
    padding: 0;
    font-size: 12.5px;
  }
  .tools li {
    display: flex;
    gap: 8px;
    align-items: baseline;
    padding: 1px 0;
  }
  .tname {
    font-family: var(--mono);
    min-width: 8em;
  }
  .state {
    color: var(--text-dim);
    font-size: 11.5px;
  }
  .state.on {
    color: var(--accent);
  }
  .blob {
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--text);
    font-family: var(--mono);
    font-size: 11.5px;
    padding: 6px 8px;
    min-height: 150px;
    resize: vertical;
    white-space: pre;
    overflow: auto;
    width: 100%;
    box-sizing: border-box;
  }
  .dim {
    color: var(--text-dim);
    margin: 0;
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
    align-items: center;
    gap: 8px;
    margin-top: 4px;
  }
  .note {
    color: var(--text-dim);
    font-size: 12px;
    margin-right: auto;
  }
  .primary {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
</style>
