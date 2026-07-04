<script lang="ts">
  import { listTools, runTool, toolSchema } from "./api";
  import { ws } from "./state.svelte";
  import type { JsonSchema, JsonSchemaProp, ToolMeta } from "./types";

  let { tool, onclose }: { tool: string; onclose: () => void } = $props();

  let meta = $state<ToolMeta | null>(null);
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
    return [...files, ...dbs];
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
  function propType(p: JsonSchemaProp | undefined): string {
    if (!p) return "string";
    return Array.isArray(p.type) ? (p.type.find((t) => t !== "null") ?? "string") : (p.type ?? "string");
  }
  const schemaProp = (name: string): JsonSchemaProp | undefined => schema?.properties?.[name];

  $effect(() => {
    void tool;
    error = null;
    result = null;
    Promise.all([listTools(), toolSchema(tool)])
      .then(([tools, schemaText]) => {
        meta = tools.find((m) => m.id === tool) ?? null;
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

    {#if !meta || !schema}
      <div class="msg">Loading…</div>
    {:else}
      <div class="fields">
        {#each meta.fields as field (field.name)}
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
</style>
