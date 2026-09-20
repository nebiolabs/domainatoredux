<script lang="ts">
  import { extractHmms, listHmmProfiles, openHmmLogoWindow } from "../api";
  import { cached } from "../cache";
  import { ws } from "../state.svelte";
  import { validateFilename } from "../filetypes";
  import NameModal from "../NameModal.svelte";
  import type { HmmSummary } from "../types";

  let { path }: { path: string } = $props();

  // True in the main window's viewer dock (which has a sibling "Logo" tab), false
  // in a detached `?view=file` window, which renders a single view with no tabs.
  const inDock = !new URLSearchParams(window.location.search).get("view");

  let profiles = $state<HmmSummary[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let filter = $state("");
  let sortDir = $state<1 | -1>(1);
  let sortCol = $state<"name" | "length">("name");

  // Multi-select (by profile index) for Extract → new .hmm.
  let selected = $state(new Set<number>());
  let anchor = -1; // last plainly-clicked profile index (shift-range base)
  let showExtract = $state(false);

  function rowClick(e: MouseEvent, p: HmmSummary) {
    if (e.shiftKey && anchor >= 0) {
      const a = filtered.findIndex((x) => x.index === anchor);
      const b = filtered.findIndex((x) => x.index === p.index);
      if (a >= 0 && b >= 0) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        const s = new Set(selected);
        for (let i = lo; i <= hi; i++) s.add(filtered[i].index);
        selected = s;
      }
    } else if (e.ctrlKey || e.metaKey) {
      const s = new Set(selected);
      s.has(p.index) ? s.delete(p.index) : s.add(p.index);
      selected = s;
      anchor = p.index;
    } else {
      selected = new Set([p.index]);
      anchor = p.index;
    }
  }

  // Throws on failure so the modal shows it inline and stays open; closes on success.
  async function doExtract(name: string) {
    const dir = ws.currentDir;
    if (!dir || selected.size === 0) return;
    const p = await extractHmms(dir, name, path, [...selected].sort((a, b) => a - b));
    await ws.refreshAndOpen(p);
    selected = new Set();
    showExtract = false;
  }

  // Load the profile summaries (name / length / description) — this is fast: it
  // never computes logos, so opening an HMM file no longer blocks on that.
  $effect(() => {
    void path;
    loading = true;
    error = null;
    filter = "";
    cached(`hmmprofiles:${path}`, () => listHmmProfiles(path))
      .then((p) => (profiles = p))
      .catch((e) => (error = String(e)))
      .finally(() => (loading = false));
  });

  const filtered = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    let list = q
      ? profiles.filter(
          (p) => p.name.toLowerCase().includes(q) || (p.desc ?? "").toLowerCase().includes(q),
        )
      : profiles;
    const dir = sortDir;
    const col = sortCol;
    list = [...list].sort((a, b) =>
      col === "length" ? (a.length - b.length) * dir : a.name.localeCompare(b.name) * dir,
    );
    return list;
  });

  function toggleSort(c: "name" | "length") {
    if (sortCol === c) sortDir = sortDir === 1 ? -1 : 1;
    else {
      sortCol = c;
      sortDir = 1;
    }
    if (scrollEl) scrollEl.scrollTop = 0;
  }

  // Show this profile's logo in the dock's sibling "Logo" tab. In a detached
  // file window there is no dock, so fall back to a logo window there.
  function showLogo(p: HmmSummary) {
    ws.focusHmmProfile[path] = p.index;
    if (inDock) ws.showSubView(path, "logo");
    else void popLogo(p);
  }

  // Open this profile's logo in its own window.
  async function popLogo(p: HmmSummary) {
    try {
      await openHmmLogoWindow(path, p.index, p.name);
    } catch (e) {
      error = String(e);
    }
  }

  // Row virtualization (HMM files can have tens of thousands of profiles).
  const ROW_H = 26;
  const OVERSCAN = 12;
  let scrollEl = $state<HTMLDivElement>();
  let scrollTop = $state(0);
  let clientH = $state(400);
  let rafPending = false;
  function onScroll() {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      if (scrollEl) {
        scrollTop = scrollEl.scrollTop;
        clientH = scrollEl.clientHeight;
      }
    });
  }
  $effect(() => {
    if (!scrollEl) return;
    clientH = scrollEl.clientHeight;
    const ro = new ResizeObserver(() => {
      if (scrollEl) clientH = scrollEl.clientHeight;
    });
    ro.observe(scrollEl);
    return () => ro.disconnect();
  });

  const total = $derived(filtered.length);
  const firstIdx = $derived(Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN));
  const lastIdx = $derived(Math.min(total, firstIdx + Math.ceil(clientH / ROW_H) + 2 * OVERSCAN));
  const visible = $derived(filtered.slice(firstIdx, lastIdx));
  const topPad = $derived(firstIdx * ROW_H);
  const bottomPad = $derived(Math.max(0, (total - lastIdx) * ROW_H));
</script>

