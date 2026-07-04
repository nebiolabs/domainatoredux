# Plan: cross-session persistent report cache

Deferred feature. Today the GUI caches computed viewer payloads only **in memory**
(`src/lib/cache.ts`, a module singleton keyed by `command + path + params`), so they
survive tab/sub-tab switches within a session but are recomputed on the next launch.
For large inputs (e.g. an 11 MB GenBank whose enum report / summary / contig view take
seconds–minutes to build) it's worth persisting those payloads to disk so reopening a
workspace is instant.

## Where

Under the reserved hidden dir: `<root>/.domainator/cache/<namespace>/`. One file per
cached payload. `<namespace>` = the command (e.g. `enum`, `summary`, `contig`,
`matrix`).

## Cache key & invalidation

Key = hash of `(namespace, absolute source path, params)`; the cache **entry** also
stores a stamp of the source file's `(size, mtime_ns)`. On read, recompute if the
stamp doesn't match the current file (this is the same staleness contract `.didx`
uses in `io/didx.rs`). This makes edits/regenerations transparently invalidate the
cache without any explicit wiring — but we should ALSO drop entries on the paths that
`cache.ts::invalidatePath` / `delete_file` already touch, to be safe.

File layout per entry (JSON):
```json
{ "stamp": { "size": 12345, "mtime_ns": 169... }, "payload": <the command's JSON> }
```

## Backend

Add `crates/domainator-core/src/app_support/disk_cache.rs`:

```rust
pub fn cached_json<F>(root: &Path, ns: &str, source: &Path, params_key: &str, compute: F)
    -> Result<Vec<u8>, String>
where F: FnOnce() -> Result<Vec<u8>, String>
```

- Build `key = blake3/seahash(ns + source-abs + params_key)`; path
  `<root>/.domainator/cache/<ns>/<key>.json`.
- `stat(source)` → `(size, mtime_ns)`. If the entry exists and its stamp matches,
  return `payload` bytes. Else `compute()`, wrap with the current stamp, write
  atomically (temp+rename, like `workspace::write_atomic`), return payload.
- Corrupt/absent entry → just recompute.

## Wiring the commands

The expensive report commands need `workspace_root` (to locate `.domainator`) and
should return the payload as `serde_json::Value` (identical wire shape, so the
frontend TS types are unchanged):

- `run_enum_report(workspace_root, path, by, columns)` — ns `enum`, params_key =
  `by + columns`.
- `run_summary_stats(workspace_root, path)` — ns `summary`.
- `load_contig_view(workspace_root, path)` — ns `contig`.
- `load_matrix_report(workspace_root, path)` — ns `matrix`.

Each becomes: `let bytes = cached_json(root, ns, path, params_key, || { let v =
<compute the struct>; serde_json::to_vec(&v) })?; serde_json::from_slice::<Value>(&bytes)`.

`load_hmm_logo` / `list_hmm_profiles` are already fast enough (no logo pre-compute in
the new HMM table view), so they can be left uncached initially.

## Frontend

- `api.ts`: add `workspaceRoot` to the four command wrappers above.
- The viewer components (`ContigView`, `EnumReport`, `SummaryView`, `MatrixView`)
  currently take only `path`; thread `ws.root` in (either as a prop from
  `ViewerDock`, or read `ws.root` directly since they already import the store).
- Keep `cache.ts` (in-memory) as the fast first tier; the disk cache is the second
  tier hit on a cold session. `clearCache()` on workspace-open stays as-is; the disk
  cache persists across opens by design.

## Housekeeping / risks

- **Staleness is the main risk** — the `(size, mtime_ns)` stamp is the guard; verify
  it triggers on both CLI edits and in-app `set_feature_color` write-backs.
- Add a "Clear cache" affordance (delete `.domainator/cache/`) and consider an
  eviction cap (e.g. by total size / age) so it doesn't grow unbounded.
- The cache is per-payload JSON; a params change (different columns) just creates a
  new entry — acceptable.

## Verification

- Probe: compute an enum report for a fixture twice; second call returns the cached
  bytes (assert identical) and does not re-parse (time it, or add a compute counter).
- Touch the source file (bump mtime) → next call recomputes.
- Confirm the on-disk JSON round-trips to the same `Value` the uncached path returns.
