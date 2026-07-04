<script lang="ts">
  import { ws } from "./state.svelte";
  import type { JobInfo } from "./types";

  let expanded = $state(true);

  const runningCount = $derived(ws.jobs.filter((j) => j.status === "running").length);

  function baseName(p: string): string {
    return p.split(/[\\/]/).pop() ?? p;
  }
  const STATUS_ICON: Record<string, string> = {
    running: "◐",
    completed: "✓",
    failed: "✕",
    cancelled: "∅",
  };
</script>

<div class="jobs">
  <div class="header">
    <button class="twisty" onclick={() => (expanded = !expanded)} aria-label="Toggle jobs">
      {expanded ? "▾" : "▸"}
    </button>
    <span class="title">Jobs</span>
    {#if runningCount > 0}<span class="running-badge">{runningCount} running</span>{:else}<span class="count">{ws.jobs.length}</span>{/if}
  </div>
  {#if expanded}
    {#if ws.jobs.length === 0}
      <div class="empty">Tool runs and report exports appear here.</div>
    {:else}
      <div class="list">
        {#each ws.jobs as job (job.job_id)}
          <div class="job status-{job.status}">
            <div class="row1">
              <span class="ic">{STATUS_ICON[job.status] ?? "•"}</span>
              <span class="tool">{job.tool}</span>
              {#if job.status === "running"}
                <button class="cancel" onclick={() => ws.cancelJob(job.job_id)} title="Cancel">×</button>
              {/if}
            </div>
            {#if job.message}<div class="msg" title={job.message}>{job.message}</div>{/if}
            {#each job.outputs as out (out)}
              {#if ws.canOpen(out)}
                <button class="out link" onclick={() => ws.openOutput(out)} title={out}>{baseName(out)}</button>
              {:else}
                <span class="out" title={out}>{baseName(out)}</span>
              {/if}
            {/each}
          </div>
        {/each}
      </div>
    {/if}
  {/if}
</div>

<style>
  .jobs {
    border-top: 1px solid var(--border);
    font-size: 12.5px;
    max-height: 40%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .header {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    background: var(--bg-panel);
    flex: none;
  }
  .twisty {
    border: none;
    background: none;
    color: var(--text-dim);
    padding: 0 2px;
    font-size: 10px;
  }
  .twisty:hover {
    background: none;
  }
  .title {
    font-weight: 600;
  }
  .count,
  .running-badge {
    color: var(--text-dim);
  }
  .running-badge {
    color: var(--accent);
    margin-left: auto;
    font-size: 11px;
  }
  .empty {
    padding: 6px 12px;
    color: var(--text-dim);
    line-height: 1.4;
  }
  .list {
    overflow: auto;
    min-height: 0;
  }
  .job {
    padding: 4px 8px 4px 12px;
    border-bottom: 1px solid var(--border);
  }
  .row1 {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ic {
    width: 12px;
    text-align: center;
  }
  .status-completed .ic {
    color: #06842b;
  }
  .status-failed .ic {
    color: #e5484d;
  }
  .status-running .ic {
    color: var(--accent);
  }
  .status-cancelled .ic {
    color: var(--text-dim);
  }
  .tool {
    font-family: var(--mono);
    font-size: 12px;
  }
  .cancel {
    margin-left: auto;
    border: none;
    background: none;
    color: var(--text-dim);
    padding: 0 4px;
    font-size: 14px;
  }
  .cancel:hover {
    background: none;
    color: #e5484d;
  }
  .msg {
    color: var(--text-dim);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding-left: 18px;
  }
  .out {
    display: block;
    padding-left: 18px;
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .out.link {
    border: none;
    background: none;
    text-align: left;
    color: var(--accent);
    cursor: pointer;
    border-radius: 0;
  }
  .out.link:hover {
    text-decoration: underline;
    background: none;
  }
</style>
