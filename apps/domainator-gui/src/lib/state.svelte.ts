import { listDatabases, listFolder, openWorkspace, registerDatabase, unregisterDatabase } from "./api";
import type { DatabaseEntry, FileEntry, FileType, FolderNode, OpenTab } from "./types";

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

  async open(root: string): Promise<void> {
    this.error = null;
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
      this.files = await listFolder(path);
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
