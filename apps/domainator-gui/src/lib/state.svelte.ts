import {
  cancelJob as apiCancelJob,
  clearJobs as apiClearJobs,
  createFolder,
  deleteFile,
  deleteJob as apiDeleteJob,
  importFiles,
  listDatabases,
  listFolder,
  loadJobs as apiLoadJobs,
  moveFiles as apiMoveFiles,
  openWorkspace,
  registerDatabase,
  renameFile as apiRenameFile,
  renameDatabase as apiRenameDatabase,
  deleteDatabase as apiDeleteDatabase,
  buildDatabaseIndex as apiBuildDatabaseIndex,
  cancelDatabaseIndex as apiCancelDatabaseIndex,
  INDEXING_CANCELED,
  rescanTree,
  syncReferenceDbs,
  unregisterDatabase,
} from "./api";
import { clearCache, invalidatePath } from "./cache";
import type { SelItem } from "./selection";
import type { DatabaseRow, FileEntry, FileType, FolderNode, JobInfo, OpenTab } from "./types";

// Classify a path to a viewer file type by extension (for opening job outputs).
function classifyPath(path: string): FileType | null {
  const lower = path.toLowerCase().replace(/\.(gz|bgz|bgzf)$/, "");
  const ext = lower.split(".").pop() ?? "";
  if (["gb", "gbk", "genbank", "gbff"].includes(ext)) return "genbank";
  if (["fasta", "fa", "fna", "faa", "pep"].includes(ext)) return "fasta";
  if (["hmm", "hmmer"].includes(ext)) return "hmm";
  if (["hdf5", "hdf", "h5"].includes(ext)) return "hdf5";
  if (["dsnv", "ssnv", "ssnview"].includes(ext)) return "ssn_viewer";
  if (["tsv", "csv", "tab"].includes(ext)) return "table";
  if (["txt", "text", "list"].includes(ext)) return "text";
  if (ext === "svg") return "svg";
  return null;
}

// Central reactive shell state (Svelte 5 runes). One instance is shared across
// all panels via the `ws` export.
class WorkspaceStore {
  root = $state<string | null>(null);
  tree = $state<FolderNode | null>(null);
  selectedFolder = $state<string | null>(null);
  files = $state<FileEntry[]>([]);
  viewedJobFiles = $state<Record<string, string>>({});
  loadingFiles = $state(false);
  tabs = $state<OpenTab[]>([]);
  activeTabId = $state<string | null>(null);
  // Most-recently-closed first, so "Reopen Closed Tab" is a stack. Bounded in
  // `rememberClosed`.
  closedTabs = $state<OpenTab[]>([]);
  // Paths with unsaved editor buffers, reported by TableView / TextView. A plain
  // object rather than a Set: a bare `Set` inside `$state` is not deeply reactive
  // in Svelte 5 (that needs `SvelteSet`), and this has to drive both the File
  // menu's Save item and the close guard.
  //
  // ContigView is deliberately absent -- it writes through `cmd_edit` on every
  // action, so it is never dirty.
  dirty = $state<Record<string, true>>({});
  // A pending "you have unsaved changes" decision, rendered by App as a
  // Save / Discard / Cancel dialog. Set by `guardDirty` whenever something would
  // close the tab of an editor holding an unsaved buffer.
  pendingDirty = $state<{ path: string; proceed: () => void } | null>(null);
  error = $state<string | null>(null);
  // Requested contig to scroll to when a viewer opens (keyed by file path);
  // consumed and cleared by the contig viewer. Used by SSN → sequence nav.
  focusContig = $state<Record<string, number>>({});
  // Same channel for HMM files: the profile index the Logo view should show,
  // consumed and cleared by HmmLogo. Set by the profile table's "logo" button.
  focusHmmProfile = $state<Record<string, number>>({});
  // The Contig View selection per sequence file (`selection.ts`), shared with
  // the file's Enum Report tab. `from` names the view that last set it and `rev`
  // counts changes, so a view shown later can tell the selection moved while it
  // was hidden. Raw state, replaced whole by `setViewSelection`: a selection can hold
  // every row of a large report, and deep proxies would make that slow.
  viewSelection = $state.raw<Record<string, { items: SelItem[]; from: string; rev: number }>>({});
  // Selected sub-view per open file (e.g. "contig" | "enum" | "summary"). Lives
  // here rather than in ViewerDock so a viewer can switch the dock to a sibling
  // tab — HmmReport → Logo, for instance.
  subActive = $state<Record<string, string>>({});
  // Bumped when a file is rewritten on disk while it may be open (a job output),
  // so ViewerDock remounts that file's views. Views otherwise stay mounted across
  // tab switches and would keep showing the old contents.
  fileRev = $state<Record<string, number>>({});
  // Set while a multi-file copy is running, so the file table can show progress
  // instead of appearing to do nothing.
  importing = $state(0);
  // Set while a folder create / file delete is in flight.
  busy = $state(false);
  // The shared database registry -- the same list in every workspace, and
  // available before any workspace is open.
  databases = $state<DatabaseRow[]>([]);
  dbError = $state<string | null>(null);
  jobs = $state<JobInfo[]>([]);
  // A request to open the tool dialog (from the Tools menu or a job "clone"),
  // consumed by App. `initial` pre-fills the form (clone).
  requestedTool = $state<{ id: string; initial?: Record<string, unknown>; files?: FileEntry[] } | null>(null);
  // A request to open the "new subfolder" input for this folder (from the folder
  // tree's context menu), consumed by App -- which owns that inline form. Same
  // consume-and-clear channel as `requestedTool`.
  requestNewFolder = $state<string | null>(null);

