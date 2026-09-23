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
  openWorkspace,
  registerDatabase,
  renameFile as apiRenameFile,
  rescanTree,
  syncReferenceDbsToWorkspace,
  unregisterDatabase,
} from "./api";
import { clearCache, invalidatePath } from "./cache";
import type { DatabaseEntry, FileEntry, FileType, FolderNode, JobInfo, OpenTab } from "./types";

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
  // unmount or replace the editor holding an unsaved buffer.
  pendingDirty = $state<{ path: string; proceed: () => void } | null>(null);
  error = $state<string | null>(null);
  // Requested contig to scroll to when a viewer opens (keyed by file path);
  // consumed and cleared by the contig viewer. Used by SSN → sequence nav.
  focusContig = $state<Record<string, number>>({});
  // Same channel for HMM files: the profile index the Logo view should show,
  // consumed and cleared by HmmLogo. Set by the profile table's "logo" button.
  focusHmmProfile = $state<Record<string, number>>({});
  // Selected sub-view per open file (e.g. "contig" | "enum" | "summary"). Lives
  // here rather than in ViewerDock so a viewer can switch the dock to a sibling
  // tab — HmmReport → Logo, for instance.
  subActive = $state<Record<string, string>>({});
  // Set while a multi-file copy is running, so the file table can show progress
  // instead of appearing to do nothing.
  importing = $state(0);
  // Set while a folder create / file delete is in flight.
  busy = $state(false);
  databases = $state<DatabaseEntry[]>([]);
  dbError = $state<string | null>(null);
  jobs = $state<JobInfo[]>([]);
  // A request to open the tool dialog (from the Tools menu or a job "clone"),
  // consumed by App. `initial` pre-fills the form (clone).
  requestedTool = $state<{ id: string; initial?: Record<string, unknown> } | null>(null);
  // A request to open the "new subfolder" input for this folder (from the folder
  // tree's context menu), consumed by App -- which owns that inline form. Same
  // consume-and-clear channel as `requestedTool`.
  requestNewFolder = $state<string | null>(null);

  /// Switch the viewer dock to sibling view `viewId` for the file at `path`.
  showSubView(path: string, viewId: string): void {
    this.guardDirty(() => {
      this.subActive[path] = viewId;
      this.activeTabId = path;
    });
  }

  /// Open the tool dialog for `id`, optionally pre-filled with `initial` params.
  openTool(id: string, initial?: Record<string, unknown>): void {
    this.requestedTool = { id, initial };
  }

  async open(root: string): Promise<void> {
    this.error = null;
    clearCache();
    try {
      const info = await openWorkspace(root);
      this.root = info.root;
      this.tree = info.tree;
      // Reference databases live in the app data dir and are shared across
      // workspaces, but the registry that tool dropdowns read is per-workspace --
      // without this, a database already on disk is invisible in a new workspace.
      // Best-effort: a failure here must not stop the workspace from opening.
      try {
        await syncReferenceDbsToWorkspace(info.root);
      } catch (e) {
        this.dbError = String(e);
      }
      await this.loadDatabases();
      await this.loadJobs(); // restore persisted job history for this workspace
      // Auto-select the root folder so the file table isn't empty on open.
      await this.selectFolder(info.tree.path);
    } catch (e) {
      this.error = String(e);
    }
  }

  async selectFolder(path: string): Promise<void> {
    this.selectedFolder = path;
    this.loadingFiles = true;
    this.error = null;
    try {
      this.files = await listFolder(this.root ?? path, path);
    } catch (e) {
      this.error = String(e);
      this.files = [];
    } finally {
      this.loadingFiles = false;
    }
  }

  openFile(file: FileEntry): void {
    const id = file.path;
    if (id === this.activeTabId) return; // already showing; nothing to unmount
    this.guardDirty(() => {
      if (!this.tabs.some((t) => t.id === id)) {
        this.tabs = [...this.tabs, { id, file }];
      }
      this.activeTabId = id;
    });
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
    if (t) this.openPath(path, t);
  }

  // --- jobs ---
  upsertJob(job: JobInfo): void {
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

  /// Ask the backend to cancel a running job. Cancellation is best-effort (the
  /// core tools have no interruption points), so the job flips to `canceling`
  /// and settles when the tool actually stops.
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

  // ── the mounted editor ─────────────────────────────────────────────────────
  // Only one editor is mounted at a time (ViewerDock keys its viewer on the
  // active tab + view), so this is a single slot rather than a map. Plain fields,
  // not `$state`: they hold a function identity that is never rendered.
  private editorPath: string | null = null;
  private editorSave: (() => Promise<void>) | null = null;

  /// Register the mounted editor's save hook; returns the unregister function,
  /// so a viewer can use it directly as an `$effect` teardown.
  registerEditor(path: string, save: () => Promise<void>): () => void {
    this.editorPath = path;
    this.editorSave = save;
    return () => {
      if (this.editorPath !== path) return; // a newer editor already took the slot
      this.editorPath = null;
      this.editorSave = null;
    };
  }

  /// File → Save: write the mounted editor's buffer. A no-op when the active view
  /// is not an editor, which is why the menu item is grayed out then.
  async saveEditor(): Promise<void> {
    await this.editorSave?.();
  }

  /// Run `proceed`, but if the mounted editor has an unsaved buffer, ask first.
  ///
  /// Every route that unmounts or replaces that editor goes through here —
  /// switching tabs, opening another file, closing a tab. Without it the buffer
  /// went with the component: `ViewerDock`'s `{#key}` destroys the viewer on a
  /// tab switch, and the teardown that clears the dirty flag ran too, so even the
  /// window-close guard could not see the edits that had just been thrown away.
  guardDirty(proceed: () => void): void {
    const p = this.editorPath;
    if (p === null || !this.dirty[p]) {
      proceed();
      return;
    }
    this.pendingDirty = { path: p, proceed };
  }

  canOpen(path: string): boolean {
    return classifyPath(path) !== null;
  }

  closeTab(id: string): void {
    const idx = this.tabs.findIndex((t) => t.id === id);
    if (idx === -1) return;
    this.guardDirty(() => {
      this.rememberClosed(this.tabs[idx]);
      this.tabs = this.tabs.filter((t) => t.id !== id);
      if (this.activeTabId === id) {
        const next = this.tabs[idx] ?? this.tabs[idx - 1] ?? null;
        this.activeTabId = next ? next.id : null;
      }
    });
  }

  /// Close every tab but `id`, which becomes active.
  closeOtherTabs(id: string): void {
    this.guardDirty(() => {
      for (const t of this.tabs) if (t.id !== id) this.rememberClosed(t);
      this.tabs = this.tabs.filter((t) => t.id === id);
      this.activeTabId = this.tabs.length ? id : null;
    });
  }

  closeAllTabs(): void {
    this.guardDirty(() => {
      for (const t of this.tabs) this.rememberClosed(t);
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
    if (id === this.activeTabId) return;
    this.guardDirty(() => (this.activeTabId = id));
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
    if (!this.root) return;
    this.error = null;
    this.busy = true;
    try {
      await deleteFile(this.root, path);
      // The file is gone, so "save your changes?" would only recreate it. Drop
      // the buffer instead of prompting.
      this.clearDirty(path);
      this.closeTab(path);
      invalidatePath(path);
      if (this.selectedFolder) await this.selectFolder(this.selectedFolder);
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

  async loadDatabases(): Promise<void> {
    if (!this.root) return;
    try {
      this.databases = await listDatabases(this.root);
    } catch (e) {
      this.dbError = String(e);
    }
  }

  async addDatabase(path: string): Promise<void> {
    if (!this.root) return;
    this.dbError = null;
    try {
      await registerDatabase(this.root, path);
      await this.loadDatabases();
    } catch (e) {
      this.dbError = String(e);
    }
  }

  async removeDatabase(path: string): Promise<void> {
    if (!this.root) return;
    try {
      await unregisterDatabase(this.root, path);
      await this.loadDatabases();
    } catch (e) {
      this.dbError = String(e);
    }
  }
}

export const ws = new WorkspaceStore();
