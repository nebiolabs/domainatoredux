<script lang="ts">
  import { runTool, toolSchemaCached } from "./api";
  import { ws } from "./state.svelte";
  import type { JsonSchema, JsonSchemaProp, ToolMeta } from "./types";

  let {
    tool,
    meta,
    onclose,
  }: { tool: string; meta: ToolMeta | null; onclose: () => void } = $props();

  let schema = $state<JsonSchema | null>(null);
  let values = $state<Record<string, any>>({});
  let error = $state<string | null>(null);
  let result = $state<string | null>(null);
  let running = $state(false);

  // The current directory: outputs land here, and input dropdowns list its files.
  const folder = $derived(ws.selectedFolder ?? ws.root ?? "");
  const folderName = $derived(folder.split(/[\\/]/).pop() ?? folder);

  const DB_TYPE: Record<string, string> = { hmm: "hmm", genbank: "genbank", fasta: "fasta" };
  function typeMatches(fileType: string, ft: string): boolean {
    if (ft === "sequence") return fileType === "genbank" || fileType === "fasta";
    // domainate/domain_search accept HMM or FASTA references (protein or nucleotide).
    if (ft === "reference") return fileType === "hmm" || fileType === "fasta";
    return fileType === ft;
  }
  // Options for an input field: current-folder files of the type, plus registered
  // databases of the matching format (so HMM references can come from the DB panel).
  function fileChoices(ft: string | null): { value: string; label: string }[] {
    if (!ft) return [];
    const files = ws.files
      .filter((f) => typeMatches(f.file_type, ft))
      .map((f) => ({ value: f.path, label: f.name }));
    const dbs = ws.databases
      .filter((d) => typeMatches(DB_TYPE[d.format] ?? d.format, ft))
      .map((d) => ({ value: d.path, label: `${d.name} (db)` }));
    // Dedupe by path: a file can be both in the current folder AND registered as a
    // database, which would otherwise put the same value twice in the dropdown — a
    // duplicate `{#each}` key that crashes the render (dialog stuck at "Loading…").
    const seen = new Set<string>();
    const out: { value: string; label: string }[] = [];
    for (const c of [...files, ...dbs]) {
      if (seen.has(c.value)) continue;
      seen.add(c.value);
      out.push(c);
    }
    return out;
  }
  function stemOf(p: string): string {
    const base = p.slice(Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")) + 1);
    const dot = base.indexOf(".");
    return dot > 0 ? base.slice(0, dot) : base;
  }
  // Default output filename for a field, derived from the primary input's stem.
  function defaultOutput(toolId: string, fieldName: string, inputPath: string): string {
    const stem = inputPath ? stemOf(inputPath) : "output";
    switch (`${toolId}:${fieldName}`) {
      case "transform-matrix:dense":
        return `${stem}.transformed.hdf5`;
      case "build-ssn:xgmml":
        return `${stem}.xgmml`;
      case "build-ssn:cluster_tsv":
        return `${stem}.clusters.tsv`;
      case "build-ssn-viewer:output":
        return `${stem}.ssnv`;
      case "domain-search:output":
        return `${stem}.search.gb`;
      case "domainate:output":
        return `${stem}.annotated.gb`;
      case "select-by-cds:output":
        return `${stem}.selected_cds.gb`;
      case "select-by-contig:output":
        return `${stem}.selected.gb`;
      case "filter-domains:output":
        return `${stem}.filtered.gb`;
      case "hmmer-select:output":
        return `${stem}.selected.hmm`;
      case "hmmer-build:output":
        return `${stem}.hmm`;
      case "genbank-to-fasta:output":
        return `${stem}.fasta`;
      case "clean-sequences:output":
        return `${stem}.cleaned.gb`;
      case "sort-contigs:output":
        return `${stem}.sorted.gb`;
      case "select-by-coord:output":
        return `${stem}.bycoord.gb`;
      case "trim-contigs:output":
        return `${stem}.trimmed.gb`;
      case "extract-domains:output":
        return `${stem}.domains.gb`;
      case "extract-peptides:output":
        return `${stem}.peptides.gb`;
      case "extract-unannotated:output":
        return `${stem}.unannotated.gb`;
      case "color-genbank:output":
        return `${stem}.colored.gb`;
      case "color-table-to-legend:svg":
        return `${stem}.legend.svg`;
      case "deduplicate:output":
        return `${stem}.dedup.gb`;
      case "hmmer-compare:output":
        return `${stem}.compare.tsv`;
      case "hmmer-search:output":
        return `${stem}.searched.hmm`;
      case "hmmer-report:output":
        return `${stem}.hmm_report.tsv`;
      case "seq-dist:dense":
        return `${stem}.dist.hdf5`;
      case "compare-contigs:dense":
        return `${stem}.compare.hdf5`;
      case "enum-report:output":
        return `${stem}.enum.tsv`;
      case "matrix-report:output":
        return `${stem}.report.txt`;
      default:
        return "";
    }
  }
  const isInputRole = (role: string) => role === "input_file" || role === "input_files";

  // --- hmmer-report column builder (role "hmm_columns") ---
  // Each column is a tagged object: {type:"source"|"acc"|"desc"|"length"|"consensus"}
  // or {type:"append", name, value_type, value} for a constant column.
  type HmmCol = { type: string; name?: string; value_type?: string; value?: string };
  const HMM_COLUMN_TYPES = ["source", "acc", "desc", "length", "consensus", "append"];
  const HMM_VALUE_TYPES = ["str", "int", "float"];
  // Ensure an "append" column has its extra fields (so binds/serialization work).
  function normalizeCol(col: HmmCol) {
    if (col.type === "append") {
      if (col.name === undefined) col.name = "";
      if (col.value_type === undefined) col.value_type = "str";
      if (col.value === undefined) col.value = "";
    }
  }
  function addCol(name: string) {
    (values[name] ??= []).push({ type: "source" });
  }
  function removeCol(name: string, i: number) {
    (values[name] as HmmCol[]).splice(i, 1);
  }

  // --- enum-report column picker (role "enum_columns") ---
  // Static analyses that need no taxonomy DB (the *_lineage columns are omitted —
  // they require a taxdump that this export doesn't take).
  const ENUM_COLUMNS = [
    "domains", "domain_descriptions", "architecture", "architecture_detailed",
    "length", "start", "end", "strand", "topology", "score", "identity",
    "definition", "cds_count", "domain_count", "sequence", "taxid_self", "taxname_self",
  ];
  function toggleEnumCol(name: string, c: string) {
    const cur = (values[name] ?? []) as string[];
    values[name] = cur.includes(c) ? cur.filter((x) => x !== c) : [...cur, c];
  }
  function propType(p: JsonSchemaProp | undefined): string {
    if (!p) return "string";
    return Array.isArray(p.type) ? (p.type.find((t) => t !== "null") ?? "string") : (p.type ?? "string");
  }
  const schemaProp = (name: string): JsonSchemaProp | undefined => schema?.properties?.[name];

  $effect(() => {
    void tool;
    error = null;
    result = null;
    schema = null;
    if (!meta) {
      error = `Unknown tool "${tool}".`;
      return;
    }
    toolSchemaCached(tool)
      .then((schemaText) => {
        schema = JSON.parse(schemaText) as JsonSchema;
        const v: Record<string, any> = {};
        // Seed param defaults from the schema (arrays shown as comma-separated text).
        for (const [k, p] of Object.entries(schema.properties ?? {})) {
          if (p.default !== undefined && p.default !== null) {
            v[k] = Array.isArray(p.default) ? p.default.join(",") : p.default;
          }
        }
        // Seed input dropdowns (prefer the active file, else the first match).
        const active = ws.tabs.find((t) => t.id === ws.activeTabId)?.file ?? null;
        let primaryInput = "";
        for (const field of meta?.fields ?? []) {
          if (isInputRole(field.role)) {
            const choices = fileChoices(field.file_type);
            const pick = choices.find((c) => c.value === active?.path)?.value ?? choices[0]?.value ?? "";
            v[field.name] = pick;
            if (field.name === "input" || (!primaryInput && pick)) primaryInput = pick;
          }
        }
        // Seed output filenames from the primary input's stem.
        for (const field of meta?.fields ?? []) {
          if (field.role === "output_file" && field.required) {
            v[field.name] = defaultOutput(tool, field.name, primaryInput);
          }
        }
        // Seed the column builder with a sensible default (overrides the schema's
        // empty-array default, which the generic seeding turns into "").
        for (const field of meta?.fields ?? []) {
          if (field.role === "hmm_columns") {
            v[field.name] = [{ type: "source" }, { type: "length" }];
          }
          if (field.role === "enum_columns") {
            v[field.name] = ["domains", "architecture", "length"];
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
      } else if (field.role === "hmm_columns") {
        // Only "append" columns carry name/value_type/value; static columns must
        // send just {type} (the config's deny_unknown_fields rejects extras).
        out[field.name] = (Array.isArray(val) ? (val as HmmCol[]) : []).map((c) =>
          c.type === "append"
            ? { type: "append", name: c.name ?? "", value_type: c.value_type ?? "str", value: c.value ?? "" }
            : { type: c.type },
        );
      } else if (field.role === "enum_columns") {
        out[field.name] = Array.isArray(val) ? val : [];
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

  async function run() {
    if (!ws.root) {
      error = "Open a workspace first.";
      return;
    }
    running = true;
    error = null;
    result = null;
    try {
      const r = await runTool(tool, collectParams(), ws.root);
      result = `${r.message} → ${r.outputs.map((o) => o.split(/[\\/]/).pop()).join(", ")}`;
      if (ws.selectedFolder) await ws.selectFolder(ws.selectedFolder);
    } catch (e) {
      error = String(e);
    } finally {
      running = false;
    }
  }
</script>

<div
  class="backdrop"
  onclick={(e) => e.target === e.currentTarget && onclose()}
  onkeydown={(e) => e.key === "Escape" && onclose()}
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
    {#if result}<div class="ok">{result}</div>{/if}

    {#if error && !schema}
      <!-- error banner already shown above; nothing else to render -->
    {:else if !meta || !schema}
      <div class="msg">Loading…</div>
    {:else}
      <div class="fields">
        {#each meta.fields as field (field.name)}
          {#if field.role === "hmm_columns"}
            <div class="field builder-field">
              <span class="flabel">{field.label}</span>
              <div class="colbuilder">
                {#each (values[field.name] ?? []) as col, i (i)}
                  <div class="colrow">
                    <select bind:value={col.type} onchange={() => normalizeCol(col)}>
                      {#each HMM_COLUMN_TYPES as t (t)}<option value={t}>{t}</option>{/each}
                    </select>
                    {#if col.type === "append"}
                      <input class="cname" type="text" placeholder="column name" bind:value={col.name} />
                      <select class="cvt" bind:value={col.value_type}>
                        {#each HMM_VALUE_TYPES as t (t)}<option value={t}>{t}</option>{/each}
                      </select>
                      <input class="cval" type="text" placeholder="value" bind:value={col.value} />
                    {/if}
                    <button type="button" class="colx" title="Remove column" onclick={() => removeCol(field.name, i)}>×</button>
                  </div>
                {/each}
                <button type="button" class="addcol" onclick={() => addCol(field.name)}>+ Add column</button>
                <span class="hint">A leading <code>name</code> column is always included. <code>append</code> adds a constant column.</span>
              </div>
            </div>
          {:else if field.role === "enum_columns"}
            <div class="field builder-field">
              <span class="flabel">{field.label}</span>
              <div class="checklist">
                {#each ENUM_COLUMNS as c (c)}
                  <label class="checkitem">
                    <input
                      type="checkbox"
                      checked={(values[field.name] ?? []).includes(c)}
                      onchange={() => toggleEnumCol(field.name, c)}
                    />
                    {c}
                  </label>
                {/each}
              </div>
            </div>
          {:else}
          {@const prop = schemaProp(field.name)}
          <label class="field">
            <span class="flabel">{field.label}{#if field.required}<span class="req">*</span>{/if}</span>

            {#if isInputRole(field.role)}
              {@const choices = fileChoices(field.file_type)}
              {#if choices.length === 0}
                <span class="nofiles">No {field.file_type} files/databases available</span>
              {:else}
                <select bind:value={values[field.name]}>
                  {#if !field.required}<option value="">(none)</option>{/if}
                  {#each choices as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
                </select>
              {/if}
            {:else if field.role === "output_file"}
              <input type="text" bind:value={values[field.name]} placeholder="filename" />
            {:else if prop?.enum}
              <select bind:value={values[field.name]}>
                {#each prop.enum as opt (opt)}<option value={opt}>{opt}</option>{/each}
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

            {#if prop?.description}<span class="hint">{prop.description}</span>{/if}
          </label>
          {/if}
        {/each}
      </div>

      <footer>
        <button onclick={onclose}>Cancel</button>
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
  .ok {
    background: #06842b22;
    color: #06842b;
    padding: 6px 10px;
    border-radius: 5px;
    margin: 8px 0;
  }
  .msg {
    padding: 20px;
    color: var(--text-dim);
  }
  .builder-field {
    align-items: start;
  }
  .colbuilder {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 0;
  }
  .colrow {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .colrow select,
  .colrow input {
    padding: 3px 6px;
    border: 1px solid var(--border);
    border-radius: 5px;
    background: var(--bg-elevated);
    color: var(--text);
    font-family: inherit;
    font-size: 12.5px;
  }
  .colrow .cname,
  .colrow .cval {
    flex: 1;
    min-width: 60px;
  }
  .colx {
    border: none;
    background: none;
    color: var(--text-dim);
    font-size: 16px;
    line-height: 1;
    cursor: pointer;
    padding: 0 4px;
  }
  .colx:hover {
    color: #e5484d;
  }
  .addcol {
    align-self: start;
    background: var(--bg-elevated);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 3px 10px;
    font-size: 12px;
    cursor: pointer;
  }
  .addcol:hover {
    border-color: var(--accent);
  }
  .checklist {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
    gap: 2px 10px;
    min-width: 0;
  }
  .checkitem {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    white-space: nowrap;
  }
</style>
