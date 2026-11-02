<script lang="ts">
  import { runTool } from "./api";
  import { parentDir, ws } from "./state.svelte";
  import { fieldAccepts, isInputRole, placeFiles, type TypedPath } from "./toolInputs";
  import ColumnBuilder from "./ColumnBuilder.svelte";
  import { columnTypes, toColumns, toRows } from "./columns";
  import type { ToolField, ToolMeta } from "./types";

  let {
    tool,
    meta,
    initial = null,
    files = [],
    onclose,
  }: {
    tool: string;
    meta: ToolMeta | null;
    // A cloned job's params; overrides the seeded defaults.
    initial?: Record<string, any> | null;
    // The selected files (`ws.toolTargets`), placed in the inputs that fit them
    // (`placeFiles`). An input left without one starts on its spec's `select`.
    files?: TypedPath[];
    onclose: () => void;
  } = $props();

  let values = $state<Record<string, any>>({});
  let error = $state<string | null>(null);
  let running = $state(false);
  let editedOutputs = new Set<string>();
  // Selected files that fit no free input, named under the directory line.
  let leftOut = $state<string[]>([]);

  // The current directory: outputs land here. Input dropdowns list the files of
  // every folder selected in the tree, of which this is the last picked.
  const folder = $derived(ws.selectedFolder ?? ws.root ?? "");
  const folderName = $derived(folder.split(/[\\/]/).pop() ?? folder);

  const DB_TYPE: Record<string, string> = { hmm: "hmm", genbank: "genbank", fasta: "fasta" };
  /// What an empty dropdown says it lacks, for the registry-only types whose ids
  /// read badly; the model downloads are in the Reference databases panel.
  const EMPTY_LABEL: Record<string, string> = {
    foldseek: "No foldseek databases (add one in the Databases panel)",
    prostt5: "No ProstT5 model (download it in Reference databases)",
    prott5: "No ProtT5 model (download it in Reference databases)",
  };
  type Choice = { value: string; label: string };
  type ChoiceGroup = { label: string; items: Choice[] };
  // An input field's dropdown: registered databases of a matching format and
  // current-folder files of the type, as two groups in the order the tool's spec
  // (`tools/<id>.yml`, `sources`) puts them. Empty groups are dropped.
  function fileGroups(field: ToolField): ChoiceGroup[] {
    if (field.file_types.length === 0) return [];
    // Several listed folders can hold files of the same name, so then each entry
    // names its folder.
    const multi = ws.selectedFolders.length > 1;
    const workspace: ChoiceGroup = {
      label: multi ? `Workspace: ${ws.selectedFolders.length} folders` : `Workspace: ${folderName}`,
      items: ws.files
        .filter((f) => fieldAccepts(field, f.file_type))
        .map((f) => ({ value: f.path, label: multi ? `${ws.folderLabel(parentDir(f.path))}/${f.name}` : f.name })),
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
  // The starting value of a dropdown no selected file was placed in, per the
  // spec's `select`. A `database:<id>` that is not on offer falls back to the
  // first entry.
  function initialChoice(field: ToolField, groups: ChoiceGroup[]): string {
    const select = field.select ?? (field.required ? "first" : "none");
    if (select === "none") return "";
    const offered = (p: string | null | undefined) => !!p && groups.some((g) => g.items.some((c) => c.value === p));
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
  /// An input field's first chosen path: `input_files` holds a list of them.
  function firstPath(name: string): string {
    const v = values[name];
    return String((Array.isArray(v) ? v.find(Boolean) : v) ?? "");
  }
  function updateOutputNames(): void {
    const input = meta?.fields.find((field) => field.name === "input" && isInputRole(field.role));
    const fallback = meta?.fields.find((field) => isInputRole(field.role) && firstPath(field.name));
    const path = (input && firstPath(input.name)) || (fallback && firstPath(fallback.name));
    const stem = path ? stemOf(String(path)) : "output";
    for (const field of meta?.fields ?? []) {
      if (field.role === "output_file" && field.output_name && !editedOutputs.has(field.name)) {
        values[field.name] = field.output_name.replaceAll("{stem}", stem);
      }
    }
  }
  const sameValue = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
  const showValue = (v: unknown) => (v === null || v === undefined ? "unset" : Array.isArray(v) ? v.join(",") || "empty" : String(v));
  const isNumber = (field: ToolField) => field.value_type === "integer" || field.value_type === "number";

  $effect(() => {
    void tool;
    void initial;
    const placement = meta ? placeFiles(meta, files) : { assigned: {}, left: [] };
    leftOut = placement.left.map((f) => f.path.split(/[\\/]/).pop() ?? f.path);
    error = null;
    editedOutputs = new Set();
    if (!meta) {
      error = `Unknown tool "${tool}".`;
      return;
    }
    const v: Record<string, any> = {};
    // Seed params from the field's default (the spec's GUI override, else the
    // CLI's). Arrays are shown as comma-separated text, except in the column
    // builder, which edits the list itself.
    for (const field of meta?.fields ?? []) {
      const d = field.default;
      if (field.role === "columns") {
        v[field.name] = toRows(d, columnTypes(field));
      } else if (d !== undefined && d !== null) {
        v[field.name] = Array.isArray(d) ? d.join(",") : d;
      } else if (field.choices.length) {
        v[field.name] = ""; // optional choice → blank, matching the CLI's unset default
      }
    }
    // Seed input dropdowns with the placed files, else per the spec's `select`.
    let primaryInput = "";
    for (const field of meta?.fields ?? []) {
      if (isInputRole(field.role)) {
        const seeded = initial?.[field.name] ?? placement.assigned[field.name];
        const picks: string[] = seeded == null
          ? [initialChoice(field, fileGroups(field))]
          : (Array.isArray(seeded) ? seeded : [seeded]).map(String);
        const pick = picks.find(Boolean) ?? "";
        // `input_files` is a list of dropdowns, one per file.
        v[field.name] = field.role === "input_files" ? (picks.length ? picks : [""]) : pick;
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
          const list = (Array.isArray(raw) ? raw : [raw]).map(String);
          v[field.name] = list.length ? list : [""];
        } else if (field.role === "columns") {
          v[field.name] = toRows(raw, columnTypes(field));
        } else if (Array.isArray(raw)) {
          v[field.name] = raw.join(",");
        } else {
          v[field.name] = raw;
        }
      }
    }
    values = v;
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
        const list = (Array.isArray(val) ? val : [val]).filter(Boolean);
        if (list.length) out[field.name] = list;
      } else if (field.role === "output_file") {
        const name = String(val ?? "").trim();
        if (name) out[field.name] = join(folder, name);
      } else if (field.role === "columns") {
        out[field.name] = toColumns(Array.isArray(val) ? val : [], columnTypes(field));
      } else {
        if (field.kind === "flag") {
          if (val === true) out[field.name] = true;
        } else if (field.multiple) {
          const list = String(val ?? "").split(",").map((s) => s.trim()).filter(Boolean);
          if (list.length) out[field.name] = isNumber(field) ? list.map(Number) : list;
        } else if (isNumber(field)) {
          if (val !== "" && val !== undefined && val !== null) out[field.name] = Number(val);
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
        return ws.refreshFolder();
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
    {#if leftOut.length}
      <p class="left-out">
        {leftOut.length === 1 ? "1 selected file doesn't" : `${leftOut.length} selected files don't`} fit any input:
        {leftOut.join(", ")}
      </p>
    {/if}

    {#if error}<div class="err">{error}</div>{/if}

    {#if !meta}
      <!-- error banner already shown above; nothing else to render -->
    {:else}
      <div class="fields">
        {#each meta.fields as field (field.name)}
          {#if field.role === "columns"}
            <div class="field builder-field">
              <span class="flabel">{field.label}</span>
              <!-- The first render comes before the seeding effect: `bind:` to an
                   unset value throws, and the dead dialog's backdrop eats every click. -->
              {#if Array.isArray(values[field.name])}
                <ColumnBuilder types={columnTypes(field)} bind:rows={values[field.name]} />
              {/if}
              {#if field.help}<span class="hint">{field.help}</span>{/if}
            </div>
          {:else}
          <label class="field">
            <span class="flabel">{field.label}{#if field.required}<span class="req">*</span>{/if}</span>

            {#if isInputRole(field.role)}
              {@const groups = fileGroups(field)}
              {#if groups.length === 0}
                <span class="nofiles">{EMPTY_LABEL[field.file_types[0]] ?? `No ${field.file_types.join(" / ")} files/databases available`}</span>
              {:else}
                {#snippet options()}
                  {#if !field.required || field.role === "input_files"}<option value="">(none)</option>{/if}
                  {#each groups as g, gi (g.label)}
                    <!-- The bar between the two groups; `<hr>` in a `<select>` is not reliable in WebKitGTK. -->
                    {#if gi > 0}<option class="sep" disabled value="__separator__">────────────────</option>{/if}
                    <optgroup label={g.label}>
                      {#each g.items as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
                    </optgroup>
                  {/each}
                {/snippet}
                {#if field.role === "input_files" && Array.isArray(values[field.name])}
                  <div class="multi">
                    {#each values[field.name] as _, i (i)}
                      <div class="multi-row">
                        <select bind:value={values[field.name][i]} onchange={() => queueMicrotask(updateOutputNames)}>
                          {@render options()}
                        </select>
                        {#if values[field.name].length > 1}
                          <button
                            type="button"
                            class="rm"
                            title="Remove this file"
                            aria-label="Remove this file"
                            onclick={(e) => {
                              e.preventDefault();
                              values[field.name].splice(i, 1);
                              updateOutputNames();
                            }}>✕</button
                          >
                        {/if}
                      </div>
                    {/each}
                    <button
                      type="button"
                      class="add"
                      onclick={(e) => {
                        e.preventDefault();
                        values[field.name].push("");
                      }}>＋ Add file</button
                    >
                  </div>
                {:else}
                  <select bind:value={values[field.name]} onchange={() => queueMicrotask(updateOutputNames)}>
                    {@render options()}
                  </select>
                {/if}
              {/if}
            {:else if field.role === "output_file"}
              <input type="text" bind:value={values[field.name]} oninput={() => editedOutputs.add(field.name)} placeholder="filename" />
            {:else if field.choices.length && !field.multiple}
              <select bind:value={values[field.name]}>
                {#if !field.required}
                  <option value="">{field.cli_default != null ? `(default: ${showValue(field.cli_default)})` : "(default)"}</option>
                {/if}
                {#each field.choices.map(String) as opt (opt)}<option value={opt}>{opt}</option>{/each}
              </select>
            {:else if field.kind === "flag"}
              <input type="checkbox" bind:checked={values[field.name]} />
            {:else if field.multiple}
              <input type="text" bind:value={values[field.name]} placeholder="comma, separated" />
            {:else if isNumber(field)}
              <input
                type="number"
                step={field.value_type === "integer" ? 1 : "any"}
                min={field.min ?? field.exclusive_min ?? undefined}
                max={field.max ?? undefined}
                bind:value={values[field.name]}
              />
            {:else}
              <input type="text" bind:value={values[field.name]} />
            {/if}

            {#if field.help}<span class="hint">{field.help}</span>{/if}
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
  .left-out {
    margin: -8px 0 12px;
    font-size: 12px;
    color: var(--text-dim);
    font-style: italic;
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
  .multi {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .multi-row {
    display: flex;
    gap: 4px;
    align-items: center;
  }
  .multi-row select {
    flex: 1;
    min-width: 0;
  }
  .rm,
  .add {
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--text-dim);
    border-radius: 4px;
    font-size: 11.5px;
    padding: 2px 7px;
    cursor: pointer;
  }
  .add {
    align-self: flex-start;
  }
  .rm:hover,
  .add:hover {
    color: var(--text);
    border-color: var(--accent);
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
  .builder-field {
    align-items: start;
  }
</style>