  /// Replace the selection of the file at `path`; `from` names the view.
  setViewSelection(path: string, items: SelItem[], from: string): void {
    const rev = (this.viewSelection[path]?.rev ?? 0) + 1;
    this.viewSelection = { ...this.viewSelection, [path]: { items, from, rev } };
  }

  /// Drop the selections of `paths`: their indices mean nothing once a file is
  /// rewritten or its views are gone.
  clearViewSelections(paths: string[]): void {
    if (!paths.some((p) => p in this.viewSelection)) return;
    const next = { ...this.viewSelection };
    for (const p of paths) delete next[p];
    this.viewSelection = next;
  }

  /// Switch the viewer dock to sibling view `viewId` for the file at `path`.
  showSubView(path: string, viewId: string): void {
    this.subActive[path] = viewId;
    this.activeTabId = path;
  }

  /// Open the tool dialog for `id`, pre-filled with a cloned job's `initial`
  /// params, or with `files` placed in its inputs (`toolInputs.placeFiles`).
  openTool(id: string, initial?: Record<string, unknown>, files?: FileEntry[]): void {
    this.requestedTool = { id, initial, files };
  }

  // ── file table selection ────────────────────────────────────────────────────
  // Paths selected in the file table. A plain object for the reason `dirty` is
  // one. Independent of the active tab: a plain click sets both, Ctrl/Shift+click
  // only this.
  selectedPaths = $state<Record<string, true>>({});
  // Where a Shift+click range starts.
  selectionAnchor = $state<string | null>(null);

  /// The selected files, in listing order.
  get selectedFiles(): FileEntry[] {
    return this.files.filter((f) => this.selectedPaths[f.path]);
  }

  /// The files a tool is offered for and pre-filled with: the selection, or
  /// failing that the active tab's file when it is in the current folder (the
  /// table highlights it). A tab open on a file from elsewhere counts for
  /// nothing: the dialog's dropdowns list only this folder's files.
  get toolTargets(): FileEntry[] {
    const sel = this.selectedFiles;
    if (sel.length) return sel;
    const active = this.tabs.find((t) => t.id === this.activeTabId)?.file;
    const here = active && this.files.find((f) => f.path === active.path);
    return here ? [here] : [];
  }

  setSelection(paths: string[], anchor: string | null = paths[0] ?? null): void {
    this.selectedPaths = Object.fromEntries(paths.map((p) => [p, true as const]));
    this.selectionAnchor = anchor;
  }

  toggleSelected(path: string): void {
    if (this.selectedPaths[path]) delete this.selectedPaths[path];
    else this.selectedPaths[path] = true;
    this.selectionAnchor = path;
  }

  clearSelection(): void {
    this.setSelection([], null);
  }

  // ── dragging files onto a folder ───────────────────────────────────────────
  // Set while files from the table are dragged (pointer events, not HTML5 DnD —
  // see FileTable), so the folder tree can highlight the folder under the pointer.
  dragging = $state<{ paths: string[]; x: number; y: number } | null>(null);
  dropTarget = $state<string | null>(null);

