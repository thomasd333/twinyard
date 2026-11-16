import { api, ARCHIVE_RE, ATTR_DIR, ATTR_HIDDEN, ATTR_SYSTEM, baseName, isRemote, joinPath, type Listing } from "./api";
import { splitExt } from "./format";

export type SortKey = "name" | "ext" | "size" | "date";

/** How Enter opens a file (set by the app: big FTP files come down as a job first). */
let openFile: (path: string, size: number) => Promise<void> = (path) => api.openPath(path);
export function setOpenFile(f: typeof openFile) {
  openFile = f;
}

/** Row index meaning the ".." entry. */
export const PARENT = -1;

export class PanelState {
  path = $state("");
  listing: Listing | null = $state.raw(null);
  /** Visible rows: listing indices in display order (".." excluded). */
  order: Uint32Array = $state.raw(new Uint32Array(0));
  hasParent = $state(false);
  // Clamp on write: negative / past-end rows never exist.
  #cursor = $state(0);
  get cursor() {
    return this.#cursor;
  }
  set cursor(v: number) {
    this.#cursor = Math.max(0, Math.min(v, this.rowCount - 1));
  }
  /** Marked listing indices. Replaced (not mutated) to trigger updates. */
  marked: Set<number> = $state.raw(new Set());
  sortKey: SortKey = $state("name");
  sortDesc = $state(false);
  showHidden = $state(false);
  /** Quick filter (Ctrl+S): "*.php;*.js", "?x*", or plain text = contains. */
  filter = $state("");
  /** Directory history (Alt+← / Alt+→ / Alt+↓). */
  back: string[] = $state([]);
  fwd: string[] = $state([]);
  loading = $state(false);
  error: string | null = $state(null);
  timing = $state({ backend: 0, ipc: 0, sort: 0 });
  /** Rows per page, set by the view (PgUp/PgDn). */
  pageRows = $state(20);
  /** Brief view: rows per column, set by the view (← / → jump a column). */
  colRows = $state(20);
  /** Folder sizes computed with Space (lower-case name -> bytes); kept over reloads of the same dir. */
  dirSizes: Map<string, number> = $state.raw(new Map());
  /** Folder whose size is being computed (lower-case name), for the "…" in the size column. */
  sizing: string | null = $state(null);

  /** Monotonic request id: late answers for older paths are dropped. */
  #req = 0;

  get rowCount(): number {
    return this.order.length + (this.hasParent ? 1 : 0);
  }

  /** Listing index for a display row, or PARENT for "..". */
  entryAt(row: number): number {
    if (this.hasParent) return row === 0 ? PARENT : this.order[row - 1];
    return this.order[row];
  }

  isDir(i: number): boolean {
    return i === PARENT || ((this.listing!.attrs[i] & ATTR_DIR) !== 0);
  }

  /** Items an operation applies to: marked ones, else the one under the cursor. */
  selection(): { names: string[]; paths: string[] } {
    const l = this.listing;
    if (!l) return { names: [], paths: [] };
    let idx = [...this.order].filter((i) => this.marked.has(i));
    if (!idx.length) {
      const i = this.entryAt(this.cursor);
      idx = i === PARENT || i === undefined ? [] : [i];
    }
    const names = idx.map((i) => l.names[i]);
    return { names, paths: names.map((n) => joinPath(this.path, n)) };
  }

  get currentName(): string | null {
    const i = this.entryAt(this.cursor);
    return i === PARENT || i === undefined || !this.listing ? null : this.listing.names[i];
  }

  get isVirtual(): boolean {
    return this.path.startsWith("::");
  }

  /**
   * Reload keeping the view: cursor goes to `focusName` (or the current
   * name); if that is gone (deleted), stays on the same row number.
   */
  async reload(focusName?: string) {
    if (this.isVirtual || !this.path) return;
    const row = this.cursor;
    const name = focusName ?? this.currentName ?? undefined;
    const marked = new Set([...this.marked].map((i) => this.listing!.names[i]));
    await this.navigate(this.path, name);
    if (name && this.currentName?.toLowerCase() !== name.toLowerCase()) {
      this.cursor = Math.min(row, Math.max(0, this.rowCount - 1));
    }
    // Keep marks on entries that still exist (e.g. after a refresh).
    if (marked.size && this.listing) {
      const l = this.listing;
      this.marked = new Set([...this.order].filter((i) => marked.has(l.names[i])));
    }
  }

