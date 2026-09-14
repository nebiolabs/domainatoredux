#!/usr/bin/env python3
"""Regenerate the vendored SSN viewer HTML shell from the Python `domainator` source.

The Rust asset ``crates/domainator-core/assets/ssn_viewer_shell.html`` is NOT authored
by hand. It is the rendered output of the Python viewer,
``domainator.ssn_viewer_html.ssn_viewer_html(title, embedded_bundle_json)``, with two
interpolation points frozen into string tokens that the Rust ``render_html`` substitutes
at runtime (see ``crates/domainator-core/src/tools/build_ssn_viewer.rs``):

  * ``@@SSN_VIEWER_TITLE@@``            -> the HTML-escaped title (the <title> tag)
  * ``@@SSN_VIEWER_HEADING@@``          -> the HTML-escaped <h1> heading
  * ``const EMBEDDED_BUNDLE_BASE64 = null;``  -> swapped for the base64 bundle on embed

On top of that render, the Rust asset carries local-only patches that do not exist in
the Python source (the "local patch inventory" below). This script reproduces the
asset exactly: render from Python with the frozen tokens, then re-apply the patches.

Because the asset is generated, it must NEVER be hand-edited. Change the Python viewer
(or the local patch here), then re-run this script.

Usage:
    python scripts/regen_ssn_viewer_shell.py            # write the asset
    python scripts/regen_ssn_viewer_shell.py --check    # verify asset is up to date
    python scripts/regen_ssn_viewer_shell.py --src PATH # override vendored src dir

Run with the domainator conda env, e.g.:
    /home/sean/miniforge3/envs/domainator/bin/python scripts/regen_ssn_viewer_shell.py

--check exits non-zero (with a short diff) when the committed asset differs from what a
fresh render + patch would produce; use it as an upstream-drift guard in CI / parity runs.
"""

from __future__ import annotations

import argparse
import difflib
import importlib
import sys
from pathlib import Path

# --- frozen token contract (must match build_ssn_viewer.rs) ---
TITLE_TOKEN = "@@SSN_VIEWER_TITLE@@"
HEADING_TOKEN = "@@SSN_VIEWER_HEADING@@"
EMBED_NULL_LINE = "const EMBEDDED_BUNDLE_BASE64 = null;"

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SRC = REPO_ROOT / "external" / "domainator" / "src"
ASSET_PATH = REPO_ROOT / "crates" / "domainator-core" / "assets" / "ssn_viewer_shell.html"

# --- local patch inventory --------------------------------------------------
# Patches applied to the Python render to produce the Rust asset. Each entry is
# (name, anchor_line, block): `block` is inserted immediately after the unique
# `anchor_line`. Keep blocks minimal and additive so they survive upstream churn.
#
# 1. "gui-host-hook": surfaces the selected node labels to a Tauri parent window
#    via postMessage. The GUI's SsnView.svelte listens for `domainator-ssn-selection`
#    to drive select -> Extract selected -> open source sequences. Without this the
#    viewer still renders but GUI node-selection silently breaks.
GUI_HOST_HOOK_ANCHOR = (
    "        document.getElementById('selection-summary').textContent = "
    "selected.length.toLocaleString() + ' nodes selected';"
)
GUI_HOST_HOOK_BLOCK = """        // Domainator GUI host hook (additive; no-op when not embedded in an iframe):
        // surface the selected node labels so the app can open their source sequences.
        try {
            if (window.parent && window.parent !== window) {
                window.parent.postMessage({
                    type: 'domainator-ssn-selection',
                    labels: selected.map(i => state.bundle.graph.nodes[i])
                }, '*');
            }
        } catch (e) { /* ignore */ }
"""
# Sentinel unique to the block, used to assert the patch is present exactly once.
GUI_HOST_HOOK_SENTINEL = "domainator-ssn-selection"

LOCAL_PATCHES = [
    ("gui-host-hook", GUI_HOST_HOOK_ANCHOR, GUI_HOST_HOOK_BLOCK, GUI_HOST_HOOK_SENTINEL),
]