  async open(root: string): Promise<void> {
    this.error = null;
    clearCache();
    try {
      const info = await openWorkspace(root);
      this.root = info.root;
      try {
        this.viewedJobFiles = JSON.parse(localStorage.getItem(`viewedJobFiles:${info.root}`) ?? "{}");
      } catch {
        this.viewedJobFiles = {};
      }
      this.tree = info.tree;
      // The database list is shared, but a workspace from before it was may
      // still carry its own; passing the root folds that in (once).
      await this.loadDatabases(info.root);
      await this.loadJobs(); // restore persisted job history for this workspace
      // Auto-select the root folder so the file table isn't empty on open.
      await this.selectFolder(info.tree.path);
    } catch (e) {
      this.error = String(e);
    }
  }

  async selectFolder(path: string): Promise<void> {
    // Another folder starts with nothing selected; re-listing the same one (a
    // refresh after a job, delete or move) keeps the selection.
    if (path !== this.selectedFolder) this.clearSelection();
    this.selectedFolder = path;
    this.loadingFiles = true;
    this.error = null;
    try {
      this.files = await listFolder(this.root ?? path, path);
      // A refresh keeps what is still listed selected; a file that went away drops out.
      const present = new Set(this.files.map((f) => f.path));
      const kept = Object.keys(this.selectedPaths).filter((p) => present.has(p));
      if (kept.length !== Object.keys(this.selectedPaths).length) {
        this.setSelection(kept, kept.includes(this.selectionAnchor ?? "") ? this.selectionAnchor : (kept[0] ?? null));
      }
    } catch (e) {
      this.error = String(e);
      this.files = [];
    } finally {
      this.loadingFiles = false;
    }
  }

  openFile(file: FileEntry): void {
    const id = file.path;
    if (id === this.activeTabId) {
      this.markViewed(file);
      return;
    }
    if (!this.tabs.some((t) => t.id === id)) {
      this.tabs = [...this.tabs, { id, file }];
    }
    this.activeTabId = id;
    this.markViewed(file);
  }

  markViewed(file: FileEntry): void {
    if (!file.created_by_job || !this.root) return;
    this.viewedJobFiles[file.path] = file.created_by_job;
    try {
      localStorage.setItem(`viewedJobFiles:${this.root}`, JSON.stringify(this.viewedJobFiles));
    } catch { /* ignore */ }
  }

  /// Open a file by path (used by cross-view nav), optionally scrolling to a contig.
  openPath(path: string, fileType: FileType, contigIndex?: number): void {
    if (contigIndex !== undefined) this.focusContig[path] = contigIndex;
    const name = path.split(/[\\/]/).pop() ?? path;
    this.openFile({
      path, name, file_type: fileType, description: "", size: 0,
      created_ms: null, modified_ms: null, created_by_job: null,
    });
  }

  /// Open a job output in a viewer if its type is viewable (else no-op).
  openOutput(path: string): void {
    const t = classifyPath(path);
    if (t) this.openFile(this.files.find((file) => file.path === path) ?? {
      path, name: path.split(/[\\/]/).pop() ?? path, file_type: t,
      description: "", size: 0, created_ms: null, modified_ms: null,
      created_by_job: this.jobs.find((job) => job.outputs.includes(path))?.job_id ?? null,
    });
  }

  // --- jobs ---
  upsertJob(job: JobInfo): void {
    if (job.status === "completed") {
      for (const output of job.outputs) {
        invalidatePath(output);
        // Remounting an editor with an unsaved buffer would silently drop it.
        if (!this.dirty[output]) this.fileRev[output] = (this.fileRev[output] ?? 0) + 1;
      }
      this.clearViewSelections(job.outputs);
    }
    const i = this.jobs.findIndex((j) => j.job_id === job.job_id);
    if (i >= 0) {
      const copy = [...this.jobs];
      copy[i] = job;
      this.jobs = copy;
    } else {
      this.jobs = [job, ...this.jobs];
    }
  }

  private sortJobs(list: JobInfo[]): JobInfo[] {
    return list.sort((a, b) => b.started_at_ms.localeCompare(a.started_at_ms));
  }

  /// Restore this workspace's persisted job history (no-op until a workspace opens).
  /// A failure here used to be swallowed, so a corrupt `jobs.json` looked exactly
  /// like an empty history.
  async loadJobs(): Promise<void> {
    if (!this.root) return;
    try {
      this.jobs = this.sortJobs(await apiLoadJobs(this.root));
    } catch (e) {
      this.error = `Could not load the job history: ${e}`;
    }
  }

