# The differential parity harness

## Why it exists

The port's correctness contract is parity with the Python tool, and upstream's
pytest suite is the best catalogue we have of *which invocations matter* — far
more flag combinations than anyone would think to write by hand, and it grows
with upstream. But porting their **assertions** is mostly wasted work: they
assert on Python-side internals, and re-deriving each expectation in Rust would
bake our own misunderstandings into the test.

So the harness ports the **invocations** instead. It mines each `main([...])`
call out of upstream's tests, replays the same argument list through both
implementations into two scratch directories, and compares every file the two
runs produced. Upstream's comparator (`test/helpers.py`) is imported, not copied,
so it can never drift.

That reversal is what makes it productive: one pass over 282 invocations found a
real `--max_hits` ranking bug, independently reproduced a known `hmmer-pure-rs`
issue, and turned up eight groups of genuine feature gaps — while correctly
classifying 111 runs as tolerated numeric differences rather than drowning the
signal in false positives.

## Running it

Needs the conda environment and the vendored upstream clone, so it is **not**
part of the CI check job. A full sweep takes about six minutes.

```bash
cargo build                                    # the harness runs target/debug/domainator
scripts/parity/differential.py                 # full sweep; exits non-zero on a new gap
scripts/parity/differential.py --only domainate --only domain_search -v
scripts/parity/differential.py --report /tmp/report.tsv   # every invocation, not just gaps
```

`scripts/sync_upstream.sh` runs it when the conda environment is present.

### External binaries

`deduplicate_genbank`'s invocations shell out to `cd-hit`, `usearch` or `diamond`,
and `seq_dist`'s `diamond_*` ones to `diamond`. When a binary is missing **Python
fails first**, so the harness classifies the invocation `skipped: python rejected
the invocation` — an `OK` outcome. A sweep run without them is therefore green
while covering nothing, which is exactly how five broken cd-hit invocations sat
invisible until the backend was written. Put the conda environment's `bin` on
`PATH` (it carries all three), and point `$DOMAINATOR_CDHIT` at a `cdhit` binary
so the Rust side can find one too:

```bash
PATH=/path/to/conda/envs/domainator/bin:$PATH DOMAINATOR_CDHIT=/path/to/cdhit \
  scripts/parity/differential.py --only deduplicate_genbank -v
```

`deduplicate_genbank` should report 8 identical and 1 skipped. If it reports more
skips, a binary is missing rather than anything being fine.

## The pieces

| path | role |
|---|---|
| `scripts/parity/harvest_invocations.py` | AST-walks upstream's tests, emits one TSV row per `main([...])` |
| `scripts/parity/differential.py` | replays each row through both implementations and classifies the result |
| `scripts/parity/toolmap.tsv` | `<python module>` → `<domainator subcommand>` |
| `scripts/parity/known_gaps.tsv` | the tracked list of invocations that do not yet reproduce |
| `scripts/parity/sync_fixtures.py` | keeps `test/data/` in step with upstream's `test/data` |
| `test/data/` | the committed fixture subset (see its `README.md`) |

Three placeholders carry through the harvest and are substituted at replay time:
`@DATA@` (the fixture directory, upstream's `shared_datadir` fixture), `@TMP@` (a
scratch directory, **distinct per implementation** — which is why upstream's
`output_dir = "test_out"` is deliberately not allowed to shadow it), and `@OUT@`
(a fallback for an output path whose binding would not resolve).

## Outcomes

Worst-wins across an invocation's output files:

| outcome | meaning |
|---|---|
| `identical` | byte-for-byte equal |
| `tolerated` | equal under the semantic comparator; bytes may differ |
| `differs` | a real divergence |
| `rust-failed` | the Rust tool exited non-zero where Python succeeded |
| `error` | timeout or harness failure |
| `skipped` | Python itself rejected the invocation, or it wrote no files |

`tolerated` is the interesting one. Compatibility is *semantic*, not byte-identical
(see `CLAUDE.md`), and search numerics are explicitly not a parity target —
`hmmer-pure-rs` is a different HMMER3 implementation. But a `tolerated` row still
names a file whose bytes differ, and **reading those is worthwhile**: the
qualifier-line-wrapping divergence surfaced that way, invisible to every gate
because `compare_seqfiles` reads both files back and gets equal values.

