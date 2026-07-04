<script lang="ts">
  import { listHmmProfiles, openHmmLogoWindow } from "../api";
  import { cached } from "../cache";
  import type { HmmSummary } from "../types";

  let { path }: { path: string } = $props();

  let profiles = $state<HmmSummary[]>([]);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let filter = $state("");
  let sortDir = $state<1 | -1>(1);
  let sortCol = $state<"name" | "length">("name");

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
          {#if topPad > 0}<tr style="height:{topPad}px"><td colspan="4"></td></tr>{/if}
          {#each visible as p (p.index)}
            <tr>
              <td class="c-name" title={p.name}>{p.name}</td>
              <td class="c-len num">{p.length}</td>
              <td class="c-desc" title={p.desc ?? ""}>{p.desc ?? ""}</td>
              <td class="c-act">
                <button class="logo" title="Pop out sequence logo" onclick={() => popLogo(p)}>logo ↗</button>
              </td>
            </tr>
          {/each}
          {#if bottomPad > 0}<tr style="height:{bottomPad}px"><td colspan="4"></td></tr>{/if}
        </tbody>
      </table>
    </div>
    <p class="legend dim">Click <b>logo ↗</b> to open a profile's sequence logo in a separate window.</p>
  {/if}
</div>

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
  }
  tbody tr:hover {
    background: var(--row-hover);
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