  /// Ask the backend to cancel a queued or running job. A running job's worker
  /// process is killed with everything it started; the job shows `canceling`
  /// until its partial outputs are removed, then `canceled`.
  async cancelJob(id: string): Promise<void> {
    try {
      await apiCancelJob(id);
    } catch (e) {
      this.error = `Could not cancel the job: ${e}`;
    }
  }

  /// Delete one job from history (persisted).
  async deleteJob(id: string): Promise<void> {
    try {
      await apiDeleteJob(id);
      this.jobs = this.jobs.filter((j) => j.job_id !== id);
    } catch (e) {
      this.error = String(e);
    }
  }

  /// Clear all job history (persisted).
  async clearJobs(): Promise<void> {
    try {
      await apiClearJobs();
      this.jobs = [];
    } catch (e) {
      this.error = String(e);
    }
  }

  markDirty(path: string): void {
    this.dirty[path] = true;
  }

  clearDirty(path: string): void {
    delete this.dirty[path];
  }

  get dirtyPaths(): string[] {
    return Object.keys(this.dirty);
  }

  // ── mounted editors ────────────────────────────────────────────────────────
  // ViewerDock keeps every visited view mounted until its tab closes, so several
  // editors can be mounted at once, each with its own buffer. A plain Map, not
  // `$state`: it holds function identities that are never rendered.
  private editors = new Map<string, () => Promise<void>>();

  /// Register a mounted editor's save hook; returns the unregister function,
  /// so a viewer can use it directly as an `$effect` teardown.
  registerEditor(path: string, save: () => Promise<void>): () => void {
    this.editors.set(path, save);
    return () => {
      if (this.editors.get(path) === save) this.editors.delete(path);
    };
  }

  /// Write the buffer of the editor for `path` (default: the active tab). A
  /// no-op when that tab is not an editor, which is why File → Save is grayed
  /// out then.
  async saveEditor(path: string | null = this.activeTabId): Promise<void> {
    if (path !== null) await this.editors.get(path)?.();
  }

  /// Write every unsaved editor buffer (the window-close "Save and close").
  async saveAllEditors(): Promise<void> {
    for (const path of this.dirtyPaths) await this.saveEditor(path);
  }

  /// Run `proceed`, but first ask about each of `paths` that has an unsaved
  /// editor buffer — the tabs about to be closed.
  ///
  /// Every route that closes a tab goes through here. Closing destroys the
  /// editor, and the teardown that clears the dirty flag goes with it, so
  /// without this even the window-close guard could not see the edits that had
  /// just been thrown away. Switching tabs no longer needs it: the editor stays
  /// mounted, buffer and all.
  guardDirty(paths: string[], proceed: () => void): void {
    const p = paths.find((path) => this.dirty[path] && this.editors.has(path));
    if (p === undefined) {
      proceed();
      return;
    }
    // Once `p` is saved or discarded it is clean, so asking again moves on to
    // the next unsaved one.
    this.pendingDirty = { path: p, proceed: () => this.guardDirty(paths, proceed) };
  }

  canOpen(path: string): boolean {
    return classifyPath(path) !== null;
  }

  closeTab(id: string): void {
    const idx = this.tabs.findIndex((t) => t.id === id);
    if (idx === -1) return;
    this.guardDirty([id], () => {
      this.rememberClosed(this.tabs[idx]);
      delete this.subActive[id];
      this.clearViewSelections([id]);
      this.tabs = this.tabs.filter((t) => t.id !== id);
      if (this.activeTabId === id) {
        const next = this.tabs[idx] ?? this.tabs[idx - 1] ?? null;
        this.activeTabId = next ? next.id : null;
      }
    });
  }

  /// Close every tab but `id`, which becomes active.
  closeOtherTabs(id: string): void {
    const closing = this.tabs.filter((t) => t.id !== id).map((t) => t.id);
    this.guardDirty(closing, () => {
      for (const t of this.tabs) if (t.id !== id) this.rememberClosed(t);
      for (const c of closing) delete this.subActive[c];
      this.clearViewSelections(closing);
      this.tabs = this.tabs.filter((t) => t.id === id);
      this.activeTabId = this.tabs.length ? id : null;
    });
  }

  closeAllTabs(): void {
    this.guardDirty(this.tabs.map((t) => t.id), () => {
      for (const t of this.tabs) this.rememberClosed(t);
      this.subActive = {};
      this.viewSelection = {};
      this.tabs = [];
      this.activeTabId = null;
    });
  }

