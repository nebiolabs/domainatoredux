# Troubleshooting

## "diamond not found" / "cdhit not found" / "usearch not found"

`seq-dist`'s default algorithm (`diamond_us`) and `deduplicate`'s default (`cd-hit`) both
call an external program. Either install it, or switch to an algorithm that needs nothing:

```bash
domainator seq-dist -i in.fasta --dense out.hdf5 --algorithm hmmer
domainator deduplicate -i in.fasta -o out.fasta --algorithm hash --id 1
```

The built-in `hmmer` algorithm is a complete HMMER3 implementation — slower than DIAMOND
on large datasets, but always available. The `hash` deduplication backend does exact
duplicate removal only, which is why it requires `--id 1`.

To install the binaries, the easiest route is the **External tools** panel in the
[desktop app](gui.md#external-tools). Otherwise put the program on your `PATH`, or point
at it explicitly:

```bash
domainator seq-dist ... --bin_path /opt/diamond/diamond      # per run
export DOMAINATOR_DIAMOND=/opt/diamond/diamond               # for the session
```

The search order is `--bin_path` → `$DOMAINATOR_DIAMOND` / `$DOMAINATOR_CDHIT` /
`$DOMAINATOR_USEARCH` → a bundled copy under `external/` → the bare name on `PATH`.

**On Windows you need a Windows binary.** A `diamond` built for Linux under WSL2 will not
run from a native Windows build.

## `deduplicate` with cd-hit complains about the molecule type

cd-hit ships as two programs: `cd-hit` for protein and `cd-hit-est` for nucleotide.
Domainator picks based on the first record's molecule type. If you point `--bin_path` at
a binary that cannot serve the request — protein `cd-hit` for nucleotide input — it
prefers the sibling executable next to it, and errors if there isn't one rather than
clustering DNA as if it were protein.

Point `--bin_path` at the `cdhit` multiplexer binary (the Rust port, which dispatches on
its first argument) and this resolves itself.

## "HTML output is not supported in this port"

`enum-report --html` and `hmmer-report --html` are deliberately not implemented, and error
rather than silently writing something else. Use TSV (`-o`) or JSON (`--json`):

```bash
domainator enum-report -i in.gb --domains --length -o table.tsv
```

The same applies to `matrix-report` and `summary-report`. For the visual version, the
[desktop app](gui.md#viewers) has live Enum Report, Summary, and Matrix Report tabs.

## "sparse distance matrices not implemented"

`transform-matrix --mode score_dist` cannot write `--sparse`, because a distance matrix's
zeros are meaningful and the sparse representation cannot express them. Write it dense:

```bash
domainator transform-matrix -i in.hdf5 --dense out.hdf5 --mode score_dist
```

## A `.gz` file is rejected

Domainator's indexed I/O needs to seek into a compressed file, which plain gzip cannot do.
Only **BGZF** (block gzip) is accepted. BGZF files are readable by any gzip reader, so
recompressing costs you nothing:

```bash
domainator format-db -i sequences.fasta --output_dir seq_db --compress --index
```

See [formatted databases](file-formats.md#formatted-databases).

## "both an unsharded file and shards are present"

A formatted database directory contains either a single file or a set of shards, not both.
Domainator refuses to guess which you meant. Remove or move the one you don't want.

## A stale `.didx` index

Indexes record the size and modification time of the file they describe, and a mismatch is
an error rather than a warning, because a stale index returns wrong records. Rebuild it:

```bash
domainator format-db -i sequences.fasta --output_dir seq_db --index --force
```

## `missing required 'input' (-i / --input or config)`

This is the "you forgot an argument" error. It is reported by the tool rather than as a
usage message because the value could legitimately have come from a `--config` file, so it
cannot be checked until after the merge. `domainator <tool> --help` lists what is needed.

## A boolean in my config file won't turn off

Booleans from a config file are OR'd with the command line, so a `true` in the file cannot
be overridden to `false` by a flag. Remove it from the config file and pass it explicitly
when you want it. See [config files](config-and-agents.md#how-a-config-file-and-the-command-line-combine).

## Scores differ slightly from HMMER or pyhmmer

Domainator uses `hmmer-pure-rs`, an independent Rust implementation of HMMER3. Which
domains hit which sequences, their coordinates, and which network edges exist all agree;
E-values and bit scores can differ in the last digits. If you need numerically identical
results to a published HMMER run, run HMMER itself and use
[`domainate`](cli-reference.md#domainate) on the annotated output.

Similarly, profiles from [`hmmer-build`](cli-reference.md#hmmer-build) are valid and
re-readable by pyhmmer but their emissions are not byte-identical to `hmmbuild`'s.

## A piped command fails partway through a large input

Piped input is spilled to a temporary file first, so a read-only or very small `TMPDIR`
will fail on large inputs. Point it somewhere with room:

```bash
TMPDIR=/scratch/tmp domainator select-by-contig --domains Cas9 < big.gb > hits.gb
```

## The run is slower than expected

Use a release build if you built from source — a debug build is several times slower:

```bash
cargo build --release -p domainator-cli
```

Then check threading. `--cpu 0` (the default) uses all cores, but only `domainate`,
`domain-search`, `deduplicate`, `seq-dist`, `compare-contigs`, `hmmer-compare` and
`hmmer-search` are threaded. For large all-vs-all comparisons, DIAMOND is far faster than
the built-in `hmmer` algorithm.

If a network is too dense to lay out, prune it rather than raising the cutoff until it
fragments:

```bash
domainator transform-matrix -i dist.hdf5 --dense pruned.hdf5 --mst_knn 5
```

## The desktop app won't launch (macOS / Linux)

The bundles are unsigned. On macOS, open it once with right-click → Open, or:

```bash
xattr -dr com.apple.quarantine /Applications/Domainator.app
```

## Canceling a job in the app doesn't stop it

Known limitation: cancel stops a job that hasn't started, but a tool already running will
finish. See [the app's limitations](gui.md#current-limitations).

## Still stuck

Open an issue at
<https://github.com/nebiolabs/domainatoredux/issues>. From the desktop app, **Help →
About** has a copyable diagnostics blob (versions, platform, installed external tools) —
please include it. For the command line, include `domainator --version` and the full
command you ran.