  async navigate(path: string, focusName?: string, record = true) {
    const prev = this.path;
    const req = ++this.#req;
    this.loading = true;
    const t0 = performance.now();
    try {
      const [listing, parent] = await Promise.all([api.listDir(path), api.parentDir(path)]);
      if (req !== this.#req) return;
      const t1 = performance.now();
      if (listing.path.toLowerCase() !== prev.toLowerCase()) {
        if (record && prev && !prev.startsWith("::")) {
          this.back = [...this.back.filter((p) => p.toLowerCase() !== prev.toLowerCase()), prev].slice(-50);
          this.fwd = [];
        }
        this.filter = ""; // a filter belongs to one directory
        this.dirSizes = new Map();
        this.sizing = null;
      }
      this.listing = listing;
      this.path = listing.path;
      this.hasParent = parent !== null;
      this.marked = new Set();
      this.error = null;
      this.resort();
      this.timing = { ...this.timing, backend: listing.elapsedMs, ipc: t1 - t0 - listing.elapsedMs };
      this.cursor = 0;
      if (focusName) this.focusByName(focusName);
    } catch (e) {
      if (req === this.#req) this.error = String(e);
    } finally {
      if (req === this.#req) this.loading = false;
    }
  }

  async loadFake(count: number) {
    const req = ++this.#req;
    const t0 = performance.now();
    const listing = await api.fakeListing(count);
    if (req !== this.#req) return;
    const t1 = performance.now();
    this.listing = listing;
    this.path = listing.path;
    this.hasParent = false;
    this.marked = new Set();
    this.error = null;
    this.resort();
    this.timing = { ...this.timing, backend: listing.elapsedMs, ipc: t1 - t0 - listing.elapsedMs };
    this.cursor = 0;
  }

  refresh() {
    void this.reload();
  }

  async goParent() {
    if (!this.hasParent) return;
    const parent = await api.parentDir(this.path);
    if (parent) await this.navigate(parent, baseName(this.path));
  }

  /** Enter: open dir / run file. */
  async activate() {
    const i = this.entryAt(this.cursor);
    if (i === undefined || !this.listing) return;
    if (i === PARENT) return this.goParent();
    const full = joinPath(this.path, this.listing.names[i]);
    if (this.isDir(i)) return this.navigate(full);
    // ZIP / 7z / RAR / tar / gz: step inside like a folder (local disks only).
    if (ARCHIVE_RE.test(full) && !isRemote(full)) return this.navigate(full);
    await openFile(full, this.listing.sizes[i]).catch((e) => (this.error = String(e)));
  }

  setSort(key: SortKey) {
    if (this.sortKey === key) this.sortDesc = !this.sortDesc;
    else {
      this.sortKey = key;
      this.sortDesc = key === "size" || key === "date"; // newest/biggest first, like TC
    }
    const cur = this.entryAt(this.cursor);
    this.resort();
    if (cur !== undefined) this.focusEntry(cur);
  }

  toggleHidden() {
    this.showHidden = !this.showHidden;
    const cur = this.entryAt(this.cursor);
    this.resort();
    this.focusEntry(cur ?? PARENT);
  }

  /** Rebuilds `order`: filter + sort. Integer compares only (see nameRank). */
  setFilter(f: string) {
    const name = this.currentName;
    this.filter = f;
    this.resort();
    if (name) this.focusByName(name);
  }

  async goBack() {
    const target = this.back.at(-1);
    if (!target) return;
    const from = this.path;
    this.back = this.back.slice(0, -1);
    await this.navigate(target, undefined, false);
    if (this.error) return;
    this.fwd = [...this.fwd, from];
  }

  async goForward() {
    const target = this.fwd.at(-1);
    if (!target) return;
    const from = this.path;
    this.fwd = this.fwd.slice(0, -1);
    await this.navigate(target, undefined, false);
    if (this.error) return;
    this.back = [...this.back, from];
  }

  resort() {
    const l = this.listing;
    if (!l) return;
    const t0 = performance.now();
    const n = l.names.length;
    const hide = this.showHidden ? 0 : ATTR_HIDDEN | ATTR_SYSTEM;
    const idx: number[] = [];
    const match = compileFilter(this.filter);
    for (let i = 0; i < n; i++) if ((l.attrs[i] & hide) === 0 && match(l.names[i])) idx.push(i);

    const { attrs, sizes, mtimes, nameRank: rank } = l;
    const dir = this.sortDesc ? -1 : 1;
    let cmp: (a: number, b: number) => number;
    switch (this.sortKey) {
      case "size":
        cmp = (a, b) => sizes[a] - sizes[b] || rank[a] - rank[b];
        break;
      case "date":
        cmp = (a, b) => mtimes[a] - mtimes[b] || rank[a] - rank[b];
        break;
      case "ext": {
        const exts = new Map<number, string>();
        for (const i of idx) exts.set(i, splitExt(l.names[i])[1].toLowerCase());
        cmp = (a, b) => {
          const ea = exts.get(a)!, eb = exts.get(b)!;
          return ea < eb ? -1 : ea > eb ? 1 : rank[a] - rank[b];
        };
        break;
      }
      default:
        cmp = (a, b) => rank[a] - rank[b];
    }
    // Folders have no size/ext: by name, reversed only when sorting by name.
    const dirDir = this.sortKey === "name" ? dir : 1;
    idx.sort((a, b) => {
      const da = attrs[a] & ATTR_DIR, db = attrs[b] & ATTR_DIR;
      if (da !== db) return da ? -1 : 1; // folders always on top
      if (da) return (rank[a] - rank[b]) * dirDir;
      return cmp(a, b) * dir;
    });
    this.order = Uint32Array.from(idx);
    this.timing = { ...this.timing, sort: performance.now() - t0 };
    if (this.cursor >= this.rowCount) this.cursor = Math.max(0, this.rowCount - 1);
  }

  focusEntry(i: number) {
    if (i === PARENT) {
      this.cursor = 0;
      return;
    }
    const pos = this.order.indexOf(i);
    this.cursor = pos < 0 ? 0 : pos + (this.hasParent ? 1 : 0);
  }

  focusByName(name: string) {
    const l = this.listing;
    if (!l) return;
    const lower = name.toLowerCase();
    const i = l.names.findIndex((n) => n.toLowerCase() === lower);
    if (i >= 0) this.focusEntry(i);
  }

  /** Quick search: first row whose name starts with prefix, searching from `from`. */
  findPrefix(prefix: string, from = 0): number {
    const l = this.listing;
    if (!l || !prefix) return -1;
    const p = prefix.toLowerCase();
    const total = this.rowCount;
    for (let k = 0; k < total; k++) {
      const row = (from + k) % total;
      const i = this.entryAt(row);
      if (i !== PARENT && l.names[i].toLowerCase().startsWith(p)) return row;
    }
    return -1;
  }

  moveCursor(delta: number) {
    const max = this.rowCount - 1;
    this.cursor = Math.max(0, Math.min(max, this.cursor + delta));
  }

  toggleMark(row = this.cursor) {
    const i = this.entryAt(row);
    if (i === PARENT || i === undefined) return;
    const m = new Set(this.marked);
    if (m.has(i)) m.delete(i);
    else m.add(i);
    this.marked = m;
  }

  /** Sets the mark of every entry in rows a..b (either order, inclusive) on `base` (default: current marks). */
  markRange(a: number, b: number, on: boolean, base: Set<number> = this.marked) {
    const m = new Set(base);
    const lo = Math.max(0, Math.min(a, b));
    const hi = Math.min(this.rowCount - 1, Math.max(a, b));
    for (let row = lo; row <= hi; row++) {
      const i = this.entryAt(row);
      if (i === PARENT || i === undefined) continue;
      if (on) m.add(i);
      else m.delete(i);
    }
    this.marked = m;
  }

  /**
   * Shift+PgUp/PgDn/Home/End (TC): marks the rows passed over, cursor row included,
   * with the opposite of the cursor entry's state — like holding Shift+↓.
   * `toEdge` (Home/End) also marks the target row.
   */
  markMove(delta: number, toEdge = false) {
    const from = this.cursor;
    const to = Math.max(0, Math.min(this.rowCount - 1, from + delta));
    if (to === from && !toEdge) return this.toggleMark();
    const i = this.entryAt(from);
    const on = i === PARENT || i === undefined ? true : !this.marked.has(i);
    // Hitting the list edge marks the last row too (Shift+↓ held at the bottom does).
    const end = toEdge || to !== from + delta ? to : to - Math.sign(delta);
    this.markRange(from, end, on);
    this.cursor = to;
  }

  invertMarks() {
    const m = new Set<number>();
    for (const i of this.order) if (!this.marked.has(i) && !this.isDir(i)) m.add(i);
    this.marked = m;
  }

  markAll() {
    this.marked = new Set(this.order);
  }

  /**
   * Num + / Num −: (un)mark by mask ("*.php;*.js", plain text = contains).
   * Like TC: files only; a mask ending with "\" (e.g. "*\") means folders.
   */
  markByMask(mask: string, add: boolean): number {
    const l = this.listing;
    if (!l) return 0;
    const dirs = mask.trim().endsWith("\\");
    const match = compileFilter(dirs ? mask.trim().slice(0, -1) || "*" : mask);
    const m = new Set(this.marked);
    let n = 0;
    for (const i of this.order) {
      if (this.isDir(i) !== dirs || !match(l.names[i])) continue;
      if (add ? !m.has(i) : m.has(i)) n++;
      if (add) m.add(i);
      else m.delete(i);
    }
    this.marked = m;
    return n;
  }

  /** Size of a folder entry: computed by Space, else undefined. */
  dirSize(i: number): number | undefined {
    return this.listing ? this.dirSizes.get(this.listing.names[i].toLowerCase()) : undefined;
  }

  /** Tab caption: last path segment, or the drive. */
  get title(): string {
    if (this.isVirtual) return "test";
    return baseName(this.path) || this.path;
  }

  /** Recomputed only when listing/order/marks change, not on every render. */
  summary = $derived.by(() => {
    const l = this.listing;
    let files = 0, dirs = 0, bytes = 0, mFiles = 0, mDirs = 0, mBytes = 0;
    if (l) {
      for (const i of this.order) {
        const d = (l.attrs[i] & ATTR_DIR) !== 0;
        const m = this.marked.has(i);
        if (d) {
          dirs++;
          if (m) mDirs++;
          // Folders sized with Space count too (like TC).
          const ds = this.dirSizes.get(l.names[i].toLowerCase());
          if (ds !== undefined) {
            bytes += ds;
            if (m) mBytes += ds;
          }
        } else {
          files++;
          bytes += l.sizes[i];
          if (m) {
            mFiles++;
            mBytes += l.sizes[i];
          }
        }
      }
    }
    return { files, dirs, bytes, mFiles, mDirs, mBytes };
  });
}

/** One side of the window: a set of tabs, each a full panel. */
export class Side {
  tabs: PanelState[] = $state([]);
  index = $state(0);
  /** Path bar is in edit mode (click on the path or Ctrl+L). */
  editingPath = $state(false);
  /** Quick-filter input is open (Ctrl+S). */
  editingFilter = $state(false);
  /** Brief view (names only, in columns) instead of full (Ctrl+F1). */
  brief = $state(false);
  current: PanelState = $derived(this.tabs[Math.min(this.index, this.tabs.length - 1)]);

  constructor() {
    this.tabs = [new PanelState()];
  }

  /** Ctrl+T: duplicate current tab (or open `path`) right after it. */
  newTab(path?: string) {
    const src = this.current;
    const p = new PanelState();
    p.sortKey = src.sortKey;
    p.sortDesc = src.sortDesc;
    p.showHidden = src.showHidden;
    const tabs = [...this.tabs];
    tabs.splice(this.index + 1, 0, p);
    this.tabs = tabs;
    this.index = this.index + 1;
    void p.navigate(path ?? src.path, path ? undefined : (src.currentName ?? undefined));
  }

  closeTab(i = this.index) {
    if (this.tabs.length <= 1) return;
    this.tabs = this.tabs.filter((_, k) => k !== i);
    if (this.index >= i && this.index > 0) this.index--;
  }

  cycle(delta: number) {
    const n = this.tabs.length;
    this.index = (this.index + delta + n) % n;
  }

  /** For localStorage: tab paths and the active index. */
  serialize(): string {
    // FTP tabs are not restored (they would need a password / reconnect).
    const kept = this.tabs.filter((t) => t.path && !t.path.startsWith("::") && !/^ftp:\/\//i.test(t.path));
    return JSON.stringify({
      tabs: kept.map((t) => t.path),
      hidden: kept.map((t) => t.showHidden),
      index: Math.min(this.index, Math.max(0, kept.length - 1)),
      brief: this.brief,
    });
  }
}

/** "*.php;*.js" / "a?c" (wildcards, ';' or space separated) or plain text (contains). */
export function compileFilter(f: string): (name: string) => boolean {
  const parts = f.split(/[;\s]+/).map((p) => p.trim().toLowerCase()).filter(Boolean);
  if (!parts.length) return () => true;
  const tests = parts.map((p) => {
    if (!/[*?]/.test(p)) return (n: string) => n.includes(p);
    const re = new RegExp("^" + p.replace(/[.+^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".") + "$");
    return (n: string) => re.test(n);
  });
  return (name) => {
    const n = name.toLowerCase();
    return tests.some((t) => t(n));
  };
}
