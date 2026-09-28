// Session cache for computed viewer payloads (enum report, matrix report, contig
// view, HMM logo, SSN html), keyed by command + path + params. It's a module
// singleton, so it survives component remounts. Views stay mounted across tab
// switches (ViewerDock's keep-alive), so what this saves now is recomputing a
// file that was closed and reopened.
//
// Promises (not resolved values) are cached, so concurrent requests for the same
// key dedupe. Failed loads are not cached (so they can be retried).

const store = new Map<string, Promise<unknown>>();

export function cached<T>(key: string, loader: () => Promise<T>): Promise<T> {
  let p = store.get(key) as Promise<T> | undefined;
  if (!p) {
    p = loader().catch((e) => {
      store.delete(key);
      throw e;
    });
    store.set(key, p);
  }
  return p;
}

/// Drop every cached entry whose key references `path` — call after editing a file
/// so its views recompute from the new contents.
export function invalidatePath(path: string): void {
  for (const k of [...store.keys()]) {
    if (k.includes(path)) store.delete(k);
  }
}

/// Drop everything (e.g. when opening a different workspace).
export function clearCache(): void {
  store.clear();
}
