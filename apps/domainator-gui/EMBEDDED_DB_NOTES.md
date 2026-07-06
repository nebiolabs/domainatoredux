# Investigation: a lightweight embedded DB in `.domainator/`?

**Question:** would an embedded DB (rusqlite / turso / redb / …) help, in addition to
or instead of, the `.json` files under `.domainator/`?

**Short answer:** Not as a replacement for the small JSON singletons. There *is* a
real case for one embedded store — but only for the **derived / high-cardinality
cache-like data** (per-file metadata sidecars, record counts, and the deferred
report cache), and if we do it we should use a **pure-Rust** store (redb), not
rusqlite. It's worth doing **together with** `PERSISTENT_CACHE_PLAN.md`, when a large
workspace actually feels slow — not urgent today.

## What's stored under `.domainator/` today

| Data | Form | Cardinality | Access | Human/Python-readable matters? |
|---|---|---|---|---|
| `workspace.json` (settings) | one JSON | 1 | read on open, write rare | nice-to-have |
| `databases.json` (registry) | one JSON | 1 (small list) | read on open + register | nice-to-have |
| `jobs/<id>/job.json` (provenance) | JSON per job | grows with jobs | write once, read in Jobs panel | **yes** (audit trail) |
| `meta/<rel>.dom.json` (description/color/created_by_job) | JSON per annotated file | **one per file** | **`FileMeta::load` per file in every `list_files`** | no (GUI-only) |
| `cache/…` (report cache) | deferred | one per payload | — | no (derived) |
| record counts | in-memory only (`RecordCountState`) | per browsed file | recomputed next session | no (derived) |
| sequence cache | in-memory only (`SeqCacheState`) | one file | recomputed next session | no (derived) |

## Where a DB helps

1. **Sidecar sprawl + per-file listing I/O.** `list_files` calls `FileMeta::load`
   for *every* file → one `fs::read` attempt per row (`workspace.rs`). A workspace
   that has annotated many files also accumulates one `.dom.json` per file. A single
   store keyed by absolute path returns a folder's metadata in **one query** and
   removes the file sprawl. This is the strongest case.
2. **Derived caches (record counts + the deferred report cache).** A table of
   `(path, size, mtime_ns, payload)` with SQL/`range` invalidation and a size cap is
   exactly the shape `PERSISTENT_CACHE_PLAN.md` wants, and it would let the
   record-count / sequence caches **persist across sessions** instead of the current
   in-memory maps. Cleaner than spraying `cache/*.json`.
3. **Concurrency.** Detached sequence windows + background jobs can touch
   `.domainator` at once; a transactional store is safer than racing JSON writes.
   (Low incidence today, but a latent hazard.)

## Where a DB does NOT help

- **The singletons** (`workspace.json`, `databases.json`): tiny, read-once,
  human-editable, and inspectable outside the app. JSON is strictly simpler here.
- **`job.json` provenance**: append-mostly and valuable *because* it's a readable
  audit trail. Keep as JSON.
- **Transparency.** The whole project deliberately keeps user data in plain,
  Python-readable files and quarantines app machinery under one hidden dir. A DB is
  opaque — acceptable for *derived* data under `.domainator`, but a reason not to
  migrate anything user-facing into it.

## Which DB (if we do it)

The project already made the relevant call once: it **vendored a pure-Rust
`hdf5-pure`** specifically to avoid a C dependency (the only non-pure runtime dep is
the external `diamond` binary). That ethos — pure Rust, painless Windows
cross-compilation — should decide this too.

- **redb** — pure-Rust, embedded, ACID, single file. **Recommended** if we add one:
  consistent with the vendored-hdf5 decision, no C toolchain, trivial cross-compile.
  Key-value/table API (no SQL), which is plenty for path-keyed caches.
- **rusqlite (bundled SQLite)** — mature and SQL-rich, but reintroduces a **C build
  dependency**, undoing the reason `hdf5-pure` was vendored and adding Windows
  cross-compile friction. Avoid unless we genuinely need SQL queries.
- **turso (libSQL Rust rewrite)** — pure-Rust and SQLite-compatible, but **alpha**;
  too immature to depend on now. Revisit later.
- **sled** — pure-Rust KV but effectively stalled/beta; not recommended.

## Recommendation

- **Keep** `workspace.json`, `databases.json`, and `job.json` as JSON.
- **Don't add a DB now.** In-memory caches already cover within-session speed, and
  the sprawl/cold-start pain only shows up at scale.
- **When** a large workspace feels slow (slow folder listing, or slow cold re-opens),
  add a single **redb** store at `.domainator/cache.redb` that folds together: (a)
  per-file metadata, (b) record counts, (c) the report cache from
  `PERSISTENT_CACHE_PLAN.md`. Treat it as a **rebuildable cache** (deletable; regenerated
  on demand) with `(size, mtime_ns)` stamps — same staleness contract as `.didx`.
  Provide a "Clear cache" affordance and an eviction cap.
- Do it **as one piece of work with the persistent-cache plan**, not piecemeal.
