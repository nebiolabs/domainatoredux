<script lang="ts">
  // The toolbar's context-sensitive Tools button: the native Tools menu's tool
  // list, cut down to the tools that can take every selected file at once. Same
  // categories in the same order (`list_tools` returns them sorted), drawn with
  // the shared HTML menu so it can hang under the button.
  import { toggleMenuBelow, SEP, type CtxItem } from "./ctxmenu.svelte";
  import { ws } from "./state.svelte";
  import { toolChoices } from "./toolMenu";
  import type { ToolMeta } from "./types";

  let { tools }: { tools: ToolMeta[] } = $props();

  let btn = $state<HTMLButtonElement>();

  // The file table's selection, else the active tab's file when it is in the
  // current folder -- the files ToolDialog places in its inputs.
  const files = $derived(ws.toolTargets);
  const groups = $derived(toolChoices(tools, files));
  const title = $derived(files.length === 1 ? files[0].name : `${files.length} files`);

  const hint = $derived(
    !files.length
      ? "Select a file to see the tools that take it as input"
      : groups.size === 0
        ? files.length === 1
          ? `No tool takes a ${files[0].file_type} file as input`
          : `No tool takes all ${files.length} selected files as input`
        : `Tools that take ${files.length === 1 ? files[0].name : `these ${files.length} files`} as input`,
  );

  function items(): CtxItem[] {
    const out: CtxItem[] = [{ label: title, disabled: true }, SEP];
    for (const [cat, list] of groups) {
      out.push({
        label: cat,
        submenu: list.map((c) => ({ label: c.tool.label, action: c.open })),
      });
    }
    return out;
  }
</script>

<button
  bind:this={btn}
  class="tools"
  title={hint}
  aria-haspopup="menu"
  disabled={!ws.root || groups.size === 0}
  onclick={() => btn && toggleMenuBelow(btn, items())}
>
  Tools <span class="caret">▾</span>
</button>

<style>
  .caret {
    font-size: 10px;
    color: var(--text-dim);
  }
</style>
