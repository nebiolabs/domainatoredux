// Shell UI state the menu has to reach.
//
// These used to be `let` locals inside App.svelte and FileTable.svelte, which is
// fine for a button right next to them but unreachable from a native menu event.
// The counters are request channels in the same consume-and-clear spirit as
// `ws.requestedTool`: a menu handler bumps one, the component that owns the
// behaviour `$effect`s on it. A counter rather than a boolean so two presses in
// a row both register.
export const ui = $state({
  aboutRequest: 0,
  preferencesRequest: 0,
  updateCheckRequest: 0,
  newFileRequest: 0,
  findRequest: 0,
  saveRequest: 0,
  exportRequest: 0,
  clearJobsRequest: 0,
  sidebarVisible: true,
  tableVisible: true,
  /// Which left-hand panel to expand and scroll to, consumed by that panel.
  focusPanel: null as "databases" | "refdbs" | "external" | null,
});
