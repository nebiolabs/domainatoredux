<script lang="ts">
  import { runTool, toolSchemaCached } from "./api";
  import { ws } from "./state.svelte";
  import { fieldAccepts, isInputRole } from "./toolInputs";
  import ColumnBuilder from "./ColumnBuilder.svelte";
  import { columnTypes, toColumns, toRows } from "./columns";
  import type { JsonSchema, JsonSchemaProp, ToolField, ToolMeta } from "./types";

  let {
    tool,
    meta,
    initial = null,
    onclose,
  }: {
    tool: string;
    meta: ToolMeta | null;
    // Pre-fill values (from a cloned job's params); overrides the seeded defaults.
    initial?: Record<string, any> | null;
    onclose: () => void;
  } = $props();

  const enumHasNull = (p: JsonSchemaProp | undefined) =>
    Array.isArray(p?.enum) && p.enum.includes(null);

  let schema = $state<JsonSchema | null>(null);
  let values = $state<Record<string, any>>({});
  let error = $state<string | null>(null);
  let running = $state(false);
  let editedOutputs = new Set<string>();

  // The current directory: outputs land here, and input dropdowns list its files.
  const folder = $derived(ws.selectedFolder ?? ws.root ?? "");
  const folderName = $derived(folder.split(/[\\/]/).pop() ?? folder);

  const DB_TYPE: Record<string, string> = { hmm: "hmm", genbank: "genbank", fasta: "fasta" };
  type Choice = { value: string; label: string };
  type ChoiceGroup = { label: string; items: Choice[] };
  // An input field's dropdown: registered databases of a matching format and
  // current-folder files of the type, as two groups in the order the tool's spec
  // (`tools/<id>.yml`, `sources`) puts them. Empty groups are dropped.
  function fileGroups(field: ToolField): ChoiceGroup[] {
    if (field.file_types.length === 0) return [];
    const workspace: ChoiceGroup = {
      label: `Workspace: ${folderName}`,
      items: ws.files.filter((f) => fieldAccepts(field, f.file_type)).map((f) => ({ value: f.path, label: f.name })),
    };
    const databases: ChoiceGroup = {
      label: "Databases",
      items: ws.databases
        .filter((d) => fieldAccepts(field, DB_TYPE[d.format] ?? d.format))
        .map((d) => ({ value: d.path, label: d.name })),
    };
    const ordered = field.sources === "databases_first" ? [databases, workspace] : [workspace, databases];
    // Dedupe by path, the first group keeping it: a file can be both in the
    // current folder AND registered as a database, which would otherwise put the
    // same value twice in the dropdown — a duplicate `{#each}` key that crashes
    // the render (dialog stuck at "Loading…").
    const seen = new Set<string>();
    return ordered
      .map((g) => ({
        label: g.label,
        items: g.items.filter((c) => (seen.has(c.value) ? false : (seen.add(c.value), true))),
      }))
      .filter((g) => g.items.length > 0);
  }
  // The dropdown's starting value, per the spec's `select`. Whatever it names,
  // when that is not on offer the first entry is used, except for `none` and
  // `active_file_or_none`, which leave it blank.
  function initialChoice(field: ToolField, groups: ChoiceGroup[], activePath: string | null): string {
    const select = field.select ?? (field.required ? "active_file" : "none");
    if (select === "none") return "";
    const offered = (p: string | null | undefined) => !!p && groups.some((g) => g.items.some((c) => c.value === p));
    if ((select === "active_file" || select === "active_file_or_none") && offered(activePath)) return activePath!;
    if (select === "active_file_or_none") return "";
    if (select.startsWith("database:")) {
      const id = select.slice("database:".length);
      const db = ws.databases.find((d) => d.managed_id === id);
      if (offered(db?.path)) return db!.path;
    }
    return groups[0]?.items[0]?.value ?? "";
  }
  function stemOf(p: string): string {
    const base = p.slice(Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")) + 1);
    const dot = base.lastIndexOf(".");
    return dot > 0 ? base.slice(0, dot) : base;
  }
  function updateOutputNames(changedField: string, selectedPath: string): void {
    const input = meta?.fields.find((field) => field.name === "input" && isInputRole(field.role));
    const primary = input && (input.name === changedField ? selectedPath : values[input.name]);
    const fallback = meta?.fields.find((field) => isInputRole(field.role)
      && (field.name === changedField ? selectedPath : values[field.name]));
    const path = primary || (fallback?.name === changedField ? selectedPath : fallback && values[fallback.name]);
    const stem = path ? stemOf(String(path)) : "output";
    for (const field of meta?.fields ?? []) {
      if (field.role === "output_file" && field.output_name && !editedOutputs.has(field.name)) {
        values[field.name] = field.output_name.replaceAll("{stem}", stem);
      }
    }
  }
  const sameValue = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
  const showValue = (v: unknown) => (v === null || v === undefined ? "unset" : Array.isArray(v) ? v.join(",") || "empty" : String(v));
  function propType(p: JsonSchemaProp | undefined): string {
    if (!p) return "string";
    return Array.isArray(p.type) ? (p.type.find((t) => t !== "null") ?? "string") : (p.type ?? "string");
  }
  const schemaProp = (name: string): JsonSchemaProp | undefined => schema?.properties?.[name];

  $effect(() => {
    void tool;
    void initial;
    error = null;
    schema = null;
    editedOutputs = new Set();
    if (!meta) {
      error = `Unknown tool "${tool}".`;
      return;
    }
    toolSchemaCached(tool)
      .then((schemaText) => {
        schema = JSON.parse(schemaText) as JsonSchema;
        const v: Record<string, any> = {};
        // Seed params from the field's default (the spec's GUI override, else the
        // CLI's). Arrays are shown as comma-separated text, except in the column
        // builder, which edits the list itself.
        for (const field of meta?.fields ?? []) {
          const d = field.default;
          if (field.role === "columns") {
            v[field.name] = toRows(d, columnTypes(field, schemaProp(field.name)));
          } else if (d !== undefined && d !== null) {
            v[field.name] = Array.isArray(d) ? d.join(",") : d;
          } else if (enumHasNull(schemaProp(field.name))) {
            v[field.name] = ""; // optional enum → blank, matching the CLI's unset default
          }
        }
        // Seed input dropdowns per the spec's `select`.
        const active = ws.tabs.find((t) => t.id === ws.activeTabId)?.file ?? null;
        let primaryInput = "";
        for (const field of meta?.fields ?? []) {
          if (isInputRole(field.role)) {
            const seeded = initial?.[field.name];
            const pick = seeded == null
              ? initialChoice(field, fileGroups(field), active?.path ?? null)
              : String(Array.isArray(seeded) ? seeded[0] ?? "" : seeded);
            v[field.name] = pick;
            // `input` names the output when it has a value; else the first input that does.
            if (pick && (field.name === "input" || !primaryInput)) primaryInput = pick;
          }
        }
        // Seed output filenames from the primary input's stem.
        const stem = primaryInput ? stemOf(primaryInput) : "output";
        for (const field of meta?.fields ?? []) {
          if (field.role === "output_file" && field.output_name) {
            v[field.name] = field.output_name.replaceAll("{stem}", stem);
          }
        }
        // Clone: overlay a prior job's params (which are in the *collected* shape —
        // output_file is a full path, input_files/arrays are arrays) onto the form.
        if (initial) {
          for (const field of meta?.fields ?? []) {
            const raw = initial[field.name];
            if (raw === undefined || raw === null) continue;
            if (field.role === "output_file") {
              v[field.name] = String(raw).split(/[\\/]/).pop() ?? "";
              editedOutputs.add(field.name);
            } else if (field.role === "input_files") {
              v[field.name] = Array.isArray(raw) ? (raw[0] ?? "") : raw;
            } else if (field.role === "columns") {
              v[field.name] = toRows(raw, columnTypes(field, schemaProp(field.name)));
            } else if (Array.isArray(raw)) {
              v[field.name] = raw.join(",");
            } else {
              v[field.name] = raw;
            }
          }
        }
        values = v;
      })
      .catch((e) => (error = String(e)));
  });

  function join(dir: string, name: string): string {
    const sep = dir.includes("\\") ? "\\" : "/";
    return `${dir}${sep}${name}`;
  }

  function collectParams(): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    for (const field of meta?.fields ?? []) {
      const val = values[field.name];
      if (field.role === "input_file") {
        if (val) out[field.name] = val;
      } else if (field.role === "input_files") {
        if (val) out[field.name] = [val]; // config expects a list
      } else if (field.role === "output_file") {
        const name = String(val ?? "").trim();
        if (name) out[field.name] = join(folder, name);
      } else if (field.role === "columns") {
        out[field.name] = toColumns(Array.isArray(val) ? val : [], columnTypes(field, schemaProp(field.name)));
      } else {
        const prop = schemaProp(field.name);
        const t = propType(prop);
        if (t === "boolean") {
          if (val === true) out[field.name] = true;
        } else if (t === "integer" || t === "number") {
          if (val !== "" && val !== undefined && val !== null) out[field.name] = Number(val);
        } else if (t === "array") {
          const list = String(val ?? "").split(",").map((s) => s.trim()).filter(Boolean);
          if (list.length) {
            const items = prop?.items?.type;
            out[field.name] = items === "integer" || items === "number" ? list.map(Number) : list;
          }
        } else {
          if (val !== "" && val !== undefined && val !== null) out[field.name] = val;
        }
      }
    }
    return out;
  }

  function run() {
    if (!ws.root) {
      error = "Open a workspace first.";
      return;
    }
    error = null;
    try {
      const params = collectParams();
      running = true;
      void runTool(tool, params, ws.root).then(() => {
        if (ws.selectedFolder) return ws.selectFolder(ws.selectedFolder);
      }).catch((e) => { ws.error = `Could not run job: ${e}`; });
      onclose();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
      running = false;
    }
  }
  /// Close, but don't silently discard a form the user has filled in — a stray
  /// click on the backdrop used to throw the whole thing away.
  function requestClose(): void {
    if (running) return;
    if (dirty && !confirmingClose) {
      confirmingClose = true;
      return;
    }
    onclose();
  }

  /// Whether the user has typed anything into the form yet.
  const dirty = $derived(Object.keys(values).length > 0);
  let confirmingClose = $state(false);
