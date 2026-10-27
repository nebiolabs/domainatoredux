#!/usr/bin/env bash
# Upstream-sync routine for the Python `domainator` package.
#
# Refreshes the vendored copy, prints the changelog to triage, and runs the automated
# drift/parity guards. It does NOT decide what to port — a human triages each commit
# against the "Python file -> Rust owner" map in UPSTREAM.md — it just removes the toil.
#
# Usage:
#   scripts/sync_upstream.sh [<target-ref>] [--from <live-checkout-path>]
#
#   <target-ref>            upstream ref to sync to (default: origin/main; if a live
#                           checkout is given, its HEAD is used when no ref is passed)
#   --from <path>           fetch from a local domainator checkout instead of `origin`
#                           (useful for commits not yet pushed to GitHub)
#
# After running: port whatever the changelog requires, then advance the pin in UPSTREAM.md.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VENDOR="$REPO_ROOT/external/domainator"
PY="${DOMAINATOREDUX_PYTHON:-/home/sean/miniforge3/envs/domainator/bin/python}"

TARGET_REF=""
LIVE_FROM=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --from) LIVE_FROM="$2"; shift 2 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) TARGET_REF="$1"; shift ;;
  esac
done

if [[ ! -d "$VENDOR/.git" ]]; then
  echo "error: $VENDOR is not a git checkout of domainator" >&2
  exit 1
fi

PIN="$(git -C "$VENDOR" rev-parse HEAD)"
echo "== current pin: ${PIN:0:12} =="

# 1. Refresh the vendored copy.
if [[ -n "$LIVE_FROM" ]]; then
  echo "== fetching from local checkout: $LIVE_FROM =="
  git -C "$VENDOR" fetch "$LIVE_FROM" "${TARGET_REF:-main}"
  TARGET="$(git -C "$VENDOR" rev-parse FETCH_HEAD)"
else
  echo "== fetching from origin =="
  git -C "$VENDOR" fetch origin
  TARGET="$(git -C "$VENDOR" rev-parse "${TARGET_REF:-origin/main}")"
fi

if [[ "$TARGET" == "$PIN" ]]; then
  echo "== already at ${TARGET:0:12}; nothing new upstream =="
else
  echo "== fast-forwarding vendored copy to ${TARGET:0:12} =="
  git -C "$VENDOR" merge --ff-only "$TARGET"

  # 2. Changelog to triage (map touched files -> Rust owners via UPSTREAM.md).
  echo
  echo "======================= CHANGELOG ${PIN:0:12}..${TARGET:0:12} ======================="
  git -C "$VENDOR" log --oneline --stat "$PIN..$TARGET"
  echo "======================================================================"
fi

# 3. Automated guards. Each sets FAILED rather than exiting, so one failure does not
#    hide the others; the script exits non-zero at the end if any tripped.
FAILED=0
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo
echo "== guard: regen_ssn_viewer_shell.py --check =="
"$PY" "$REPO_ROOT/scripts/regen_ssn_viewer_shell.py" --check \
  || { echo "  -> viewer shell is STALE; run scripts/regen_ssn_viewer_shell.py"; FAILED=1; }

echo
echo "== guard: regen_report_shells.py --check =="
"$PY" "$REPO_ROOT/scripts/regen_report_shells.py" --check \
  || { echo "  -> a report shell is STALE; run scripts/regen_report_shells.py"; FAILED=1; }

# Upstream's `domainator describe` is its complete interface; the committed snapshot is
# what the `upstream_manifest` cargo test diffs this port's `describe` against.
echo
echo "== guard: refresh_manifest.py --check =="
"$PY" "$REPO_ROOT/scripts/parity/refresh_manifest.py" --python "$PY" --check \
  || { echo "  -> upstream's interface changed; run scripts/parity/refresh_manifest.py"; FAILED=1; }