Per file type the comparator is: GenBank → upstream's `compare_seqfiles`, **plus**
a feature-coordinate/type and LOCUS-name check the upstream comparator does not
do (that blind spot is where the origin-crossing slice bugs lived); HDF5 →
`DataMatrix` + `np.allclose`; JSON/`.dsnv` → `scripts/json_parity.py` (parsed, not
bytes — `serde_json` is built with `preserve_order` and Python sorts keys); XGMML
→ parse-equal node/edge/att sets; `.hmm` → text with `DATE`/`COM` normalized out;
everything else → exact text.

## `known_gaps.tsv` is the to-do list, and it is enforced

Every invocation that is not `identical`/`tolerated`/`skipped` must appear in
`scripts/parity/known_gaps.tsv`. An ordinary run exits non-zero whenever reality
and that file disagree **in either direction** — a new divergence fails the gate,
and a fixed one cannot linger there as a stale entry claiming work that is done.

```bash
scripts/parity/differential.py --update_baseline   # after an intended change
```

Membership and the `outcome` column are machine-maintained. The `class` column is
written **by hand** and carried across regenerations, because no script can tell a
deliberate deferral from a bug:

| class | meaning |
|---|---|
| `deferred:<tag>` | out of scope on purpose, and recorded as such in `UPSTREAM.md` |
| `gap:<tag>` | a real missing feature — the actual to-do list |
| `upstream:<tag>` | the divergence is in a dependency, not in this port |
| `harness:<tag>` | an artifact of how the invocation is replayed |

A newly-appearing row arrives with an empty `class`; `--update_baseline` says how
many need triage. Group several rows under one tag when they share a root cause —
the tags are what makes the list read as a handful of problems rather than fifty
failures.

## Keeping in sync when the upstream pin moves

`scripts/sync_upstream.sh` fast-forwards the clone and prints the changelog. After
advancing the pin:

```bash
scripts/parity/sync_fixtures.py --check       # did a fixture change, or a new one appear?
scripts/parity/sync_fixtures.py --update      # copy them over; --prune drops unused ones
scripts/parity/differential.py --update_baseline
git diff scripts/parity/known_gaps.tsv        # <- read this
```

That diff is the point of the exercise. New rows are upstream features the port
has not caught up to; disappearing rows are things upstream stopped testing (or
that a fixture change quietly stopped exercising) and deserve as much suspicion as
new ones. Triage the `class` column on anything new before committing.

Because the stable key is `<test_fn>#<n>` — the n-th invocation inside that test
function — and *not* a line number, upstream reformatting a test file does not
churn the list. Renaming a test function or inserting an invocation ahead of
another in the same function does, showing up as one row removed and one added.

## Migrating an invocation into a native `cargo test`

The harness is a gate, not a test suite: it needs conda and the clone, so CI
cannot run it. When it finds a bug, the fix belongs behind a **fixture-free**
`cargo test` in the module that owns the logic — that is what actually guards the
invariant afterwards. The origin-crossing slice tests in
`crates/domainator-core/src/tools/neighborhood.rs` are the model: the harness
found the bug, and `merge_touching_parts_cases` and friends are what keep it
fixed, with the expected values taken from Python and quoted in the test.

Work backwards from the harness row:

1. Reproduce it alone: `scripts/parity/differential.py --only <module> -v`.
2. Get the argument list: `grep <test_fn> <(scripts/parity/harvest_invocations.py)`.
3. Run both by hand and diff the outputs, to find *which* value is wrong rather
   than that some file differs.
4. Write the `cargo test` against the smallest pure function that gets it wrong,
   with the expected value computed in Python and the derivation in a comment.
5. Fix, then `--update_baseline` so the row leaves `known_gaps.tsv`.

Use a `test/data/` fixture only when the logic genuinely needs a real file. Most
of these bugs live in pure coordinate or formatting code that takes a literal.

## When an invocation is being skipped

The harvester drops any invocation where an argument will not resolve to a
literal, and reports `skipped: python rejected the invocation` when the replayed
command fails on the Python side too — usually because it depends on a file an
earlier step of the same test function produced, which the harness does not
replay. Both are deliberate: a half-resolved command line is worse than a missing
one.

If a newly interesting test is being skipped, widen `Resolver.render` in
`harvest_invocations.py` — it currently handles constants, local bindings,
`str()`, pathlib `/`, `+`, and f-strings over any of those. Each addition is a few
lines. Multi-step tests (build a file, then consume it) would need the harness to
replay a whole test function as a sequence, which it deliberately does not do.

Upstream's tests also lean on argparse accepting any unambiguous option prefix
(`--o` for `--output`); clap does not, so `expand_abbreviations` resolves them
against the Rust tool's own `--help` before running. An abbreviation that is *not*
unique on our side is left alone, so a genuinely absent flag still reports as one.
