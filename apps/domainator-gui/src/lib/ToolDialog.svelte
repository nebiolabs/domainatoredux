<script lang="ts">
  import { listTools, runTool, toolSchema } from "./api";
  import { ws } from "./state.svelte";
  import type { JsonSchema, JsonSchemaProp, ToolMeta } from "./types";

  let { tool, onclose }: { tool: string; onclose: () => void } = $props();

  let meta = $state<ToolMeta | null>(null);
  let schema = $state<JsonSchema | null>(null);
  // Dynamically typed by the tool's JSON schema, so values are `any`.
  let values = $state<Record<string, any>>({});
  let error = $state<string | null>(null);
  let result = $state<string | null>(null);
  let running = $state(false);

  const activePath = $derived(ws.tabs.find((t) => t.id === ws.activeTabId)?.file.path ?? null);

  function dirOf(p: string): string {
    const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
    return i >= 0 ? p.slice(0, i) : ".";
  }
  function stemOf(p: string): string {
    const base = p.slice(Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\")) + 1);
    const dot = base.indexOf(".");
    return dot > 0 ? base.slice(0, dot) : base;
  }

  // Suggested output path when the tool writes a derived file, based on input.
  function suggestOutput(t: string, input: string): Record<string, string> {
    const dir = dirOf(input);
    const stem = stemOf(input);
    if (t === "build-ssn-viewer") return { output: `${dir}/${stem}.ssnv` };
    if (t === "transform-matrix") return { dense_text: `${dir}/${stem}.transformed.tsv` };
    return {};
  }

  function propType(p: JsonSchemaProp): string {
    return Array.isArray(p.type) ? (p.type.find((t) => t !== "null") ?? "string") : (p.type ?? "string");
  }

  $effect(() => {
    void tool;
    error = null;
    result = null;
    Promise.all([listTools(), toolSchema(tool)])
      .then(([tools, schemaText]) => {
        meta = tools.find((m) => m.id === tool) ?? null;
        const s: JsonSchema = JSON.parse(schemaText);
        schema = s;
        // Seed defaults, then prefill input + suggested output.
        const v: Record<string, unknown> = {};
        for (const [k, p] of Object.entries(s.properties ?? {})) {
          if (p.default !== undefined && p.default !== null) v[k] = p.default;
        }
        if (activePath && s.properties && "input" in s.properties) v.input = activePath;
        if (activePath) Object.assign(v, suggestOutput(tool, activePath));
        values = v;
      })
      .catch((e) => (error = String(e)));
  });

  // Build the params object: include set scalars, checked bools, non-empty arrays.
  function collectParams(): Record<string, unknown> {
    const out: Record<string, unknown> = {};
    for (const [k, p] of Object.entries(schema?.properties ?? {})) {
      const t = propType(p);
      const val = values[k];
      if (t === "boolean") {
        if (val === true) out[k] = true;
      } else if (t === "array") {
        const list = String(val ?? "")
          .split(",")
          .map((s) => s.trim())
          .filter(Boolean);
        if (list.length) out[k] = list;
      } else if (t === "integer" || t === "number") {
        if (val !== "" && val !== undefined && val !== null) out[k] = Number(val);
      } else {
        if (val !== "" && val !== undefined && val !== null) out[k] = val;
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
      result = `${r.message} → ${r.outputs.join(", ")}`;
      // Refresh the current folder so new outputs appear in the table.
      if (ws.selectedFolder) await ws.selectFolder(ws.selectedFolder);
    } catch (e) {
      error = String(e);
    } finally {
      running = false;
    }
  }

  const entries = $derived(Object.entries(schema?.properties ?? {}));
  const isRequired = (k: string) => schema?.required?.includes(k) ?? false;
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

    {#if error}<div class="err">{error}</div>{/if}
    {#if result}<div class="ok">{result}</div>{/if}

    {#if !schema}
      <div class="msg">Loading…</div>
    {:else}
      <div class="fields">
        {#each entries as [key, prop] (key)}
          {@const t = propType(prop)}
          <label class="field">
            <span class="flabel">{key}{#if isRequired(key)}<span class="req">*</span>{/if}</span>
            {#if prop.enum}
              <select bind:value={values[key]}>
                {#each prop.enum as opt (opt)}<option value={opt}>{opt}</option>{/each}
              </select>
            {:else if t === "boolean"}
              <input type="checkbox" bind:checked={values[key]} />
            {:else if t === "integer" || t === "number"}
              <input type="number" step="any" bind:value={values[key]} />
            {:else}
              <input type="text" bind:value={values[key]} placeholder={t === "array" ? "comma,separated" : ""} />
            {/if}
            {#if prop.description}<span class="hint">{prop.description}</span>{/if}
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
    margin: 4px 0 12px;
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
    font-family: var(--mono);
    font-size: 12px;
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