<div class="hr">
  <div class="controls">
    <input class="filter" placeholder="filter profiles…" bind:value={filter} />
    <span class="dim">
      {#if filter.trim()}{total.toLocaleString()} of {profiles.length.toLocaleString()}{:else}{profiles.length.toLocaleString()}{/if}
      profile{total === 1 ? "" : "s"}
    </span>
    {#if selected.size > 0}
      <span class="sel-badge">{selected.size} selected</span>
      <button class="extract" onclick={() => (showExtract = true)}>Extract →</button>
      <button class="clear" onclick={() => (selected = new Set())}>clear</button>
    {/if}
  </div>

  {#if loading}
    <div class="msg">Loading…</div>
  {:else if error}
    <div class="msg err">{error}</div>
  {:else if profiles.length === 0}
    <div class="msg">No profiles in this file.</div>
  {:else}
    <div class="scroll" bind:this={scrollEl} onscroll={onScroll}>
      <table>
        <thead>
          <tr>
            <th class="c-sel"></th>
            <th class="c-name" onclick={() => toggleSort("name")}>
              name{#if sortCol === "name"}<span class="arrow">{sortDir === 1 ? " ▲" : " ▼"}</span>{/if}
            </th>
            <th class="c-len num" onclick={() => toggleSort("length")}>
              length{#if sortCol === "length"}<span class="arrow">{sortDir === 1 ? " ▲" : " ▼"}</span>{/if}
            </th>
            <th class="c-desc">description</th>
            <th class="c-act"></th>
          </tr>
        </thead>
        <tbody>
          {#if topPad > 0}<tr style="height:{topPad}px"><td colspan="5"></td></tr>{/if}
          {#each visible as p (p.index)}
            <tr class:sel={selected.has(p.index)} onclick={(e) => rowClick(e, p)}>
              <td class="c-sel"><input type="checkbox" checked={selected.has(p.index)} tabindex="-1" /></td>
              <td class="c-name" title={p.name}>{p.name}</td>
              <td class="c-len num">{p.length}</td>
              <td class="c-desc" title={p.desc ?? ""}>{p.desc ?? ""}</td>
              <td class="c-act">
                <button class="logo" title="Show this profile's sequence logo" onclick={(e) => { e.stopPropagation(); showLogo(p); }}>logo</button>
                <button class="logo pop" title="Open the logo in a new window" aria-label="Open the logo in a new window" onclick={(e) => { e.stopPropagation(); popLogo(p); }}>⧉</button>
              </td>
            </tr>
          {/each}
          {#if bottomPad > 0}<tr style="height:{bottomPad}px"><td colspan="5"></td></tr>{/if}
        </tbody>
      </table>
    </div>
    <p class="legend dim">
      Click a row to select (Ctrl/Shift for multi-select), then <b>Extract →</b> to save the chosen
      profiles as a new HMM file. Click <b>logo</b> to draw a profile's sequence logo, or
      <b>⧉</b> to open it in its own window.
    </p>
  {/if}
</div>

{#if showExtract}
  <NameModal
    title="Extract {selected.size} profile{selected.size === 1 ? '' : 's'} to a new HMM file"
    defaultName="profiles_subset.hmm"
    submitLabel="Extract"
    validate={validateFilename}
    onsubmit={(name) => doExtract(name)}
    oncancel={() => (showExtract = false)}
  />
{/if}

<style>
  .hr {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-panel);
  }
  .filter {
    padding: 3px 6px;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: var(--bg-elevated);
    color: var(--text);
    font-size: 12px;
    min-width: 200px;
  }
  .dim {
    color: var(--text-dim);
  }
  .scroll {
    flex: 1;
    overflow: auto;
  }
  table {
    border-collapse: collapse;
    font-size: 12.5px;
    width: 100%;
  }
  th,
  td {
    text-align: left;
    padding: 0 12px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    height: 26px;
    box-sizing: border-box;
  }
  thead th {
    position: sticky;
    top: 0;
    background: var(--bg-panel);
    cursor: pointer;
    user-select: none;
    font-weight: 600;
  }
  .c-desc {
    width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 0;
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .c-act {
    text-align: right;
    white-space: nowrap;
  }
  tbody tr {
    cursor: pointer;
  }
  tbody tr:hover {
    background: var(--row-hover);
  }
  tbody tr.sel {
    background: var(--row-selected);
  }
  .c-sel {
    width: 28px;
    text-align: center;
    padding: 0 4px;
  }
  .c-sel input {
    pointer-events: none;
    margin: 0;
  }
  .sel-badge {
    color: var(--accent);
    font-size: 12px;
  }
  .extract {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
    border-radius: 5px;
    padding: 2px 10px;
    font-size: 12px;
  }
  .clear {
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--text);
    border-radius: 5px;
    padding: 2px 8px;
    font-size: 12px;
  }
  .logo {
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--text);
    border-radius: 5px;
    padding: 1px 8px;
    font-size: 11px;
    cursor: pointer;
  }
  .logo:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .logo.pop {
    margin-left: 4px;
    padding: 1px 5px;
  }
  .arrow {
    color: var(--accent);
  }
  .legend {
    padding: 4px 12px 8px;
    font-size: 11px;
  }
  .msg {
    padding: 24px;
    color: var(--text-dim);
  }
  .msg.err {
    color: #e5484d;
  }
</style>
