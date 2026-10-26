# Plan: unbounded text / table editing

Deferred feature. Today the Text and Table views load the **whole file** as one JS
string over the JSON IPC bridge, and `cmd_files::read_text_file` refuses anything
above 64 MB. Within that cap the Table view is already unbounded in *rows* (its
virtualizer falls back to proportional scrolling past the browser's maximum element
height), so the remaining limit is the file itself, not the row count.

This is the plan for lifting that cap. It is a real project — a windowed document
model in Rust plus a custom editor widget — so it is staged, and **Phase 1 alone is
probably most of the value**: it removes "cannot open the file at all", which is the
complaint users actually hit.

## Why the cap exists

Three separate costs, all proportional to file size:

1. **IPC.** The file crosses as a JSON string: escaped on the Rust side, parsed by the
   webview, several copies live at once. At 64 MB this is already hundreds of ms.
2. **Memory.** The JS string, plus (Table) the parsed `string[][]`, plus Svelte's deep
   proxies. Five to ten times the file size is normal.
3. **The widget.** `<textarea>` holds the entire value; layout, undo and IME cost grow
   with content. Typing latency degrades well before 64 MB.

Raising the cap does not fix any of these — it trades a clear refusal for an OOM'd
webview. The fix is to stop moving the whole file.

## Target

Resident memory `O(visible window + accumulated edits + index)`, independent of file
size. Bounded by disk, not RAM.

---

## Phase 1 — windowed read-only viewing above the cap

Ships on its own and is worth doing first.

### Backend: a sparse line index

New `crates/domainator-core/src/app_support/line_index.rs`.

A dense `Vec<u64>` of line starts costs 8 bytes per line — 400 MB for a 50M-line file,
which defeats the purpose. Store **one offset per 64-line block** instead (0.125
bytes/line; 6 MB for 50M lines) and rescan within a block to reach an exact line. This
is the same shape as the sparse offset tables in `io/didx.rs`.

```rust
pub struct LineIndex {
    block_starts: Vec<u64>,   // byte offset of every 64th line
    pub lines: u64,
    pub bytes: u64,
    fingerprint: (u64, u64),  // (size, mtime_ns), as io::didx::source_fingerprint
}

impl LineIndex {
    pub fn build(path: &Path) -> io::Result<Self>;          // one memchr scan
    pub fn line_range(&self, f: &File, line: u64) -> io::Result<(u64, u64)>;
    pub fn read_lines(&self, f: &File, first: u64, count: u64) -> io::Result<Vec<String>>;
    pub fn is_stale(&self, path: &Path) -> bool;            // fingerprint mismatch
}
```

`const BLOCK: u64 = 64;` — tune against a real file, but the index must stay small
enough that opening a multi-GB file is instant.

### Backend: commands

New `cmd_bigtext.rs`, with a `tauri::State` holding open documents:

```rust
pub struct OpenDocs(pub Mutex<HashMap<DocId, Doc>>);

open_document(path) -> { doc_id, lines, bytes, truncated_utf8: bool }
read_document_lines(doc_id, first, count) -> Vec<String>
close_document(doc_id)
```

`open_document` builds the index and keeps the `File` handle. `close_document` must be
called from the viewer's `$effect` teardown or the handles leak — worth a debug
assertion on the map's size.

### Frontend

- `LargeTextView.svelte` / `LargeTableView.svelte`, chosen in `viewers/registry.ts`
  by file size (the `FileEntry.size` the workspace scan already carries), falling back
  to today's editors below the cap.
- Reuse the virtualizer from `TableView.svelte` verbatim — including the proportional
  fallback past `MAX_SPACER`, which is the same problem at a larger scale. Factor it
  into `lib/virtualize.ts` rather than copying it a third time.
- Fetch windows on scroll with the same overscan + in-flight coalescing
  `ContigView.ensureSeqRange` uses; cache a few hundred lines either side.
- Label the view read-only in the toolbar, explicitly. A silently read-only editor is
  the bug this whole document exists to undo.

---

## Phase 2 — editing

### The document model: a piece table

The original file stays on disk and is never fully read. The document is a list of
pieces, each either a byte range of the original or a range of an in-memory add
buffer that accumulates inserted text:

```rust
enum Source { Original, Add }
struct Piece { source: Source, offset: u64, len: u64, lines: u64 }
```

Keeping `lines` per piece is what makes "line N" resolvable in `O(log n)` — but only
if the pieces live in a balanced tree keyed by cumulative line count, not a `Vec`.
Start with a `Vec` and a linear scan (correct, and fine for a few thousand edits),
and swap in a tree behind the same API if editing a large file ever gets slow. Do not
build the tree first.

This has to live in **Rust**: the original buffer is a file, and the whole point is
not to have it in the webview.

```rust
replace_range(doc_id, start_line, start_col, end_line, end_col, text) -> DocEdit
undo(doc_id) / redo(doc_id) -> DocEdit
save_document(doc_id) -> ()
```