</script>

<div
  class="backdrop"
  onclick={(e) => e.target === e.currentTarget && requestClose()}
  onkeydown={(e) => e.key === "Escape" && requestClose()}
  role="presentation"
>
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Run tool">
    <header>
      <h2>{meta?.label ?? tool}</h2>
      <button class="x" onclick={onclose} aria-label="Close">×</button>
    </header>
    {#if meta}<p class="desc">{meta.description}</p>{/if}
    <p class="folder">Directory: <code>{folderName}</code></p>

    {#if error}<div class="err">{error}</div>{/if}

    {#if error && !schema}
      <!-- error banner already shown above; nothing else to render -->
    {:else if !meta || !schema}
      <div class="msg">Loading…</div>
    {:else}
      <div class="fields">
        {#each meta.fields as field (field.name)}
          {#if field.role === "columns"}
            <div class="field builder-field">
              <span class="flabel">{field.label}</span>
              <ColumnBuilder types={columnTypes(field, schemaProp(field.name))} bind:rows={values[field.name]} />
              {#if field.help}<span class="hint">{field.help}</span>{/if}
            </div>
          {:else}
          {@const prop = schemaProp(field.name)}
          <label class="field">
            <span class="flabel">{field.label}{#if field.required}<span class="req">*</span>{/if}</span>

            {#if isInputRole(field.role)}
              {@const groups = fileGroups(field)}
              {#if groups.length === 0}
                <span class="nofiles">No {field.file_types.join(" / ")} files/databases available</span>
              {:else}
                <select bind:value={values[field.name]} onchange={(e) => updateOutputNames(field.name, e.currentTarget.value)}>
                  {#if !field.required}<option value="">(none)</option>{/if}
                  {#each groups as g, gi (g.label)}
                    <!-- The bar between the two groups; `<hr>` in a `<select>` is not reliable in WebKitGTK. -->
                    {#if gi > 0}<option class="sep" disabled value="__separator__">────────────────</option>{/if}
                    <optgroup label={g.label}>
                      {#each g.items as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
                    </optgroup>
                  {/each}
                </select>
              {/if}
            {:else if field.role === "output_file"}
              <input type="text" bind:value={values[field.name]} oninput={() => editedOutputs.add(field.name)} placeholder="filename" />
            {:else if prop?.enum}
              <select bind:value={values[field.name]}>
                {#if enumHasNull(prop) || !field.required}
                  <option value="">{field.cli_default != null ? `(default: ${showValue(field.cli_default)})` : "(default)"}</option>
                {/if}
                {#each prop.enum.filter((o) => o !== null) as opt (opt)}<option value={opt}>{opt}</option>{/each}
              </select>
            {:else if propType(prop) === "boolean"}
              <input type="checkbox" bind:checked={values[field.name]} />
            {:else if propType(prop) === "integer" || propType(prop) === "number"}
              <input type="number" step="any" bind:value={values[field.name]} />
            {:else if propType(prop) === "array"}
              <input type="text" bind:value={values[field.name]} placeholder="comma, separated" />
            {:else}
              <input type="text" bind:value={values[field.name]} />
            {/if}

            {#if field.help ?? prop?.description}<span class="hint">{field.help ?? prop?.description}</span>{/if}
            {#if field.role === "param" && !sameValue(field.default, field.cli_default)}
              <span class="hint">GUI default; the command line's is {showValue(field.cli_default)}.</span>
            {/if}
          </label>
          {/if}
        {/each}
      </div>

      <footer>
        {#if confirmingClose}
          <span class="confirm">Discard this form?</span>
          <button onclick={onclose}>Discard</button>
          <button onclick={() => (confirmingClose = false)}>Keep editing</button>
        {:else}
          <button onclick={onclose}>Cancel</button>
        {/if}
        <button class="run" onclick={run} disabled={running}>{running ? "Running…" : "Run"}</button>
      </footer>
    {/if}
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.4);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }
  .dialog {
    width: min(560px, 92vw);
    max-height: 88vh;
    overflow: auto;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 12px 40px rgba(0, 0, 0, 0.4);
    padding: 16px 18px;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  h2 {
    margin: 0;
    font-size: 16px;
  }
  .x {
    border: none;
    background: none;
    font-size: 20px;
    color: var(--text-dim);
  }
  .desc {
    color: var(--text-dim);
    margin: 4px 0 4px;
  }
  .folder {
    margin: 0 0 12px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .folder code {
    font-family: var(--mono);
    color: var(--text);
  }
  .fields {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .field {
    display: grid;
    grid-template-columns: 150px 1fr;
    align-items: center;
    gap: 8px 10px;
  }
  .flabel {
    font-size: 12.5px;
    text-align: right;
  }
  .req {
    color: #e5484d;
    margin-left: 2px;
  }
  .field input[type="text"],
  .field input[type="number"],
  .field select {
    padding: 4px 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg-elevated);
    color: var(--text);
    font-family: inherit;
    font-size: 12.5px;
    width: 100%;
  }
  .field input[type="checkbox"] {
    justify-self: start;
  }
  .field option.sep {
    color: var(--text-dim);
  }
  .nofiles {
    color: #e5484d;
    font-size: 12px;
  }
  .hint {
    grid-column: 2;
    color: var(--text-dim);
    font-size: 11px;
    line-height: 1.3;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
  .run {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .err {
    background: #e5484d22;
    color: #e5484d;
    padding: 6px 10px;
    border-radius: 5px;
    margin: 8px 0;
  }
  .confirm {
    margin-right: auto;
    align-self: center;
    color: var(--text-dim);
    font-size: 12px;
  }
  .msg {
    padding: 20px;
    color: var(--text-dim);
  }
  .builder-field {
    align-items: start;
  }
</style>