  /// Push a tab onto the reopen stack. Bounded because it holds file entries and
  /// a long session would otherwise keep every file ever opened alive.
  private rememberClosed(tab: OpenTab | undefined): void {
    if (!tab) return;
    this.closedTabs = [tab, ...this.closedTabs.filter((t) => t.id !== tab.id)].slice(0, 20);
  }

  /// Reopen the most recently closed tab (Ctrl+Shift+T, once the menu lands).
  reopenClosedTab(): void {
    const [tab, ...rest] = this.closedTabs;
    if (!tab) return;
    this.closedTabs = rest;
    this.openFile(tab.file);
  }

  setActive(id: string): void {
    this.activeTabId = id;
  }

  async refreshTree(): Promise<void> {
    if (!this.root) return;
    try {
      this.tree = await rescanTree(this.root);
    } catch (e) {
      this.error = String(e);
    }
  }

  /// Delete a data file: close its tab, drop cached views, remove from disk, refresh.
  async deleteFile(path: string): Promise<void> {
    await this.deleteFiles([path]);
  }

  /// Delete several data files. One that fails does not stop the rest; the
  /// failures are reported together and the folder is re-listed once.
  async deleteFiles(paths: string[]): Promise<void> {
    if (!this.root || !paths.length) return;
    this.error = null;
    this.busy = true;
    const failed: string[] = [];
    try {
      for (const path of paths) {
        try {
          await deleteFile(this.root, path);
        } catch (e) {
          failed.push(String(e));
          continue;
        }
        // The file is gone, so "save your changes?" would only recreate it. Drop
        // the buffer instead of prompting.
        this.clearDirty(path);
        this.closeTab(path);
        invalidatePath(path);
      }
      if (this.selectedFolder) await this.selectFolder(this.selectedFolder);
    } finally {
      this.busy = false;
    }
    if (failed.length) this.error = failed.length === 1 ? failed[0] : `${failed.length} files were not deleted: ${failed.join("; ")}`;
  }

  /// Move data files into workspace folder `destDir` (sidecars and `.didx`
  /// indexes go with them). An open tab follows its file, as it does a rename.
  /// A name already taken in `destDir` is skipped and reported, never overwritten.
  async moveFiles(paths: string[], destDir: string): Promise<void> {
    if (!this.root || !paths.length) return;
    this.error = null;
    // An unsaved buffer is keyed by the old path, and saving it after the move
    // would recreate the file there. Leave such a file where it is.
    const unsaved = paths.filter((p) => this.dirty[p]);
    const movable = paths.filter((p) => !this.dirty[p]);
    this.busy = true;
    try {
      const report = await apiMoveFiles(this.root, movable, destDir);
      report.failed.push(...unsaved.map((p): [string, string] => [p, "has unsaved changes; save it first"]));
      for (const [from, to] of report.moved) {
        if (from === to) continue;
        invalidatePath(from);
        if (this.tabs.some((t) => t.id === from)) {
          this.closeTab(from);
          this.openOutput(to);
        }
      }
      if (report.failed.length) {
        const names = report.failed.map(([p, why]) => `${p.split(/[\\/]/).pop()}: ${why}`);
        this.error = report.failed.length === 1 ? `Not moved — ${names[0]}` : `${report.failed.length} files were not moved: ${names.join("; ")}`;
      }
      await this.refreshFolder();
    } catch (e) {
      this.error = String(e);
    } finally {
      this.busy = false;
    }
  }

  /// The folder new files should be written to (selected folder, else root).
  get currentDir(): string | null {
    return this.selectedFolder ?? this.root;
  }

  /// Reload the currently-selected folder's file list.
  async refreshFolder(): Promise<void> {
    if (this.selectedFolder) await this.selectFolder(this.selectedFolder);
  }

  /// After creating a new file (extract / new-from-paste): refresh the folder and
  /// open the file in a viewer if its type is recognized.
  async refreshAndOpen(path: string): Promise<void> {
    await this.refreshFolder();
    this.openOutput(path);
  }

  /// Rename a data file in place; re-point an open tab and refresh the listing.
  /// Throws on failure so the caller (rename modal) can show it inline.
  async renameFile(path: string, newName: string): Promise<void> {
    if (!this.root) return;
    this.error = null;
    const np = await apiRenameFile(this.root, path, newName);
    invalidatePath(path);
    if (this.tabs.some((t) => t.id === path)) {
      this.closeTab(path);
      this.openOutput(np);
    }
    await this.refreshFolder();
  }

