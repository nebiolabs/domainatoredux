<script lang="ts">
  // A styled confirm, replacing `window.confirm`.
  //
  // Same reason NameModal exists instead of `window.prompt`: browser modals are
  // not dependable in these webviews, and they cannot be styled, so a destructive
  // action looked identical to a benign one. `danger` paints the confirm button
  // red for the ones that delete something.
  let {
    title,
    body = "",
    confirmLabel = "OK",
    danger = false,
    onconfirm,
    oncancel,
  }: {
    title: string;
    body?: string;
    confirmLabel?: string;
    danger?: boolean;
    /// May be async and may throw; the message is shown inline and the dialog
    /// stays open so the user can read it.
    onconfirm: () => void | Promise<void>;
    oncancel: () => void;
  } = $props();

  let busy = $state(false);
  let err = $state<string | null>(null);
  let confirmEl = $state<HTMLButtonElement>();

  // Focus Confirm, not Cancel: the user right-clicked and chose this deliberately,
  // and Escape / click-outside are both one keystroke away.
  $effect(() => {
    confirmEl?.focus();
  });

  async function go() {
    if (busy) return;
    busy = true;
    err = null;
    try {
      await onconfirm();
    } catch (e) {
      err = String(e);
      busy = false;
    }
  }
</script>

<div class="overlay" role="presentation" onclick={oncancel}>
  <div
    class="modal"
    role="dialog"
    aria-modal="true"
    aria-label={title}
    tabindex="-1"
    onclick={(e) => e.stopPropagation()}
    onkeydown={(e) => e.key === "Escape" && oncancel()}
  >
    <h3>{title}</h3>
    {#if body}<p class="body">{body}</p>{/if}
    {#if err}<div class="err">{err}</div>{/if}
    <div class="actions">
      <button onclick={oncancel} disabled={busy}>Cancel</button>
      <button
        bind:this={confirmEl}
        class="primary"
        class:danger
        onclick={go}
        disabled={busy}
      >
        {busy ? "Working…" : confirmLabel}
      </button>
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
    width: min(460px, 90vw);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: 14px;
  }
  .body {
    margin: 0;
    font-size: 12.5px;
    color: var(--text-dim);
    line-height: 1.45;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
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
