import {
  cancelJob as apiCancelJob,
  createFolder,
  deleteFile,
  listDatabases,
  listFolder,
  listJobs,
  openWorkspace,
  registerDatabase,
  rescanTree,
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
  if (["ssnv", "ssnview"].includes(ext)) return "ssn_viewer";
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
  error = $state<string | null>(null);
  // Requested contig to scroll to when a viewer opens (keyed by file path);
  // consumed and cleared by the contig viewer. Used by SSN → sequence nav.
  focusContig = $state<Record<string, number>>({});
  databases = $state<DatabaseEntry[]>([]);
  dbError = $state<string | null>(null);
  jobs = $state<JobInfo[]>([]);

  async open(root: string): Promise<void> {
    this.error = null;
    clearCache();
    try {
      const info = await openWorkspace(root);
      this.root = info.root;
      this.tree = info.tree;
      await this.loadDatabases();
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
    if (!this.tabs.some((t) => t.id === id)) {
      this.tabs = [...this.tabs, { id, file }];
    }
    this.activeTabId = id;
  }

  /// Open a file by path (used by cross-view nav), optionally scrolling to a contig.
  openPath(path: string, fileType: FileType, contigIndex?: number): void {
    if (contigIndex !== undefined) this.focusContig[path] = contigIndex;
    const name = path.split(/[\\/]/).pop() ?? path;
    this.openFile({ path, name, file_type: fileType, description: "", size: 0, created_by_job: null });
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

  async loadJobs(): Promise<void> {
    try {
      const list = await listJobs();
      this.jobs = list.sort((a, b) => b.started_at_ms.localeCompare(a.started_at_ms));
    } catch {
      /* ignore */
    }
  }

  async cancelJob(id: string): Promise<void> {
    try {
      await apiCancelJob(id);
    } catch {
      /* ignore */
    }
  }

  canOpen(path: string): boolean {
    return classifyPath(path) !== null;
  }

  closeTab(id: string): void {
    const idx = this.tabs.findIndex((t) => t.id === id);
    if (idx === -1) return;
    this.tabs = this.tabs.filter((t) => t.id !== id);
    if (this.activeTabId === id) {
      const next = this.tabs[idx] ?? this.tabs[idx - 1] ?? null;
      this.activeTabId = next ? next.id : null;
    }
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
    if (!this.root) return;
    this.error = null;
    try {
      await deleteFile(this.root, path);
      this.closeTab(path);
      invalidatePath(path);
      if (this.selectedFolder) await this.selectFolder(this.selectedFolder);
    } catch (e) {
      this.error = String(e);
    }
  }

  async createSubfolder(parent: string, name: string): Promise<void> {
    this.error = null;
    try {
      const path = await createFolder(parent, name);
      await this.refreshTree();
      await this.selectFolder(path);
    } catch (e) {
      this.error = String(e);
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
