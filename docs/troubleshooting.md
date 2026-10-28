# Troubleshooting

## "diamond not found" / "cdhit not found" / "usearch not found"

`seq_dist`'s default algorithm (`diamond_us`) and `deduplicate_genbank`'s default (`cd-hit`) both
call an external program. `domainatoredux doctor` shows which of them it can find, and where
it looked. Either install the missing one, or switch to an algorithm that needs nothing:

```bash
domainatoredux seq_dist -i in.fasta -o out.hdf5 --algorithm hmmer
domainatoredux deduplicate_genbank -i in.fasta --fasta_out out.fasta --algorithm hash --identity 1
```

The built-in `hmmer` algorithm is a complete HMMER3 implementation — slower than DIAMOND
on large datasets, but always available. The `hash` deduplication backend does exact
duplicate removal only, which is why it requires `--identity 1`.

To install the binaries, the easiest route is the **External tools** panel in the
[desktop app](gui.md#external-tools). Otherwise put the program on your `PATH`, or point
at it explicitly:

```bash
domainatoredux seq_dist ... --diamond_path /opt/diamond/diamond  # per run
export DOMAINATOREDUX_DIAMOND=/opt/diamond/diamond               # for the session
```

The search order is the tool's `--diamond_path`/`--cdhit_path`/`--usearch_path` → `$DOMAINATOREDUX_DIAMOND` / `$DOMAINATOREDUX_CDHIT` /
`$DOMAINATOREDUX_USEARCH` → a bundled copy under `external/` → the bare name on `PATH`.

**On Windows you need a Windows binary.** A `diamond` built for Linux under WSL2 will not
run from a native Windows build.

## "orphos not found", or orphos is too old

`gene_call`, and `--gene_call` on `domainate` and `domain_search`, run
[Orphos](https://github.com/FullHuman/orphos) (a Rust reimplementation of Prodigal) to
predict genes. Install it from the **External tools** panel, or download a release and set
`$DOMAINATOREDUX_ORPHOS` (or pass `gene_call --orphos_path`). It must be version 0.4.0 or newer:
older releases ignored the genetic-code flag, so Domainator refuses them rather than
calling genes with the wrong table.

Orphos publishes no Intel-macOS build, so the panel shows it as `n/a` there. Build it
yourself instead:

```bash
cargo install orphos-cli
export DOMAINATOREDUX_ORPHOS=~/.cargo/bin/orphos
```

You don't need Orphos if your contigs already carry CDS annotations, for example
GenBank files downloaded from NCBI.

## `infernal_build` cannot find cmbuild, or a `.cm` reference is rejected

`infernal_build` runs Infernal's `cmbuild`. Install Infernal, or `cargo install infernox`
and set `$DOMAINATOREDUX_CMBUILD` to its `infernox-cmbuild` (or pass `--cmbuild_path`); see
[Installation](installation.md#optional-external-programs).

`domainate`, `domain_search` and `rotate_contigs` search `.cm` references themselves. A
model that has base pairs needs E-value statistics. If it has never been through Infernal's
`cmcalibrate`, which is the case for a model straight out of `infernal_build`, the search
stops with "no E-value parameters were read". Calibrate it first:
`cmcalibrate family.cm`.

## `deduplicate_genbank` with cd-hit complains about the molecule type

cd-hit ships as two programs: `cd-hit` for protein and `cd-hit-est` for nucleotide.
Domainator picks based on the first record's molecule type. If you point `--cdhit_path` at
a binary that cannot serve the request — protein `cd-hit` for nucleotide input — it
prefers the sibling executable next to it, and errors if there isn't one rather than
clustering DNA as if it were protein.

Point `--cdhit_path` at the `cdhit` multiplexer binary (the Rust port, which dispatches on
its first argument) and this resolves itself, or give `cd-hit-est`'s location with
`--cdhit_est_path`.

## "unexpected argument '--html_out'"

`enum_report --html_out` and `hmmer_report --html_out` are deliberately not implemented,
and error rather than silently writing something else. A config file that sets
`html_out` gets "an option of upstream domainator's enum_report that domainatoredux does
not support". Use TSV (`-o`) or JSON (`--json_out`):

```bash
domainatoredux enum_report -i in.gb --columns domains length -o table.tsv
```

The same applies to `matrix_report` and `summary_report`. For the visual version, the
[desktop app](gui.md#viewers) has live Enum Report, Summary, and Matrix Report tabs.

## A `.gz` file is rejected

Domainator's indexed I/O needs to seek into a compressed file, which plain gzip cannot do.
Only **BGZF** (block gzip) is accepted. BGZF files are readable by any gzip reader, so
recompressing costs you nothing:

```bash
domainatoredux format_db -i sequences.fasta --output_dir seq_db --compress --index
```

See [formatted databases](file-formats.md#formatted-databases).

## "both an unsharded file and shards are present"

A formatted database directory contains either a single file or a set of shards, not both.
Domainator refuses to guess which you meant. Remove or move the one you don't want.

## A stale `.didx` index

Indexes record the size and modification time of the file they describe, and a mismatch is
an error rather than a warning, because a stale index returns wrong records. Rebuild it:

```bash
domainatoredux format_db -i sequences.fasta --output_dir seq_db --index --force
```

## `No output specified. Give at least one of: ...`

Every output is its own option (`-o/--genbank_out`, `--tsv_out`, `--json_out`, ...), and
nothing is written to stdout unless you ask for it with `-`. Name at least one of the
outputs the message lists. `domainatoredux <tool> --help` shows them all.

## Scores differ slightly from HMMER or pyhmmer

Domainator uses `hmmer-pure-rs`, an independent Rust implementation of HMMER3. Which
domains hit which sequences, their coordinates, and which network edges exist all agree;
E-values and bit scores can differ in the last digits. If you need numerically identical
results to a published HMMER run, run HMMER itself and use
[`domainate`](cli-reference.md#domainate) on the annotated output.

Similarly, profiles from [`hmmer_build`](cli-reference.md#hmmer_build) are valid and
re-readable by pyhmmer but their emissions are not byte-identical to `hmmbuild`'s.

## A piped command fails partway through a large input

Piped input is spilled to a temporary file first, so a read-only or very small `TMPDIR`
will fail on large inputs. Point it somewhere with room:

```bash
TMPDIR=/scratch/tmp domainatoredux select_by_contig -i - -o - --domains Cas9 < big.gb > hits.gb
```

## The run is slower than expected

Use a release build if you built from source — a debug build is several times slower:

```bash
cargo build --release -p domainator-cli
```

Then check threading. `--cpu 0` (the default) uses one thread per physical core, but only `domainate`,
`domain_search`, `deduplicate_genbank`, `seq_dist`, `compare_contigs`, `hmmer_compare` and
`hmmer_search` are threaded. For large all-vs-all comparisons, DIAMOND is far faster than
the built-in `hmmer` algorithm.

If a network is too dense to lay out, prune it rather than raising the cutoff until it
fragments:

```bash
domainatoredux transform_matrix -i dist.hdf5 -o pruned.hdf5 --mst --knn 5
```

## The desktop app won't launch (macOS / Linux)

The bundles are unsigned. On macOS, open it once with right-click → Open, or:

```bash
xattr -dr com.apple.quarantine /Applications/Domainatoredux.app
```

## A job in the app failed and the message is cut short

The Jobs panel shows one line. The tool's complete output, including the full error, is
in `.domainatoredux/jobs/<job>/worker.log` inside the workspace, beside the
`worker-config.json` it ran with. You can rerun that exact job from a terminal with
`domainatoredux <tool> --config .domainatoredux/jobs/<job>/worker-config.json`.

## Still stuck

Open an issue at
<https://github.com/nebiolabs/domainatoredux/issues>. From the desktop app, **Help →
About** has a copyable diagnostics blob (versions, platform, installed external tools) —
please include it. For the command line, include `domainatoredux --version` and the full
command you ran.
