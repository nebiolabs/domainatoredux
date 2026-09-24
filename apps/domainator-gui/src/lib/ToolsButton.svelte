<script lang="ts">
  // The toolbar's context-sensitive Tools button: the native Tools menu's tool
  // list, cut down to the tools that take the selected file as an input. Same
  // categories in the same order (`list_tools` returns them sorted), drawn with
  // the shared HTML menu so it can hang under the button.
  import { toggleMenuBelow, SEP, type CtxItem } from "./ctxmenu.svelte";
  import { ws } from "./state.svelte";
  import { toolAcceptsFile } from "./toolInputs";
  import type { ToolMeta } from "./types";

  let { tools }: { tools: ToolMeta[] } = $props();

  let btn = $state<HTMLButtonElement>();

  // "Selected" is the active tab -- the file the table highlights and the one
  // ToolDialog pre-fills its inputs with.
  const file = $derived(ws.tabs.find((t) => t.id === ws.activeTabId)?.file ?? null);
  const fitting = $derived(file ? tools.filter((t) => toolAcceptsFile(t, file.file_type)) : []);

  const hint = $derived(
    !file
      ? "Select a file to see the tools that take it as input"
      : fitting.length === 0
        ? `No tool takes a ${file.file_type} file as input`
        : `Tools that take ${file.name} as input`,
  );

  function items(): CtxItem[] {
    const groups = new Map<string, ToolMeta[]>();
    for (const t of fitting) {
      const g = groups.get(t.category);
      if (g) g.push(t);
      else groups.set(t.category, [t]);
    }
    const out: CtxItem[] = [{ label: file!.name, disabled: true }, SEP];
    for (const [cat, list] of groups) {
      out.push({
        label: cat,
        submenu: list.map((t) => ({ label: t.label, action: () => ws.openTool(t.id) })),
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
  disabled={!ws.root || fitting.length === 0}
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