`DocEdit` returns the new line count plus the range that changed, so the frontend can
invalidate its window cache without refetching the file.

### Save

Stream the pieces to a temp file — `io::copy` on a `Take` for original pieces, the
slice for add pieces — then rename, as `write_text_file` already does. Never
materializes the document in RAM.

Two ordering traps:

- The original file must stay open until the temp file is complete, because pieces
  reference it. Rename after, then rebuild the table as a single whole-file piece and
  re-fingerprint.
- If the write fails partway, the original is untouched (the rename never happens) and
  the piece table is still valid. Verify this with a test that fails the write.

### The editor widget

`<textarea>` cannot render windowed content, so this is a custom component. The
established shape:

- A scroll container sized `lines * lineHeight` (proportional fallback again).
- Only visible lines rendered, as absolutely positioned rows.
- A **hidden, single-line `<textarea>` positioned at the caret** to receive keystrokes,
  clipboard and IME composition. This is the CodeMirror 5 trick and the reason to not
  reach for `contenteditable`.
- Caret and selection are model `(line, col)` coordinates, drawn as positioned
  elements; they are not DOM selections.

This is the largest single piece of work, and the part with the long tail: selection
across window boundaries, mouse drag with autoscroll, keyboard navigation, word/line
operations, clipboard, undo granularity, accessibility.

**CodeMirror 6 is not a shortcut here.** Its document is a rope, but it is an
*in-memory* rope — it still wants the whole file, and degrades around 100 MB. It would
solve the widget and not the problem.

---

## Phase 3 — the long tail

- **Search.** Streaming scan on the Rust side returning match offsets; the frontend
  jumps by line. Do not pull the file into JS to run a regex over it.
- **Encoding.** `read_to_string` requires valid UTF-8, so a file with one bad byte
  fails to open today. Read lossily and say so in the toolbar. Window reads must never
  split a multi-byte character at a boundary — an explicit test case.
- **External modification.** A long-lived document holds byte ranges into a file a tool
  run may overwrite. Check the `(size, mtime_ns)` fingerprint before every window read
  and on focus; prompt to reload. This is the same staleness contract `.didx` uses.

---

## Line endings — already handled, keep it that way

This was a live bug (a CRLF table was silently rewritten as LF on save) and has since
been fixed in the current editors, independently of this plan:
`app_support::text_file` detects the terminator on read, `read_text_file` hands the
frontend LF plus the detected `eol`, and `write_text_file` / `create_text_file` put it
back. The editors carry `eol` alongside the buffer and show it in their toolbars.

The windowed model must preserve the same contract — reading a file and saving it
unedited is byte-identical — and `text_file.rs`'s
`a_file_read_then_saved_unedited_is_byte_identical` is the test to extend rather than
replace. Note that a windowed editor cannot detect from the whole file cheaply; detect
from the first block read and keep it on the document.

---

## Integration with what already exists

The seams are in place, so the new views plug into them rather than adding parallel
machinery:

- `ws.registerEditor(path, save)` — registers the save hook File → Save calls, and the
  one the unsaved-changes dialog calls on "Save".
- `ws.markDirty(path)` / `ws.clearDirty(path)` — drives the File menu's enabled state
  and the close guard.
- `ws.guardDirty(paths, ...)` — already wraps every route that closes a tab, so closes
  prompt with no extra work. Tab switches do not unmount the editor (ViewerDock keeps
  visited views mounted), so they need no prompt.

## Verification

Per the repo's verification model this is all net-new with no upstream counterpart, so
`cargo test` is the only gate. The tests that carry their weight:

- **Piece table against an oracle.** Apply a long pseudo-random sequence of inserts,
  deletes and replacements to both the piece table and a plain `String`; assert equal
  content, equal line count, and equal `line_range` for every line. This is the test
  that actually finds the bugs — a piece table is easy to get subtly wrong at piece
  boundaries.
- **Line index edge cases.** Empty file; no trailing newline; CRLF; a lone CR; a file
  that is one enormous line; multi-byte UTF-8 straddling a block boundary. Compare
  against a naive full-scan line split.
- **Save round-trip.** Byte-identical output when nothing was edited (this is the
  strongest single assertion available). Correct after edits. Original intact when the
  write fails.
- **Window reads never split a character** — read every window size from 1 to 200 over
  a file of multi-byte text and assert each result re-encodes to the expected slice.
- **Fingerprint staleness** — touch the file, assert `is_stale`.
- **Handle hygiene** — open and close many documents, assert the map empties.

## Open questions for whoever picks this up

1. **Is editing multi-GB text actually needed, or is viewing enough?** If viewing is
   enough, Phase 1 is the whole project and Phase 2 never has to happen. Ask before
   building the piece table.
2. **What is the real size distribution?** If files cluster under ~200 MB, raising the
   cap and streaming the IPC payload may beat a windowed model outright, at a fraction
   of the cost. Measure before committing.
3. Should the Table view share the document model, or stay a separate row-oriented one?
   A TSV is lines, so sharing is plausible — but cell editing wants a different
   granularity of undo.
