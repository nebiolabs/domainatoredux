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
PY="${DOMAINATOR_PYTHON:-/home/sean/miniforge3/envs/domainator/bin/python}"

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

# 3. Automated guards.
echo
echo "== guard: regen_ssn_viewer_shell.py --check =="
"$PY" "$REPO_ROOT/scripts/regen_ssn_viewer_shell.py" --check \
  || { echo "  -> viewer shell is STALE; run scripts/regen_ssn_viewer_shell.py"; }

echo
echo "== guard: ssn_probe (MST/hierarchy/merge-event core) =="
( cd "$REPO_ROOT" && cargo run -q -p domainator-core --example ssn_probe ) \
  || echo "  -> ssn_probe failed; investigate matrix/ssn.rs"

echo
echo "== reminders =="
echo "  * Triage the changelog against UPSTREAM.md's file->owner map; port surfaces #2-#4."
echo "  * Run the per-tool parity gates for any tool whose Python file changed."
echo "  * Advance the pin in UPSTREAM.md to ${TARGET:0:12} and note anything deferred."