echo
echo "== guard: UPSTREAM_VERSION vs upstream __version__ =="
# The release this port reports parity with (`--version`, About, provenance, the
# `.dsnv` domainator_version) is a constant, not something read at build time.
UP_VER="$(sed -nE 's/^__version__ *= *"([^"]+)".*/\1/p' "$VENDOR/src/domainator/__init__.py")"
# First quoted string after the macro's name, so a rustfmt reflow cannot break it.
OUR_VER="$(awk '/macro_rules! upstream_version/{f=1} f && match($0, /"[^"]+"/){print substr($0, RSTART+1, RLENGTH-2); exit}' \
  "$REPO_ROOT/crates/domainator-core/src/version.rs")"
echo "  upstream __version__: ${UP_VER:-?}   UPSTREAM_VERSION: ${OUR_VER:-?}"
if [[ -z "$UP_VER" || -z "$OUR_VER" || "$UP_VER" != "$OUR_VER" ]]; then
  echo "  -> update upstream_version!() in crates/domainator-core/src/version.rs"
  FAILED=1
fi

echo
echo "== guard: cargo test =="
( cd "$REPO_ROOT" && cargo test --workspace --quiet ) \
  || { echo "  -> cargo test failed"; FAILED=1; }

# Both zero-warning gates. `--all-targets` matters: `cargo build` does not compile
# `#[cfg(test)]` code, so a test-only break is invisible to the build alone.
echo "== guard: cargo build + clippy (zero warnings) =="
( cd "$REPO_ROOT" && RUSTFLAGS="-D warnings" cargo build --workspace --quiet ) \
  || { echo "  -> cargo build produced warnings or errors"; FAILED=1; }
( cd "$REPO_ROOT" && cargo clippy --workspace --all-targets --quiet -- -D warnings ) \
  || { echo "  -> cargo clippy produced warnings or errors"; FAILED=1; }

# The SSN core is compared against Python field for field: both probes print the same
# JSON document, and json_parity.py diffs the parsed form (serde_json is built with
# preserve_order while Python writes sort_keys=True, so bytes never match and are not
# part of the contract).
echo
echo "== guard: ssn_probe vs domainator.ssn_edges/ssn_hierarchy =="
MATRIX="${SSN_PROBE_MATRIX:-$VENDOR/test/data/FeSOD_dist.dense.hdf5}"
if [[ ! -f "$MATRIX" ]]; then
  echo "  -> SKIPPED: no fixture at $MATRIX (set SSN_PROBE_MATRIX to override)"
else
  for CAP in 500 5; do
    if ( cd "$REPO_ROOT" && cargo run -q -p domainator-core --example ssn_probe -- \
           "$MATRIX" "$CAP" ) > "$TMP/rust.json" \
       && "$PY" "$REPO_ROOT/scripts/ssn_probe.py" "$MATRIX" --max_merge_events "$CAP" \
           > "$TMP/py.json"; then
      "$PY" "$REPO_ROOT/scripts/json_parity.py" "$TMP/py.json" "$TMP/rust.json" \
        || { echo "  -> ssn_probe DIVERGED at --max_merge_events=$CAP; investigate matrix/ssn.rs"; FAILED=1; }
    else
      echo "  -> ssn_probe could not run at --max_merge_events=$CAP"; FAILED=1
    fi
  done
fi

# The fixture copy under test/data/ is derived from what upstream's tests reference,
# so a new or changed fixture upstream must be copied over before the harness runs.
echo
echo "== guard: committed fixtures vs upstream test/data =="
"$PY" "$REPO_ROOT/scripts/parity/sync_fixtures.py" --check \
  || { echo "  -> run scripts/parity/sync_fixtures.py --update"; FAILED=1; }

# The broad-coverage gate: replay every invocation upstream's pytest suite makes
# through both implementations. known_gaps.tsv is the tracked list of what does not
# yet reproduce, and this trips when reality and that list disagree either way --
# a new divergence, or a stale entry claiming work that is already done.
echo
echo "== guard: differential parity sweep (see docs/parity.md) =="
if [[ ! -x "$REPO_ROOT/target/debug/domainatoredux" ]]; then
  echo "  -> SKIPPED: no target/debug/domainatoredux (run cargo build)"
elif ! "$PY" -c 'import domainator' 2>/dev/null; then
  echo "  -> SKIPPED: $PY cannot import domainator"
else
  ( cd "$REPO_ROOT" && "$PY" scripts/parity/differential.py ) \
    || { echo "  -> parity sweep disagrees with scripts/parity/known_gaps.tsv"; FAILED=1; }
fi

echo
echo "== reminders =="
echo "  * Triage the changelog against UPSTREAM.md's file->owner map; port surfaces #2-#4."
echo "  * Run the per-tool parity gates for any tool whose Python file changed."
echo "  * Read the scripts/parity/known_gaps.tsv diff, and triage the class column on new rows."
echo "  * After refresh_manifest.py, regenerate the interface allowlist and triage new rows:"
echo "    UPDATE_MANIFEST_ALLOWLIST=1 cargo test -p domainator-cli --test upstream_manifest"
echo "  * Advance the pin in UPSTREAM.md to ${TARGET:0:12} and note anything deferred;"
echo "    set upstream_commit!() in crates/domainator-core/src/version.rs to match."

if [[ "$FAILED" -ne 0 ]]; then
  echo
  echo "One or more guards FAILED (see above)."
  exit 1
fi