  /// Import (copy) files from disk into the currently-selected folder, then
  /// refresh the file list. Returns the number of files actually imported.
  async importInto(sources: string[]): Promise<number> {
    const dir = this.selectedFolder ?? this.root;
    if (!this.root || !dir || sources.length === 0) return 0;
    this.error = null;
    this.importing = sources.length;
    try {
      const imported = await importFiles(this.root, dir, sources);
      if (this.selectedFolder) await this.selectFolder(this.selectedFolder);
      return imported.length;
    } catch (e) {
      this.error = String(e);
      return 0;
    } finally {
      this.importing = 0;
    }
  }

  async createSubfolder(parent: string, name: string): Promise<void> {
    this.error = null;
    this.busy = true;
    try {
      const path = await createFolder(parent, name);
      await this.refreshTree();
      await this.selectFolder(path);
    } catch (e) {
      this.error = String(e);
    } finally {
      this.busy = false;
    }
  }

  /// Load the shared database list at startup, first making sure installed
  /// reference downloads are on it. Best-effort: never blocks the app.
  async initDatabases(): Promise<void> {
    try {
      await syncReferenceDbs();
    } catch (e) {
      this.dbError = String(e);
    }
    await this.loadDatabases();
  }

  /// Reload the shared database list. `migrateFrom` folds a workspace's old
  /// per-workspace list in first (see `ws.open`).
  async loadDatabases(migrateFrom: string | null = null): Promise<void> {
    // Launch and workspace-open both load; only the newest request may land, or
    // a slow pre-migration answer could overwrite the migrated list.
    const seq = ++this.dbLoadSeq;
    try {
      const rows = await listDatabases(migrateFrom);
      if (seq === this.dbLoadSeq) this.databases = rows;
    } catch (e) {
      this.dbError = String(e);
    }
  }
  private dbLoadSeq = 0;

  /// Rename a database on disk (shards and indexes too). Throws, so the rename
  /// modal can show the message and stay open. An open tab follows the rename,
  /// as it does for a file.
  async renameDatabase(path: string, newName: string): Promise<void> {
    const entry = await apiRenameDatabase(path, newName);
    invalidatePath(path);
    if (this.tabs.some((t) => t.id === path)) {
      this.closeTab(path);
      this.openOutput(entry.path);
    }
    await this.loadDatabases();
  }

  /// Delete a database's files and take it off the list.
  async deleteDatabase(path: string): Promise<void> {
    this.dbError = null;
    try {
      await apiDeleteDatabase(path);
      // Gone from disk: "save your changes?" would only recreate part of it.
      this.clearDirty(path);
      this.closeTab(path);
      invalidatePath(path);
    } catch (e) {
      this.dbError = String(e);
    }
    await this.loadDatabases();
  }

  /// Paths whose index is being built, for the panel's "Indexing…" state.
  indexing = $state<Record<string, boolean>>({});

  async buildDatabaseIndex(path: string): Promise<void> {
    if (this.indexing[path]) return;
    this.dbError = null;
    this.indexing[path] = true;
    try {
      await apiBuildDatabaseIndex(path);
    } catch (e) {
      // A cancel is what the user asked for, not a failure.
      if (String(e) !== INDEXING_CANCELED) this.dbError = String(e);
    } finally {
      delete this.indexing[path];
    }
    await this.loadDatabases();
  }

  /// Stop an index build. The indexer runs in a worker process, so this kills it
  /// and removes the partial index; `buildDatabaseIndex` then settles.
  async cancelDatabaseIndex(path: string): Promise<void> {
    try {
      await apiCancelDatabaseIndex(path);
    } catch (e) {
      this.dbError = `Could not cancel indexing: ${e}`;
    }
  }

  async addDatabase(path: string): Promise<void> {
    this.dbError = null;
    try {
      await registerDatabase(path);
      await this.loadDatabases();
    } catch (e) {
      this.dbError = String(e);
    }
  }

  /// Take a database off the list, leaving its files on disk.
  async removeDatabase(path: string): Promise<void> {
    try {
      await unregisterDatabase(path);
      await this.loadDatabases();
    } catch (e) {
      this.dbError = String(e);
    }
  }
}

export const ws = new WorkspaceStore();
