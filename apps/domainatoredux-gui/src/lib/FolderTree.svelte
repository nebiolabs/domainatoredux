<script lang="ts">
  import { untrack } from "svelte";
  import { ws } from "./state.svelte";
  import type { FolderNode } from "./types";
  import Self from "./FolderTree.svelte";
  import { openContextMenu, SEP, type CtxItem } from "./ctxmenu.svelte";
  import { copyText } from "./clipboard";
  import { listFolder, revealInFileManager } from "./api";
  import { ui } from "./ui.svelte";

  let { node, depth }: { node: FolderNode; depth: number } = $props();
  // Auto-expand the top level once; `depth` is fixed per node instance.
  let expanded = $state(untrack(() => depth < 1));

  const selected = $derived(ws.selectedFolders.includes(node.path));
  // With several folders listed, the one new files and tool outputs go to.
  const primary = $derived(selected && ws.selectedFolders.length > 1 && ws.selectedFolder === node.path);
  // Files dragged from the table are over this folder (`FileTable` moves them
  // here on release). Hovering a collapsed folder opens it, so a drag can reach
  // a subfolder without letting go.
  const dropping = $derived(ws.dropTarget === node.path);
  $effect(() => {
    if (!dropping || expanded || !node.children.length) return;
    const t = setTimeout(() => (expanded = true), 600);
    return () => clearTimeout(t);
  });

  function select() {
    ws.folderAnchor = node.path;
    ws.selectFolder(node.path);
    if (node.children.length) expanded = true;
  }

  // A plain click shows this folder alone; Ctrl/Cmd+click adds it to the listed
  // folders (or takes it out), and Shift+click lists every visible folder from
  // the anchor to here. Neither modifier expands the folder.
  function click(e: MouseEvent) {
    if (e.shiftKey && ws.folderAnchor) {
      // The rows are siblings in display order (each subtree renders flat after
      // its parent), so the visible range is a slice of them; a collapsed
      // folder's children are not rows and stay out.
      const row = e.currentTarget as HTMLElement;
      const order = [...(row.parentElement?.querySelectorAll<HTMLElement>(":scope > [data-folder-path]") ?? [])].map(
        (el) => el.dataset.folderPath!,
      );
      const a = order.indexOf(ws.folderAnchor);
      const b = order.indexOf(node.path);
      if (a >= 0 && b >= 0) {
        const range = order.slice(Math.min(a, b), Math.max(a, b) + 1).filter((p) => p !== node.path);
        ws.selectFolders([...range, node.path]); // the clicked one is the primary
        return;
      }
    }
    if (e.ctrlKey || e.metaKey) {
      ws.folderAnchor = node.path;
      ws.toggleFolder(node.path);
      return;
    }
    select();
  }

  // Folder rename/delete/move are absent on purpose: `cmd_workspace`'s
  // rename_file / delete_file / move_files are file-level (files move here by
  // dragging them from the table), and folder equivalents would be new backend
  // commands with their own reserved-name and recursion rules.
  function menu(e: MouseEvent) {
    const items: CtxItem[] = [
      { label: "Select", action: select },
      {
        label: selected ? "Remove from Selection" : "Add to Selection",
        disabled: selected && ws.selectedFolders.length === 1,
        action: () => {
          ws.folderAnchor = node.path;
          ws.toggleFolder(node.path);
        },
      },
      {
        label: node.children.length ? (expanded ? "Collapse" : "Expand") : "Expand",
        disabled: !node.children.length,
        action: () => {
          expanded = !expanded;
        },
      },
      SEP,
      {
        label: "New Subfolder…",
        action: () => {
          ws.selectFolder(node.path);
          ws.requestNewFolder = node.path;
        },
      },
      {
        label: "Export Files…",
        action: async () => {
          // The folder's own data files, as the file table lists them -- not
          // its subfolders.
          const files = await listFolder(ws.root ?? node.path, node.path);
          if (files.length) ui.exportFor = files.map((f) => f.path);
          else ws.error = "No files to export in this folder.";
        },
      },
      SEP,
      {
        label: "Copy Full Path",
        action: async () => {
          if (!(await copyText(node.path))) ws.error = "Could not copy to the clipboard.";
        },
      },
      { label: "Show in File Manager", action: () => revealInFileManager(ws.root, node.path) },
      SEP,
      { label: "Refresh Tree", action: () => ws.refreshTree() },
    ];
    openContextMenu(e, items);
  }
</script>

<div
  class="row"
  class:selected
  class:primary
  class:dropping
  data-folder-path={node.path}
  style="padding-left: {depth * 14 + 8}px"
  onclick={click}
  role="treeitem"
  aria-selected={selected}
  aria-expanded={node.children.length ? expanded : undefined}
  tabindex="0"
  onkeydown={(e) => e.key === "Enter" && select()}
  oncontextmenu={menu}
>
  {#if node.children.length}
    <button
      class="twisty"
      onclick={(e) => {
        e.stopPropagation();
        expanded = !expanded;
      }}
      aria-label={expanded ? "Collapse" : "Expand"}>{expanded ? "▾" : "▸"}</button
    >
  {:else}
    <span class="twisty spacer"></span>
  {/if}
  <span class="icon">🗀</span>
  <span class="name" title={primary ? `${node.name} (new files and tool outputs go here)` : node.name}>{node.name}</span>
</div>

{#if expanded}
  {#each node.children as child (child.path)}
    <Self node={child} depth={depth + 1} />
  {/each}
{/if}

<style>
  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding-right: 8px;
    cursor: default;
    user-select: none;
    white-space: nowrap;
  }
  .row:hover {
    background: var(--row-hover);
  }
  .row.selected {
    background: var(--row-selected);
  }
  .row.primary .name {
    font-weight: 600;
  }
  .row.dropping {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
  .twisty {
    width: 16px;
    height: 16px;
    padding: 0;
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: 13px;
    line-height: 1;
  }
  .twisty:hover {
    background: none;
    color: var(--text);
  }
  .spacer {
    display: inline-block;
  }
  .icon {
    font-size: 12px;
    opacity: 0.8;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