# 2. "heading-token": upstream's `viewer_heading()` decides at RENDER time whether the
#    <h1> is the bare app name or "<app name>: <network>", collapsing titles that name
#    no particular network. Rendering with a frozen token bakes the "<app>: " prefix in,
#    so that choice could never fire in the Rust asset -- the default CLI name is one of
#    upstream's generic titles, which would render "...Viewer: Domainator SSN Viewer".
#    Swap the whole heading for its own token and let `render_html` apply the rule.
HEADING_PREFIX = "Domainator Similarity Network Viewer: "


def _apply_heading_token(html: str) -> str:
    baked = f">{HEADING_PREFIX}{TITLE_TOKEN}</h1>"
    n = html.count(baked)
    if n != 1:
        _fail(f"patch 'heading-token': expected the baked <h1> heading exactly once, "
              f"found {n} (viewer_heading() changed upstream; update HEADING_PREFIX)")
    return html.replace(baked, f">{HEADING_TOKEN}</h1>", 1)


def _fail(msg: str) -> "NoReturn":  # type: ignore[name-defined]
    sys.exit(f"regen_ssn_viewer_shell: {msg}")


def render_shell(src_dir: Path) -> str:
    """Render the shell from the vendored Python source with frozen tokens + local patches."""
    src_dir = src_dir.resolve()
    if not (src_dir / "domainator" / "ssn_viewer_html.py").exists():
        _fail(f"no domainator/ssn_viewer_html.py under --src {src_dir}")

    # Make the vendored copy win over any installed `domainator`.
    sys.path.insert(0, str(src_dir))
    for mod in list(sys.modules):
        if mod == "domainator" or mod.startswith("domainator."):
            del sys.modules[mod]
    mod = importlib.import_module("domainator.ssn_viewer_html")

    html = mod.ssn_viewer_html(title=TITLE_TOKEN, embedded_bundle_json=None)

    # Token sanity: the two interpolation points must have frozen as expected.
    n_title = html.count(TITLE_TOKEN)
    if n_title != 2:
        _fail(f"expected {TITLE_TOKEN} exactly twice, found {n_title} "
              "(title escaping or template changed upstream)")
    n_embed = html.count(EMBED_NULL_LINE)
    if n_embed != 1:
        _fail(f"expected '{EMBED_NULL_LINE}' exactly once, found {n_embed} "
              "(embedded-bundle wiring changed upstream)")

    html = _apply_heading_token(html)
    if html.count(TITLE_TOKEN) != 1 or html.count(HEADING_TOKEN) != 1:
        _fail("heading-token patch did not leave exactly one of each title token")

    for name, anchor, block, sentinel in LOCAL_PATCHES:
        if sentinel in html:
            _fail(f"patch '{name}': sentinel already present in the Python render "
                  "(upstream may have absorbed it; drop it from LOCAL_PATCHES)")
        n_anchor = html.count(anchor)
        if n_anchor != 1:
            _fail(f"patch '{name}': anchor line found {n_anchor} times, expected 1 "
                  "(anchor changed upstream; update the patch)")
        html = html.replace(anchor + "\n", anchor + "\n" + block, 1)
        if html.count(sentinel) != 1:
            _fail(f"patch '{name}': re-application did not yield exactly one insertion")

    return html


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--src", type=Path, default=DEFAULT_SRC,
                    help=f"vendored domainator src dir (default: {DEFAULT_SRC})")
    ap.add_argument("--check", action="store_true",
                    help="verify the committed asset matches render+patch; exit 1 if not")
    args = ap.parse_args()

    rendered = render_shell(args.src)

    if args.check:
        if not ASSET_PATH.exists():
            _fail(f"asset missing: {ASSET_PATH}")
        current = ASSET_PATH.read_text()
        if current == rendered:
            print(f"OK: {ASSET_PATH.relative_to(REPO_ROOT)} is up to date "
                  f"({len(rendered)} bytes).")
            return
        diff = "".join(difflib.unified_diff(
            current.splitlines(keepends=True), rendered.splitlines(keepends=True),
            fromfile="committed", tofile="render+patch"))
        sys.stdout.write(diff[:8000])
        if len(diff) > 8000:
            sys.stdout.write("\n... (diff truncated) ...\n")
        _fail("asset is STALE — run `python scripts/regen_ssn_viewer_shell.py` to update.")

    ASSET_PATH.write_text(rendered)
    print(f"wrote {ASSET_PATH.relative_to(REPO_ROOT)} ({len(rendered)} bytes).")


if __name__ == "__main__":
    main()
