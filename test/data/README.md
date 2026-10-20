# Test fixtures

A committed subset of upstream `domainator`'s `test/data`, copied verbatim. These
are the fixtures the [differential parity harness](../../docs/parity.md)
references, so the harness (and any hand-written Rust test that wants a real
GenBank/HMM/matrix file) works without the gitignored `external/domainator/`
clone, and the inputs cannot shift underfoot between runs.

Which files live here is derived, not curated: `scripts/parity/sync_fixtures.py`
takes the set of fixtures that upstream's harvested test invocations, and the
hand-written ones in `scripts/parity/extra_invocations.tsv`, reference.
Do not add or edit files here by hand.

```bash
scripts/parity/sync_fixtures.py --check    # verify they still match upstream
scripts/parity/sync_fixtures.py --update   # re-copy after the pin moves
```

Upstream is MIT-licensed (`external/domainator/LICENSE.txt`); the Biopython-derived
code that license also covers lives under `src/domainator/Bio`, not here.
Source: <https://github.com/nebiolabs/domainator>, at the commit `UPSTREAM.md` pins.
