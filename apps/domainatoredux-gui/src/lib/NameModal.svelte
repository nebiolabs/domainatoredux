<script lang="ts">
  import { untrack } from "svelte";
  // Reusable prompt modal: a required filename, an optional multiline body
  // (for New-file-from-pasted-text), and submit/cancel. Reliable in Tauri
  // webviews where window.prompt() is a no-op.
  let {
    title,
    defaultName = "",
    withBody = false,
    bodyPlaceholder = "",
    submitLabel = "Create",
    validate,
    onsubmit,
    oncancel,
  }: {
    title: string;
    defaultName?: string;
    withBody?: boolean;
    bodyPlaceholder?: string;
    submitLabel?: string;
    // Optional sync pre-check; return an error string to block submit.
    validate?: (name: string) => string | null;
    // May be async and may throw — the message is shown inline and the modal
    // stays open (so pasted content isn't lost). The caller closes it on success.
    onsubmit: (name: string, body: string) => void | Promise<void>;
    oncancel: () => void;
  } = $props();

  // The modal is mounted fresh per open, so the initial name is captured once.
  let name = $state(untrack(() => defaultName));
  let body = $state("");
  let err = $state<string | null>(null);
  let busy = $state(false);
  let nameEl = $state<HTMLInputElement>();

  $effect(() => {
    nameEl?.focus();
    nameEl?.select();
  });

  async function submit() {
    const n = name.trim();
    if (!n || busy) return;
    const v = validate?.(n);
    if (v) {
      err = v;
      return;
    }
    err = null;
    busy = true;
    try {
      await onsubmit(n, body);
    } catch (e) {
      err = String(e);
      busy = false;
    }
  }
</script>

<!-- No click-outside-to-close: a stray click on the backdrop used to throw away
     a pasted body along with the dialog. Closing takes the × or Cancel (or
     Escape, on a name-only prompt where there is nothing to lose). -->
<div class="overlay" role="presentation">
  <div
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    onkeydown={(e) => e.key === "Escape" && !withBody && !busy && oncancel()}
  >
    <div class="head">
      <h3>{title}</h3>
      <button class="close" onclick={oncancel} disabled={busy} title="Close" aria-label="Close">×</button>
    </div>
    <input
      bind:this={nameEl}
      bind:value={name}
      class="name"
      placeholder="filename"
      spellcheck="false"
      onkeydown={(e) => e.key === "Enter" && !withBody && submit()}
    />
    {#if withBody}
      <textarea bind:value={body} class="body" placeholder={bodyPlaceholder} spellcheck="false"></textarea>
    {/if}
    {#if err}<div class="err">{err}</div>{/if}
    <div class="actions">
      <button onclick={oncancel} disabled={busy}>Cancel</button>
      <button class="primary" onclick={submit} disabled={!name.trim() || busy}>
        {busy ? "Working…" : submitLabel}
      </button>
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
    width: min(560px, 90vw);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  h3 {
    margin: 0;
    font-size: 14px;
    flex: 1;
  }
  .close {
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: 18px;
    line-height: 1;
    padding: 0 4px;
  }
  .close:hover:not(:disabled) {
    color: var(--text);
  }
  .name,
  .body {
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg);
    color: var(--text);
    font-size: 13px;
    padding: 6px 8px;
    width: 100%;
    box-sizing: border-box;
  }
  .body {
    min-height: 220px;
    resize: vertical;
    font-family: var(--mono);
    font-size: 12px;
    white-space: pre;
    overflow: auto;
  }
  .err {
    color: #e5484d;
    font-size: 12px;
    line-height: 1.4;
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
  .primary:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
