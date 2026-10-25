<script lang="ts">
  // The report column builder (ToolDialog's `columns` role): an ordered list of
  // columns, each with its arguments. The order is the output column order. The
  // columns come from the tool's manifest; see `columns.ts`.
  import { listHas, newRow, retype, toggleListValue, type ColumnRow, type ColumnType } from "./columns";

  let { types, rows = $bindable([]) }: { types: ColumnType[]; rows: ColumnRow[] } = $props();

  // Distinct ids for the text boxes' suggestion lists, per builder instance.
  const uid = Math.random().toString(36).slice(2, 8);

  const typeOf = (row: ColumnRow) => types.find((t) => t.type === row.type);
  // A hidden type still shows in a row that already uses it (a cloned job's).
  const offered = (row: ColumnRow) => types.filter((t) => t.show || t.type === row.type);

  function add() {
    const t = types.find((x) => x.show);
    if (t) rows.push(newRow(t));
  }
  function move(i: number, by: number) {
    const j = i + by;
    if (j < 0 || j >= rows.length) return;
    [rows[i], rows[j]] = [rows[j], rows[i]];
  }
</script>

<div class="builder">
  {#each rows as row, i (i)}
    {@const t = typeOf(row)}
    <div class="row">
      <div class="main">
        <span class="num">{i + 1}</span>
        <select class="type" value={row.type} onchange={(e) => retype(row, e.currentTarget.value, types)}>
          {#if !t}<option value={row.type}>{row.type} (unknown)</option>{/if}
          {#each offered(row) as o (o.type)}<option value={o.type}>{o.label}</option>{/each}
        </select>
        <span class="spacer"></span>
        <button type="button" class="icon" title="Move up" disabled={i === 0} onclick={() => move(i, -1)}>↑</button>
        <button type="button" class="icon" title="Move down" disabled={i === rows.length - 1} onclick={() => move(i, 1)}>↓</button>
        <button type="button" class="icon x" title="Remove column" onclick={() => rows.splice(i, 1)}>×</button>
      </div>
      {#if t && t.params.length}
        <div class="params">
          {#each t.params as p (p.name)}
            <label class="param" title={p.help}>
              <span class="plabel">{p.label}</span>
              <input
                type="text"
                class:missing={!(row.params[p.name] ?? "").trim()}
                bind:value={row.params[p.name]}
                placeholder={p.kind === "list" ? `${p.help || p.label} (comma-separated)` : p.help}
                list={p.kind === "text" && p.choices.length ? `${uid}-${i}-${p.name}` : undefined}
              />
              {#if p.kind === "text" && p.choices.length}
                <datalist id={`${uid}-${i}-${p.name}`}>
                  {#each p.choices as c (c)}<option value={c}></option>{/each}
                </datalist>
              {/if}
            </label>
            {#if p.kind === "list" && p.choices.length}
              <div class="chips">
                {#each p.choices as c (c)}
                  <button
                    type="button"
                    class="chip"
                    class:on={listHas(row.params[p.name] ?? "", c)}
                    onclick={() => (row.params[p.name] = toggleListValue(row.params[p.name] ?? "", c))}
                  >{c}</button>
                {/each}
              </div>
            {/if}
          {/each}
        </div>
      {/if}
      {#if t?.help}<span class="rowhint">{t.help}</span>{/if}
    </div>
  {/each}
  <button type="button" class="add" onclick={add}>+ Add column</button>
</div>

<style>
  .builder {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .row {
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 5px 6px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .main {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .num {
    color: var(--text-dim);
    font-size: 11px;
    min-width: 1.2em;
    text-align: right;
  }
  .spacer {
    flex: 1;
  }
  select,
  input {
    padding: 3px 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg-elevated);
    color: var(--text);
    font-family: inherit;
    font-size: 12.5px;
    min-width: 0;
  }
  input.missing {
    border-color: #e5484d;
  }
  .type {
    flex: 0 1 auto;
  }
  .params {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-left: calc(1.2em + 6px);
  }
  .param {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .param input {
    flex: 1;
  }
  .plabel {
    font-size: 11.5px;
    color: var(--text-dim);
    min-width: 70px;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
    padding-left: 76px;
  }
  .chip {
    font-size: 11px;
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: var(--bg-elevated);
    color: var(--text-dim);
    cursor: pointer;
  }
  .chip.on {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .rowhint {
    color: var(--text-dim);
    font-size: 11px;
    line-height: 1.3;
    padding-left: calc(1.2em + 6px);
  }
  .icon {
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: 14px;
    line-height: 1;
    cursor: pointer;
    padding: 0 3px;
  }
  .icon:disabled {
    opacity: 0.3;
    cursor: default;
  }
  .icon.x:hover {
    color: #e5484d;
  }
  .add {
    align-self: start;
    background: var(--bg-elevated);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .add:hover {
    border-color: var(--accent);
  }
</style>
