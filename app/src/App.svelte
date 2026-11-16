<script lang="ts">
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, ARCHIVE_RE, baseName, ftp, inArchive, inReadOnlyArchive, inZip, isRemote, joinPath, setPasswordPrompt, remoteId, type Drive, type Favorite, type OpKind } from "./lib/api";
  import { t, tn } from "./lib/i18n.svelte";
  import Connections from "./lib/Connections.svelte";
  import DiffView from "./lib/DiffView.svelte";
  import Help from "./lib/Help.svelte";
  import History from "./lib/History.svelte";
  import Search from "./lib/Search.svelte";
  import Settings from "./lib/Settings.svelte";
  import SyncView from "./lib/SyncView.svelte";
  import type { ToolAction } from "./lib/TitleBar.svelte";
  import Viewer from "./lib/Viewer.svelte";
  import { JobsState, type Job } from "./lib/jobs.svelte";
  import Favorites from "./lib/Favorites.svelte";
  import JobsView from "./lib/JobsView.svelte";
  import Modal from "./lib/Modal.svelte";
  import MultiRename, { type RenameItem } from "./lib/MultiRename.svelte";
  import Panel from "./lib/Panel.svelte";
  import { PARENT, PanelState, Side, setOpenFile } from "./lib/panelState.svelte";
  import TitleBar from "./lib/TitleBar.svelte";
  import { setCloseGuard, store } from "./lib/uiState";
  import { applyColors, colorTemplate } from "./lib/userColors";
  import { formatBytes, formatHuman } from "./lib/format";


  // ---------- state ----------
  let sides = $state([new Side(), new Side()]);
  let activeIdx = $state(0);
  let drives: Drive[] = $state([]);
  let toast: string | null = $state(null);
  let quick = $state("");

  const active = $derived(sides[activeIdx].current);
  const other = $derived(sides[1 - activeIdx].current);

  const jobs = new JobsState(onJobDone);

  // ---------- theme (sun / moon; follows the system until toggled) ----------
  const media = window.matchMedia("(prefers-color-scheme: dark)");
  let systemDark = $state(media.matches);
  let theme: "dark" | "light" | null = $state((store.get("theme") as "dark" | "light" | null) || null);
  const dark = $derived(theme ? theme === "dark" : systemDark);

  $effect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
    if (theme) store.set("theme", theme);
    getCurrentWindow()
      .setTheme(dark ? "dark" : "light")
      .catch(() => {});
  });

  // ---------- dialogs ----------
  type Dialog =
    | {
        kind: "input";
        title: string;
        label: string;
        value: string;
        okLabel: string;
        selectStem?: boolean;
        password?: boolean;
        /** Optional checkbox under the field (e.g. "remember password"). */
        check?: { label: string; value: boolean };
        onok: (v: string) => unknown;
        /** Closed without OK (Escape, Anuluj, replaced by another dialog). */
        oncancel?: () => void;
      }
    | {
        kind: "confirm";
        title: string;
        message: string;
        okLabel: string;
        danger?: boolean;
        /** Just information: only the OK button. */
        info?: boolean;
        check?: { label: string; value: boolean };
        onok: () => unknown;
      };
  let dialog: Dialog | null = $state(null);
  let dialogError: string | null = $state(null);
  let favorites: Favorite[] = $state([]);
  let favOpen = $state(false);
  let connOpen = $state(false);
  let viewerPath: string | null = $state(null);
  let helpOpen = $state(false);
  let historyOpen = $state(false);
  let searchOpen = $state(false);
  let syncOpen = $state(false);
  let settingsOpen = $state(false);
  let renameItems: RenameItem[] | null = $state(null);
  let fbarHidden = $state(store.get("fbarHidden") === "1");
  let toolbarHidden = $state(new Set<string>(JSON.parse(store.get("toolbarHidden") ?? "[]")));
  let diffPair: [string, string] | null = $state(null);
  const modalOpen = $derived(
    dialog !== null ||
      favOpen ||
      connOpen ||
      helpOpen ||
      historyOpen ||
      searchOpen ||
      syncOpen ||
      settingsOpen ||
      renameItems !== null ||
      viewerPath !== null ||
      diffPair !== null ||
      jobs.asks.length > 0,
  );

  async function openFavorites() {
    favorites = await api.getFavorites().catch(() => favorites);
    favOpen = true;
  }

  function saveFavorites(list: Favorite[]) {
    favorites = list;
    api.setFavorites(list).catch((e) => notify(String(e)));
  }

  function openFavorite(path: string, newTab: boolean) {
    favOpen = false;
    if (newTab) sides[activeIdx].newTab(path);
    else void active.navigate(path);
  }

  function openDialog(d: Dialog) {
    const prev = dialog;
    dialogError = null;
    dialog = d;
    if (prev?.kind === "input" && prev !== d) prev.oncancel?.();
  }

  function cancelDialog() {
    const d = dialog;
    dialog = null;
    if (d?.kind === "input") d.oncancel?.();
  }

  // Encrypted archives: any backend call that needs a password lands here
  // (api.ts retries the call with it). Kept in memory until the app closes.
  setPasswordPrompt(
    (archive, wrong) =>
      new Promise((resolve) => {
        let settled = false;
        const done = (v: string | null) => {
          if (!settled) resolve(v);
          settled = true;
        };
        openDialog({
          kind: "input",
          title: wrong ? t("Złe hasło") : t("Archiwum z hasłem"),
          label: (wrong ? t("Złe hasło — spróbuj jeszcze raz.") + " " : "") + t("Hasło do {0}:", baseName(archive)),
          value: "",
          okLabel: "OK",
          password: true,
          onok: (v) => done(v),
          oncancel: () => done(null),
        });
      }),
  );

  async function submitDialog() {
    const d = dialog;
    if (!d) return;
    try {
      if (d.kind === "input") {
        if (!d.value.trim() && !d.password) return;
        await d.onok(d.password ? d.value : d.value.trim());
      } else await d.onok();
      if (dialog === d) dialog = null; // onok may have opened the next dialog
    } catch (e) {
      dialogError = String(e);
    }
  }

  /** Selects the name without extension, like Explorer's rename. */
  function initInput(el: HTMLInputElement, selectStem: boolean) {
    setTimeout(() => {
      el.focus();
      const dot = el.value.lastIndexOf(".");
      if (selectStem && dot > 0) el.setSelectionRange(0, dot);
      else el.select();
    });
  }

  // ---------- auto-refresh: watch the dirs visible in both panels ----------
  $effect(() => {
    const paths = sides.map((s) => s.current.path).filter((p) => p);
    api.watchDirs(paths).catch(() => {});
  });

  // ---------- persistence ----------
  // Only after the saved tabs were restored — otherwise the first run of this
  // effect (empty panels at start-up) overwrote them and the app always
  // started on C:\.
  let restored = $state(false);
  $effect(() => {
    if (!restored) return;
    store.set("side0", sides[0].serialize());
    store.set("side1", sides[1].serialize());
  });

  async function restoreSide(side: Side, key: string, fallback: string) {
    let saved: { tabs: string[]; hidden?: boolean[]; index: number; brief?: boolean } | null = null;
    try {
      saved = JSON.parse(store.get(key) ?? "null");
    } catch {
      /* ignore broken state */
    }
    const paths = saved?.tabs?.length ? saved.tabs : [fallback];
    side.tabs = paths.map((_, i) => {
      const p = new PanelState();
      p.showHidden = saved?.tabs?.length ? saved.hidden?.[i] === true : false;
      return p;
    });
    side.index = Math.min(saved?.index ?? 0, paths.length - 1);
    side.brief = saved?.brief === true;
    await Promise.all(
      side.tabs.map(async (p, i) => {
        await p.navigate(paths[i]);
        if (p.error) await p.navigate(fallback); // unplugged drive etc.
      }),
    );
  }

  // Closing the window while files are being copied / uploaded would cut
  // them off mid-file: ask first.
  let forceClose = false;
  setCloseGuard(() => {
    if (forceClose || !jobs.busy) return true;
    openDialog({
      kind: "confirm",
      title: t("Trwają operacje na plikach"),
      message: t("Niedokończone operacje ({0}) zostaną przerwane w połowie. Zamknąć program?", jobs.jobs.filter((j) => !j.outcome).length),
      okLabel: t("Zamknij program"),
      danger: true,
      onok: async () => {
        // Cancel first and let the jobs tidy up (an overwrite in progress
        // keeps the old file; a new half-file is removed), then close.
        forceClose = true;
        await jobs.cancelAll(15000);
        void getCurrentWindow().close();
      },
    });
    return false;
  });

  // Colour overrides from config.toml: at start and whenever the window gets
  // focus again (back from the editor), so edits show without a restart.
  let colorsError = "";
  function loadColors() {
    api
      .getColors()
      .then((c) => {
        colorsError = "";
        const bad = applyColors(c);
        if (bad.length) notify(t("config.toml: pominięte kolory (zła nazwa lub wartość): {0}", bad.join(", ")));
      })
      .catch((e) => {
        // The whole file failed to parse: say so once, not on every focus.
        if (String(e) !== colorsError) notify(String(e));
        colorsError = String(e);
      });
  }

  onMount(async () => {
    loadColors();
    window.addEventListener("focus", loadColors);
    media.addEventListener("change", (e) => (systemDark = e.matches));
    await jobs.init();
    await listenExternalDrops();
    await listen<{ local: string; remote: string; auto: boolean }>("remote-edit-changed", (e) => onRemoteEdit(e.payload));
    await listen<string>("dir-changed", (e) => {
      const dir = e.payload.replace(/\\+$/, "").toLowerCase();
      for (const side of sides) {
        const q = side.current;
        if (!q.loading && q.path.replace(/\\+$/, "").toLowerCase() === dir) void q.reload();
      }
    });
    drives = await api.listDrives();
    const home = await api.homeDir();
    await Promise.all([restoreSide(sides[0], "side0", home), restoreSide(sides[1], "side1", "C:\\")]);
    restored = true;
  });

  // ---------- helpers ----------
  let toastTimer: ReturnType<typeof setTimeout>;
  function notify(msg: string) {
    toast = msg;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (toast = null), 2400);
  }

  function describe(names: string[]) {
    return names.length === 1 ? t("„{0}”", names[0]) : `${names.length} ${tn(names.length, "element|elementy|elementów")}`;
  }

  function parentOf(path: string): string {
    const i = path.lastIndexOf(isRemote(path) ? "/" : "\\");
    return i > 0 ? path.slice(0, i) : path;
  }

  /**
   * Destination typed in a dialog -> full path. Anything without a drive /
   * server ("backup", "..\x", "\tmp") is relative to `base` (the directory
   * the files come from) — never to the program's own folder.
   */
  function absDest(v: string, base: string): string {
    if (isRemote(v) || /^[a-zA-Z]:/.test(v) || v.startsWith("\\\\")) return v;
    if (isRemote(base)) return v.startsWith("/") ? `ftp://${remoteId(base)}${v}` : joinPath(base, v);
    if (/^[\\/]/.test(v) && /^[a-zA-Z]:/.test(base)) return base.slice(0, 2) + v; // root of the same drive
    return joinPath(base, v);
  }

  function onJobDone(job: Job) {
    if (job.keepPanels) return;
    // Unmark only what this job worked on: with a queue the user may already
    // be marking files for the next job.
    const done = new Set(job.spec.sources.map((s) => s.replace(/\\+$/, "").toLowerCase()));
    for (const side of sides) {
      const p = side.current;
      const names = p.listing?.names;
      if (names) p.marked = new Set([...p.marked].filter((i) => !done.has(joinPath(p.path, names[i]).toLowerCase())));
      void p.reload();
    }
    if (job.outcome?.cancelled) notify(t("Operacja przerwana"));
  }

  // ---------- operations ----------
  /** 7z / RAR / tar / gz: say so right away instead of asking first and failing after. */
  function needWritable(path = active.path): boolean {
    if (!inReadOnlyArchive(path)) return true;
    notify(t("Archiwa 7z, RAR, tar i gz są tylko do odczytu — można z nich kopiować (F5)"));
    return false;
  }

  function needReal(): boolean {
    if (active.isVirtual) {
      notify(t("To jest lista testowa — przejdź do prawdziwego katalogu"));
      return false;
    }
    return true;
  }

  // ---------- Explorer context menu ----------
  function contextMenu(paths: string[]) {
    if (!paths.length) return;
    api.shellContextMenu(paths).catch((e) => notify(String(e)));
  }

  // ---------- drive buttons ----------
  /** Last directory seen per drive letter (like TC), for drive switches. */
  const lastOnDrive = new Map<string, string>();
  $effect(() => {
    for (const s of sides) {
      const p = s.current.path;
      if (/^[a-z]:\\/i.test(p)) lastOnDrive.set(p[0].toUpperCase(), p);
    }
  });

  /** Same drive as the other panel -> its directory; else the last one on that drive. */
  async function gotoDrive(i: number, root: string) {
    activeIdx = i;
    const letter = root[0].toUpperCase();
    const otherPath = sides[1 - i].current.path;
    const here = sides[i].current;
    const target =
      otherPath[0]?.toUpperCase() === letter && otherPath[1] === ":"
        ? otherPath
        : here.path[0]?.toUpperCase() === letter
          ? root // clicking the current drive again = its root
          : (lastOnDrive.get(letter) ?? root);
    await here.navigate(target);
    if (here.error && target !== root) await here.navigate(root); // remembered dir gone
  }

  // ---------- drag & drop ----------
  type Drag = { from: number; paths: string[]; names: string[]; x: number; y: number; side: number | null; dir: string | null; move: boolean };
  let drag: Drag | null = $state(null);

  /** Panel / folder row under a point (client coordinates). */
  function dropTargetAt(x: number, y: number, dragged: string[]): { side: number | null; dir: string | null } {
    const el = document.elementFromPoint(x, y) as HTMLElement | null;
    const sideEl = el?.closest<HTMLElement>("[data-side]");
    const dirEl = el?.closest<HTMLElement>("[data-drop-dir]");
    let dir = dirEl?.dataset.dropDir ?? null;
    if (dir && dragged.some((p) => p.toLowerCase() === dir!.toLowerCase())) dir = null; // not into itself
    return { side: sideEl ? Number(sideEl.dataset.side) : null, dir };
  }

  function startDrag(from: number, paths: string[], names: string[], x: number, y: number) {
    drag = { from, paths, names, x, y, side: null, dir: null, move: false };
    const move = (e: MouseEvent) => {
      if (!drag) return;
      // While the button is held, WebView2 keeps capturing the mouse, so `mouseleave`
      // never fires — but mousemove keeps coming with coordinates outside the window.
      if (e.clientX < 0 || e.clientY < 0 || e.clientX >= window.innerWidth || e.clientY >= window.innerHeight) return leave();
      const hit = dropTargetAt(e.clientX, e.clientY, drag.paths);
      drag = { ...drag, x: e.clientX, y: e.clientY, move: e.shiftKey, ...hit };
    };
    const up = (e: MouseEvent) => {
      const d = drag;
      end();
      if (!d || d.side === null) return;
      const target = d.dir ?? sides[d.side].current.path;
      if (d.side === d.from && !d.dir) return; // dropped back where it started
      if (sides[d.side].current.isVirtual) return;
      copyMove(e.shiftKey ? "move" : "copy", { names: d.names, paths: d.paths }, target);
    };
    // Leaving the window hands the files over to Windows (Explorer, mail…).
    const leave = () => {
      const d = drag;
      end();
      if (d?.paths.some((p) => inArchive(p))) notify(t("Z archiwum przeciągnij na panel (rozpakowanie) — Eksplorator tych plików nie widzi"));
      else if (d && !d.paths.some((p) => isRemote(p))) {
        api.dragOut(d.paths, d.move ? "move" : "copy").catch((e) => notify(String(e)));
      }
    };
    const end = () => {
      drag = null;
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
      document.documentElement.removeEventListener("mouseleave", leave);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
    document.documentElement.addEventListener("mouseleave", leave);
  }

  // Files dropped from Explorer (or from our own drag-out coming back).
  let externalDrop: { side: number | null; dir: string | null } | null = $state(null);
  async function listenExternalDrops() {
    await getCurrentWebview().onDragDropEvent((e) => {
      const p = e.payload;
      if (p.type === "leave") return void (externalDrop = null);
      const dpr = window.devicePixelRatio || 1;
      const hit = dropTargetAt(p.position.x / dpr, p.position.y / dpr, p.type === "drop" ? p.paths : []);
      if (p.type === "over" || p.type === "enter") externalDrop = hit;
      else if (p.type === "drop") {
        externalDrop = null;
        if (hit.side === null || !p.paths.length || modalOpen) return;
        const target = hit.dir ?? sides[hit.side].current.path;
        if (sides[hit.side].current.isVirtual) return;
        // Dropped onto the same folder it came from: nothing to do.
        const parent = p.paths[0].slice(0, p.paths[0].lastIndexOf("\\"));
        if (parent.toLowerCase() === target.replace(/\\+$/, "").toLowerCase()) return;
        copyMove("copy", { names: p.paths.map((x) => baseName(x)), paths: p.paths }, target);
      }
    });
  }

  /** Alt+F5: pack the selection into a ZIP in the other panel. */
  function pack() {
    if (!needReal()) return;
    const { names, paths } = active.selection();
    if (!names.length) return;
    const base = names.length === 1 ? names[0].replace(/\.[^.\\]*$/, "") || names[0] : baseName(active.path) || t("archiwum");
    const destDir = other.isVirtual ? active.path : other.path;
    const from = active.path;
    openDialog({
      kind: "input",
      title: t("Spakuj {0} do ZIP", describe(names)),
      label: t("Plik archiwum:"),
      value: joinPath(destDir, base + ".zip"),
      okLabel: t("Spakuj"),
      onok: (v) => {
        const dest = absDest(/\.zip$/i.test(v) ? v : v + ".zip", from);
        return jobs.start({ kind: "pack", sources: paths, dest }, t("Pakowanie {0} → {1}", describe(names), baseName(dest)));
      },
    });
  }

  /** Shift+F5: copy in the same directory under a new name. */
  function copyHere() {
    if (!needReal() || !needWritable()) return;
    const { names, paths } = active.selection();
    if (names.length !== 1) return notify(t("Kopia w tym samym katalogu — ustaw kursor na jednym pliku lub katalogu"));
    const p = active;
    const [stem, ext] = names[0].includes(".") ? [names[0].slice(0, names[0].lastIndexOf(".")), names[0].slice(names[0].lastIndexOf("."))] : [names[0], ""];
    openDialog({
      kind: "input",
      title: t("Kopiuj „{0}” tutaj", names[0]),
      label: t("Nowa nazwa:"),
      value: `${stem} - ${t("kopia")}${ext}`,
      okLabel: t("Kopiuj"),
      selectStem: true,
      onok: async (v) => {
        if (v === names[0]) throw new Error(t("Podaj inną nazwę"));
        if (/[\\/:*?"<>|]/.test(v)) throw new Error(t("Nazwa nie może zawierać znaków {0}", '\\ / : * ? " < > |'));
        await jobs.start({ kind: "copy", sources: paths, dest: joinPath(p.path, v) }, t("Kopiowanie {0} → {1}", names[0], v));
      },
    });
  }

  const historyList = $derived([...active.back].reverse().concat([...active.fwd].reverse()));

  async function openHistory(path: string) {
    historyOpen = false;
    await active.navigate(path);
  }

  function openSearch() {
    if (active.isVirtual) return notify(t("Wyszukiwanie działa na prawdziwych katalogach"));
    searchOpen = true;
  }

  /** Search result -> show it in the active panel (cursor on the file). */
  async function gotoFound(path: string) {
    searchOpen = false;
    const i = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
    const remoteRoot = isRemote(path) && path.indexOf("/", 6) === i; // ftp://id/file
    const dir = i > 2 && !remoteRoot ? path.slice(0, i) : path.slice(0, i + 1);
    await active.navigate(dir, path.slice(i + 1));
  }

  function openSync() {
    if (sides.some((s) => s.current.isVirtual)) return notify(t("Porównanie działa na prawdziwych katalogach"));
    syncOpen = true;
  }

  // ---------- folder sizes (Space, Alt+Shift+Enter) ----------
  async function sizeDir(p: PanelState, name: string) {
    const key = name.toLowerCase();
    const path = p.path;
    p.sizing = key;
    try {
      const s = await api.dirSize(joinPath(path, name));
      if (p.path !== path) return; // left the folder meanwhile
      if (!s.partial) p.dirSizes = new Map(p.dirSizes).set(key, s.bytes);
    } catch (e) {
      notify(String(e));
    } finally {
      if (p.sizing === key) p.sizing = null;
    }
  }

  /** Space on a folder: mark it and compute its size (like TC). */
  async function spaceKey(p: PanelState) {
    const i = p.entryAt(p.cursor);
    const name = p.currentName;
    p.toggleMark();
    if (name && i !== undefined && p.isDir(i) && !p.isVirtual && p.marked.has(i)) await sizeDir(p, name);
  }

  /** Alt+Shift+Enter: sizes of all folders here, one after another (Esc stops). */
  async function sizeAllDirs(p: PanelState) {
    const l = p.listing;
    if (!l || p.isVirtual) return;
    const path = p.path;
    const names = [...p.order].filter((i) => p.isDir(i)).map((i) => l.names[i]);
    sizingAll = true;
    for (const n of names) {
      if (!sizingAll || p.path !== path) break;
      await sizeDir(p, n);
    }
    sizingAll = false;
  }
  let sizingAll = false;

  /** Ctrl+Q: total size and count of files / folders of the selection (or the item under the cursor). */
  async function countSelection(p: PanelState) {
    const l = p.listing;
    if (!l || p.isVirtual) return;
    const path = p.path;
    let idx = [...p.order].filter((i) => p.marked.has(i));
    if (!idx.length) {
      const i = p.entryAt(p.cursor);
      idx = i === PARENT || i === undefined ? [] : [i];
    }
    if (!idx.length) return;
    let bytes = 0,
      files = 0,
      dirs = 0;
    sizingAll = true;
    try {
      for (const i of idx) {
        if (!p.isDir(i)) {
          bytes += l.sizes[i];
          files++;
          continue;
        }
        const name = l.names[i];
        const key = name.toLowerCase();
        p.sizing = key;
        try {
          const s = await api.dirSize(joinPath(path, name));
          if (p.path !== path) return;
          if (s.partial || !sizingAll) return notify(t("Liczenie przerwane"));
          p.dirSizes = new Map(p.dirSizes).set(key, s.bytes);
          bytes += s.bytes;
          files += s.files;
          dirs += 1 + s.dirs;
        } finally {
          if (p.sizing === key) p.sizing = null;
        }
      }
    } catch (e) {
      return notify(String(e));
    } finally {
      sizingAll = false;
    }
    const what = idx.length === 1 ? t("„{0}”", l.names[idx[0]]) : `${idx.length} ${tn(idx.length, "element|elementy|elementów")}`;
    openDialog({
      kind: "confirm",
      info: true,
      title: t("Rozmiar: {0}", what),
      message:
        `${t("Rozmiar:")} ${formatHuman(bytes)} (${formatBytes(bytes)} B)\n` +
        `${t("Pliki:")} ${formatBytes(files)}\n` +
        `${t("Katalogi:")} ${formatBytes(dirs)}`,
      okLabel: "OK",
      onok: () => {},
    });
  }

  function stopSizing() {
    sizingAll = false;
    void api.dirSizeCancel();
  }

  // ---------- mark by mask (Num + / Num −) ----------
  let lastMask = "*";
  function markByMask(add: boolean) {
    const p = active;
    if (!p.listing) return;
    openDialog({
      kind: "input",
      title: add ? t("Zaznacz według maski") : t("Odznacz według maski"),
      label: t("Maska (np. *.php;*.js, fragment nazwy; „*\\” = katalogi):"),
      value: lastMask,
      okLabel: add ? t("Zaznacz") : t("Odznacz"),
      onok: (v) => {
        lastMask = v;
        const n = p.markByMask(v, add);
        if (!n) notify(t("Nic nie pasuje do „{0}”", v));
      },
    });
  }

  // ---------- clipboard (Ctrl+C / Ctrl+X / Ctrl+V, Ctrl+Shift+C, Ctrl+Alt+C) ----------
  function clipFiles(cut: boolean) {
    if (!needReal()) return;
    const { names, paths } = active.selection();
    if (!names.length) return;
    api
      .clipboardSetFiles(paths, cut)
      .then(() => notify(cut ? t("Wycięto do schowka: {0}", describe(names)) : t("Skopiowano do schowka: {0}", describe(names))))
      .catch((e) => notify(String(e)));
  }

  function clipText(fullPaths: boolean) {
    if (!needReal()) return;
    const { names, paths } = active.selection();
    if (!names.length) return;
    const text = (fullPaths ? paths : names).join("\r\n");
    api
      .clipboardSetText(text)
      .then(() => notify(fullPaths ? t("Ścieżki w schowku ({0})", names.length) : t("Nazwy w schowku ({0})", names.length)))
      .catch((e) => notify(String(e)));
  }

  async function paste() {
    if (!needReal() || !needWritable()) return;
    const p = active;
    let clip;
    try {
      clip = await api.clipboardGetFiles();
    } catch (e) {
      return notify(String(e));
    }
    if (!clip?.paths.length) return notify(t("W schowku nie ma plików"));
    const here = p.path.replace(/\\+$/, "").toLowerCase();
    if (!clip.cut && clip.paths.every((x) => parentOf(x).toLowerCase() === here)) {
      return notify(t("Te pliki już są w tym katalogu — kopia pod nową nazwą: Shift+F5"));
    }
    const names = clip.paths.map((x) => baseName(x));
    const kind = clip.cut ? "move" : "copy";
    const dest = joinPath(p.path, "");
    const out = await jobs.run(
      { kind, sources: clip.paths, dest },
      kind === "copy" ? t("Kopiowanie {0} → {1}", describe(names), dest) : t("Przenoszenie {0} → {1}", describe(names), dest),
    );
    // Moved files are gone from their old place: don't offer them again.
    if (clip.cut && !out.cancelled && !out.errors.length) void api.clipboardClear();
  }

  /** Ctrl+← / Ctrl+→: the folder (or archive) under the cursor — else this one — in the left / right panel. */
  function openIn(target: number) {
    const p = active;
    if (p.isVirtual) return;
    const i = p.entryAt(p.cursor);
    const name = p.currentName;
    const enter = name && i !== undefined && i !== PARENT && (p.isDir(i) || (ARCHIVE_RE.test(name) && !isRemote(p.path)));
    const path = enter ? joinPath(p.path, name!) : p.path;
    void sides[target].current.navigate(path); // focus stays in this panel
  }

  /** Ctrl+M: rename the marked entries (or the one under the cursor) with a mask. */
  function multiRename() {
    if (!needReal() || !needWritable()) return;
    const p = active;
    const l = p.listing;
    if (!l) return;
    let idx = [...p.order].filter((i) => p.marked.has(i));
    if (!idx.length) {
      const i = p.entryAt(p.cursor);
      if (i === undefined || i === PARENT) return;
      idx = [i];
    }
    renameItems = idx.map((i) => ({ name: l.names[i], dir: p.isDir(i), mtime: l.mtimes[i] }));
  }

  function openTerminal() {
    if (active.isVirtual || isRemote(active.path)) return notify(t("Terminal tylko dla katalogów lokalnych"));
    api.openTerminal(active.path).catch((e) => notify(String(e)));
  }

  // Title-bar toolbar (icons; every action also has a shortcut, see F1).
  // `label` is the Polish name: translated for the tooltip, and kept as the
  // stable id under which hidden buttons are remembered (state.json).
  const tool = (label: string, keys: string, icon: string, run: () => void, extra: Partial<ToolAction> = {}): ToolAction => ({
    id: label,
    title: t(label) + (keys ? ` (${keys})` : ""),
    icon,
    run,
    ...extra,
  });
  const toolbar: ToolAction[] = $derived([
    tool("Odśwież", "Ctrl+R", "M13 8a5 5 0 1 1-1.5-3.5M13 2.5V5h-2.5", () => active.refresh()),
    tool("Wstecz", "Alt+←", "M10 3.5 5.5 8l4.5 4.5", () => void active.goBack()),
    tool("Dalej", "Alt+→", "M6 3.5 10.5 8 6 12.5", () => void active.goForward()),
    tool("Historia katalogów", "Alt+↓", "M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM8 4.5V8l2.5 1.5", () => (historyOpen = true)),
    tool("Nowa zakładka", "Ctrl+T", "M2.5 5.5h11v8h-11zM2.5 5.5 4 3h4l1.5 2.5M8 7.5v4M6 9.5h4", () => sides[activeIdx].newTab(), { sep: true }),
    tool("Szukaj plików", "Alt+F7", "M7 2.5a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9zM10.3 10.3 14 14", openSearch),
    tool("Ulubione", "Ctrl+D", "M8 2l1.8 3.8 4.2.5-3.1 2.9.8 4.1L8 11.3l-3.7 2 .8-4.1L2 6.3l4.2-.5z", () => void openFavorites()),
    tool("Filtr", "Ctrl+S", "M2 3h12l-4.5 5.5V13l-3 1.5V8.5z", () => (sides[activeIdx].editingFilter = true), { active: !!active.filter }),
    tool("Pokaż ukryte", "Ctrl+H", "M1.5 8S4 3.5 8 3.5c1.4 0 2.6.5 3.6 1.2M14.5 8S12 12.5 8 12.5c-1.4 0-2.6-.5-3.6-1.2M6.6 9.4a2 2 0 0 1 2.8-2.8M2.5 13.5l11-11", () => active.toggleHidden(), { active: active.showHidden }),
    tool("Nowy katalog", "F7", "M1.5 3.5h4L7 5h7.5v8.5h-13zM8 7.5v4M6 9.5h4", makeDir, { sep: true }),
    tool("Spakuj do ZIP", "Alt+F5", "M1.5 3.5h13v9h-13zM3.5 6h2.6l-2.6 4h2.6M8 6v4M10.2 10V6h1.5a1.2 1.2 0 0 1 0 2.4h-1.5", pack),
    tool("Nowy plik", "Shift+F4", "M3.5 1.5h6l3 3v10h-9zM8 7v5M5.5 9.5h5", newFile),
    tool("Zmiana nazw wielu plików", "Ctrl+M", "M2 12 4.5 4 7 12M3 9.5h3M9 12.5 13 8.5l1.5 1.5-4 4H9z", multiRename),
    tool("Porównaj pliki", "Ctrl+F2", "M4.5 2.5v5M2 5h5M9 11h5M2.5 13.5l11-11", compare),
    tool("Porównaj / synchronizuj katalogi", "Shift+F2", "M2.5 2.5h4.5v11H2.5zM9 2.5h4.5v11H9zM4 6h1.5M10.5 6H12M4 9h1.5M10.5 9H12", openSync),
    tool("Terminal w tym katalogu", "", "M2 3h12v10H2zM4.5 6.5 6.5 8l-2 1.5M8 10h3", openTerminal),
    tool("Połączenia FTP", "Ctrl+F", "M4.5 12.5a3 3 0 0 1-.4-6A4 4 0 0 1 11.8 5.5a3.5 3.5 0 0 1-.3 7z", () => (connOpen = true), { sep: true }),
  ]);

  function copyMove(
    kind: Extract<OpKind, "copy" | "move">,
    from?: { names: string[]; paths: string[] },
    target?: string,
  ) {
    if (!from && !needReal()) return;
    const { names, paths } = from ?? active.selection();
    if (!names.length) return;
    // Moving out of an archive = deleting from it. The folder the items are in,
    // not the item: F6 on a plain backup.tar.gz file is fine.
    if (kind === "move" && !needWritable(parentOf(paths[0]))) return;
    if (target && !needWritable(target)) return; // dragged onto a read-only archive
    const what = describe(names);
    const dest = target ? joinPath(target, "") : other.isVirtual ? active.path : joinPath(other.path, "");
    const base = parentOf(paths[0]);
    openDialog({
      kind: "input",
      title: kind === "copy" ? t("Kopiuj {0}", what) : t("Przenieś {0}", what),
      label: t("Do:"),
      value: dest,
      okLabel: kind === "copy" ? t("Kopiuj") : t("Przenieś"),
      onok: (v) => {
        const to = absDest(v, base);
        return jobs.start({ kind, sources: paths, dest: to }, kind === "copy" ? t("Kopiowanie {0} → {1}", what, to) : t("Przenoszenie {0} → {1}", what, to));
      },
    });
  }

  function remove(permanent: boolean) {
    if (!needReal() || !needWritable()) return;
    const { names, paths } = active.selection();
    if (!names.length) return;
    const archive = inArchive(active.path);
    if (isRemote(active.path) || archive) permanent = true; // no recycle bin on FTP / in archives
    openDialog({
      kind: "confirm",
      title: isRemote(active.path) ? t("Usuń z serwera") : archive ? t("Usuń z archiwum") : permanent ? t("Usuń trwale") : t("Przenieś do kosza"),
      message: permanent
        ? t("Usunąć TRWALE {0}? Tej operacji nie można cofnąć.", describe(names))
        : t("Przenieść do kosza {0}?", describe(names)),
      okLabel: permanent ? t("Usuń trwale") : t("Do kosza"),
      danger: permanent,
      onok: () =>
        jobs.start({ kind: permanent ? "delete" : "trash", sources: paths }, permanent ? t("Usuwanie {0}", describe(names)) : t("Do kosza: {0}", describe(names))),
    });
  }

  function makeDir() {
    if (!needReal() || !needWritable()) return;
    const p = active;
    openDialog({
      kind: "input",
      title: t("Nowy katalog"),
      label: t("Nazwa (można podać ścieżkę a\\b\\c):"),
      value: "",
      okLabel: t("Utwórz"),
      onok: async (v) => {
        await api.createDir(joinPath(p.path, v));
        await p.reload(v.split(/[\\/]/)[0]);
        reloadOthers(p);
      },
    });
  }

  /** Shift+F4: new (text) file in the active dir, then open it in the editor. */
  function newFile() {
    if (!needReal() || !needWritable()) return;
    const p = active;
    const cur = p.currentName && !p.isDir(p.entryAt(p.cursor)) ? p.currentName : "";
    openDialog({
      kind: "input",
      title: t("Nowy plik"),
      label: t("Nazwa (istniejący plik zostanie po prostu otwarty):"),
      value: cur || t("nowy.txt"),
      okLabel: t("Utwórz i edytuj"),
      selectStem: true,
      onok: async (v) => {
        if (/[*?"<>|]/.test(v)) throw new Error(t("Nazwa nie może zawierać znaków {0}", '* ? " < > |'));
        if (isRemote(p.path)) {
          // FTP: empty file on the server, then edit it like F4 (upload on save).
          if (/[\\/]/.test(v)) throw new Error(t("Na FTP podaj samą nazwę pliku"));
          const remotePath = joinPath(p.path, v);
          const exists = p.listing?.names.some((n) => n.toLowerCase() === v.toLowerCase());
          const local = exists ? await api.editRemote(remotePath) : await api.editNewRemote(remotePath);
          if (!exists) await p.reload(v);
          await api.runTool("editor", [local]);
          notify(t("Po zapisie w edytorze zapytam o wysłanie na serwer"));
          return;
        }
        const full = /^([a-zA-Z]:|\\\\)/.test(v) ? v : joinPath(p.path, v);
        const created = await api.createFile(full);
        if (created) {
          await p.reload(v.split(/[\\/]/)[0]);
          reloadOthers(p);
        }
        await api.runTool("editor", [full]);
      },
    });
  }

  /** Other visible panels showing the same directory see the change too. */
  function reloadOthers(changed: PanelState) {
    const dir = changed.path.toLowerCase();
    for (const side of sides) {
      const q = side.current;
      if (q !== changed && !q.isVirtual && q.path.toLowerCase() === dir) void q.reload();
    }
  }

  function rename() {
    if (!needReal() || !needWritable()) return;
    const p = active;
    const name = p.currentName;
    if (!name) return;
    openDialog({
      kind: "input",
      title: t("Zmień nazwę „{0}”", name),
      label: t("Nowa nazwa:"),
      value: name,
      okLabel: t("Zmień"),
      selectStem: !p.isDir(p.entryAt(p.cursor)),
      onok: async (v) => {
        if (v === name) return;
        if (/[\\/:*?"<>|]/.test(v)) throw new Error(t("Nazwa nie może zawierać znaków {0}", '\\ / : * ? " < > |'));
        await api.renamePath(joinPath(p.path, name), joinPath(p.path, v));
        await p.reload(v);
        reloadOthers(p);
      },
    });
  }

  function currentFilePath(p: PanelState): string | null {
    const i = p.entryAt(p.cursor);
    if (p.isVirtual || !p.currentName || p.isDir(i)) return null;
    return joinPath(p.path, p.currentName);
  }

  /** Remote files from this size on are downloaded as a job (progress, cancel, retries). */
  const BIG_REMOTE = 1024 * 1024;

  /** Size of the entry under the cursor (0 if unknown). */
  function cursorSize(p: PanelState): number {
    const i = p.entryAt(p.cursor);
    return p.listing && i !== undefined && i >= 0 ? p.listing.sizes[i] : 0;
  }

  /**
   * A big remote file -> downloaded to %TEMP% as a normal job; returns the
   * local copy, or null when cancelled / failed (the job card shows why).
   * Small files return null at once: the backend fetches them directly.
   */
  async function fetchBig(remote: string, size: number): Promise<string | null | undefined> {
    if (!isRemote(remote) || size < BIG_REMOTE) return undefined;
    const local = await api.remoteTempPath(remote);
    const out = await jobs.run({ kind: "copy", sources: [remote], dest: local }, t("Pobieranie {0}", baseName(remote)), { keepPanels: true });
    return out.cancelled || out.errors.length || !out.doneFiles ? null : local;
  }

  /** Enter on a file: big remote ones come down with progress first. */
  setOpenFile(async (full, size) => {
    const big = await fetchBig(full, size);
    if (big === null) return;
    await api.openPath(big ?? full);
  });

  async function runTool(kind: "editor" | "viewer") {
    const f = currentFilePath(active);
    if (!f) return notify(t("Ustaw kursor na pliku"));
    try {
      const big = await fetchBig(f, cursorSize(active));
      if (big === null) return;
      if (kind === "editor" && (isRemote(f) || inZip(f))) {
        if (!big) notify(isRemote(f) ? t("Pobieranie do edycji…") : t("Wypakowanie do edycji…"));
        const local = await api.editRemote(f, big);
        await api.runTool("editor", [local]);
        notify(isRemote(f) ? t("Po zapisie w edytorze zapytam o wysłanie na serwer") : t("Po zapisie w edytorze zapytam o aktualizację archiwum"));
      } else if (!(await api.runTool(kind, [big ?? f]))) viewerPath = big ?? f; // 'builtin' viewer
    } catch (e) {
      notify(String(e));
    }
  }

  // ---------- remote edit: upload after save ----------
  const askingUpload = new Set<string>();
  /** Uploads in progress (local copy -> another save came in meanwhile). One at a time per file. */
  const uploading = new Map<string, boolean>();

  async function uploadEdited(local: string, remote: string) {
    if (uploading.has(local)) {
      uploading.set(local, true); // sent again once the running upload ends
      return;
    }
    uploading.set(local, false);
    try {
      await uploadOnce(local, remote);
    } finally {
      const again = uploading.get(local);
      uploading.delete(local);
      if (again) void uploadEdited(local, remote);
    }
  }

  async function uploadOnce(local: string, remote: string) {
    const zip = !isRemote(remote);
    try {
      notify(zip ? t("Aktualizacja archiwum: {0}…", baseName(remote)) : t("Wysyłanie {0}…", baseName(remote)));
      await api.editUpload(local, remote);
      notify(zip ? t("Zaktualizowano w archiwum: {0}", baseName(remote)) : t("Wysłano na serwer: {0}", baseName(remote)));
      const dir = remote.slice(0, remote.lastIndexOf(zip ? "\\" : "/")) || remote;
      for (const side of sides) {
        const q = side.current;
        if (q.path.replace(/[\\/]$/, "").toLowerCase() === dir.toLowerCase()) void q.reload();
      }
    } catch (e) {
      openDialog({
        kind: "confirm",
        title: zip ? t("Nie udało się zaktualizować archiwum") : t("Nie udało się wysłać pliku"),
        message: `${remote}

${e}

${t("Kopia lokalna: {0}", local)}`,
        okLabel: t("Ponów"),
        onok: () => uploadEdited(local, remote),
      });
    }
  }

  function onRemoteEdit(c: { local: string; remote: string; auto: boolean }) {
    if (c.auto) return void uploadEdited(c.local, c.remote);
    if (askingUpload.has(c.local)) return;
    if (dialog) return void setTimeout(() => onRemoteEdit(c), 1000); // don't clobber another dialog
    askingUpload.add(c.local);
    const d: Dialog = {
      kind: "confirm",
      title: t("Plik zmieniony w edytorze"),
      message:
        (isRemote(c.remote) ? t("Wysłać „{0}” na serwer?", baseName(c.remote)) : t("Zaktualizować „{0}” w archiwum?", baseName(c.remote))) +
        "\n" +
        c.remote,
      okLabel: isRemote(c.remote) ? t("Wyślij") : t("Aktualizuj"),
      check: {
        label: isRemote(c.remote)
          ? t("Wysyłaj automatycznie przy kolejnych zapisach tego pliku")
          : t("Aktualizuj automatycznie przy kolejnych zapisach tego pliku"),
        value: false,
      },
      onok: async () => {
        askingUpload.delete(c.local);
        if (d.check?.value) await api.editSetAuto(c.local, true);
        await uploadEdited(c.local, c.remote);
      },
    };
    openDialog(d);
  }

  $effect(() => {
    // Closing the question without sending clears the "asking" mark.
    if (!dialog) askingUpload.clear();
  });

  /**
   * Compare: two marked files in the active panel, or the file under the
   * cursor against the other panel's cursor file / the same name there.
   */
  function compare() {
    let pair: string[] | null = null;
    const sel = active.selection();
    const l = active.listing;
    const markedFiles = [...active.marked].filter((i) => !active.isDir(i));
    if (markedFiles.length === 2 && l) pair = markedFiles.map((i) => joinPath(active.path, l.names[i]));
    else {
      const left = currentFilePath(active);
      if (left && !other.isVirtual) {
        const right = currentFilePath(other) ?? (sel.names.length === 1 ? joinPath(other.path, sel.names[0]) : null);
        if (right) pair = [left, right];
      }
    }
    if (!pair) return notify(t("Zaznacz 2 pliki albo ustaw kursory na plikach w obu panelach"));
    if (activeIdx === 1) pair.reverse(); // keep left panel on the left
    const p = pair as [string, string];
    api
      .runTool("diff", p)
      .then((launched) => {
        if (!launched) diffPair = p;
        else notify(t("Porównanie {0} ↔ {1} — otwarte w zewnętrznym programie", baseName(p[0]), baseName(p[1])));
      })
      .catch((e) => notify(String(e)));
  }

  function settings() {
    settingsOpen = true;
  }

  function setToolbarHidden(h: Set<string>) {
    toolbarHidden = h;
    store.set("toolbarHidden", JSON.stringify([...h]));
  }

  // ---------- FTP ----------
  /** Connect flow: password prompt and certificate trust as needed. */
  async function connect(id: string, password: string | null = null, remember = false) {
    const target = sides[activeIdx];
    notify(t("Łączenie…"));
    let r;
    try {
      r = await ftp.connect(id, password, remember);
    } catch (e) {
      toast = null;
      openDialog({ kind: "confirm", title: t("Nie można połączyć"), message: String(e), okLabel: t("Ponów"), onok: () => connect(id, password, remember) });
      return;
    }
    toast = null;
    if (r.kind === "ok") {
      connOpen = false;
      await target.current.navigate(r.path);
    } else if (r.kind === "needPassword") {
      const d: Dialog = {
        kind: "input",
        title: t("Hasło — {0}", r.name),
        label: (r.error ? r.error + "\n" : "") + t("Hasło dla {0}:", r.user || "anonymous"),
        value: "",
        okLabel: t("Połącz"),
        password: true,
        check: { label: t("Zapamiętaj hasło (zaszyfrowane dla Twojego konta Windows)"), value: true },
        onok: (v) => connect(id, v, d.kind === "input" && !!d.check?.value),
      };
      openDialog(d);
    } else {
      openDialog({
        kind: "confirm",
        title: t("Niezaufany certyfikat — {0}", r.name),
        message:
          t("Serwer {0} przedstawił certyfikat, którego Windows nie potwierdza ({1}).", r.host, r.reason) +
          `\n\nSHA-256: ${r.sha256}\n\n` +
          t("To typowe dla hostingów łączonych po IP lub z certyfikatem self-signed. Zaufać temu certyfikatowi na stałe? Jeśli kiedyś się zmieni, program zapyta ponownie."),
        okLabel: t("Zaufaj i połącz"),
        onok: async () => {
          await ftp.trust(id, r.sha256);
          await connect(id, password, remember);
        },
      });
    }
  }

  async function disconnect(sideIdx = activeIdx) {
    const p = sides[sideIdx].current;
    const id = remoteId(p.path);
    if (!id) return notify(t("Ten panel nie jest połączony z FTP"));
    await ftp.disconnect(id).catch(() => {});
    await p.navigate(await api.homeDir());
    notify(t("Rozłączono"));
  }

  // ---------- keyboard ----------
  let quickTimer: ReturnType<typeof setTimeout>;
  function quickSearch(ch: string) {
    clearTimeout(quickTimer);
    const p = active;
    // Same single letter again -> next match (like Explorer).
    const next = quick.length === 1 && quick === ch.toLowerCase();
    const q = next ? quick : quick + ch.toLowerCase();
    const row = p.findPrefix(q, next ? p.cursor + 1 : p.cursor);
    if (row >= 0) {
      p.cursor = row;
      quick = q;
    }
    quickTimer = setTimeout(() => (quick = ""), 1200);
  }

  // Browser shortcuts that must never reach WebView2 (reload, find, print, history…).
  const BLOCK_CTRL = new Set(["f", "g", "p", "r", "u", "j", "s", "o", "n", "h", "e", "l", "k", "d", "t", "w", "m"]);

  function onKey(e: KeyboardEvent) {
    const k = e.key;
    const ctrl = e.ctrlKey,
      alt = e.altKey,
      shift = e.shiftKey;

    if (/^F\d{1,2}$/.test(k)) e.preventDefault(); // every F-key is ours
    if (alt && (k === "ArrowLeft" || k === "ArrowRight" || k === "ArrowDown")) e.preventDefault(); // browser history
    if (ctrl && BLOCK_CTRL.has(k.toLowerCase())) e.preventDefault();
    if (modalOpen) return;

    const p = active;
    const side = sides[activeIdx];
    const plain = !ctrl && !alt;
    let handled = true;

    if (k === "Escape" && (active.sizing || sizingAll)) stopSizing();
    else if (k === "Escape" && quick) quick = "";
    else if (k === "Backspace" && quick) quick = quick.slice(0, -1);
    else if (k === "Tab" && ctrl) side.cycle(shift ? -1 : 1);
    else if (k === "Tab" && !alt) activeIdx = 1 - activeIdx;
    else if (alt && k === "ArrowLeft") void p.goBack();
    else if (alt && k === "ArrowRight") void p.goForward();
    else if (alt && k === "ArrowDown") historyOpen = true;
    else if (alt && k === "F7") openSearch();
    else if (k === "ContextMenu" || (shift && !ctrl && !alt && k === "F10")) contextMenu(p.selection().paths);
    else if (shift && !ctrl && !alt && k === "F2") openSync();
    else if (ctrl && !alt && !shift && k === "ArrowLeft") openIn(0);
    else if (ctrl && !alt && !shift && k === "ArrowRight") openIn(1);
    else if (alt && shift && k === "Enter") void sizeAllDirs(p);
    else if (ctrl && !alt && !shift && k.toLowerCase() === "q") void countSelection(p);
    else if (ctrl && !alt && !shift && k.toLowerCase() === "c") clipFiles(false);
    else if (ctrl && !alt && !shift && k.toLowerCase() === "x") clipFiles(true);
    else if (ctrl && !alt && !shift && k.toLowerCase() === "v") void paste();
    else if (ctrl && shift && !alt && k.toLowerCase() === "c") clipText(true);
    else if (ctrl && alt && !shift && k.toLowerCase() === "c") clipText(false);
    else if (k === "ArrowUp" && ctrl) {
      const name = p.currentName;
      if (name && p.isDir(p.entryAt(p.cursor))) side.newTab(joinPath(p.path, name));
    } else if (plain && side.brief && (k === "ArrowLeft" || k === "ArrowRight")) {
      if (shift) p.toggleMark();
      p.moveCursor(k === "ArrowLeft" ? -p.colRows : p.colRows);
    } else if (k === "ArrowDown") {
      if (shift) p.toggleMark();
      p.moveCursor(1);
    } else if (k === "ArrowUp") {
      if (shift) p.toggleMark();
      p.moveCursor(-1);
    } else if (k === "PageDown" && ctrl) void p.activate();
    else if (k === "PageUp" && ctrl) void p.goParent();
    else if (k === "PageDown" && shift) p.markMove(p.pageRows);
    else if (k === "PageUp" && shift) p.markMove(-p.pageRows);
    else if (k === "Home" && shift) p.markMove(-p.cursor, true);
    else if (k === "End" && shift) p.markMove(p.rowCount - 1 - p.cursor, true);
    else if (k === "PageDown") p.moveCursor(p.pageRows);
    else if (k === "PageUp") p.moveCursor(-p.pageRows);
    else if (k === "Home") p.cursor = 0;
    else if (k === "End") p.cursor = p.rowCount - 1;
    else if (k === "Enter" && !alt) void p.activate();
    else if (k === "Backspace") void p.goParent();
    else if (k === "Insert") {
      p.toggleMark();
      p.moveCursor(1);
    } else if (k === " " && !quick) void spaceKey(p);
    else if (e.code === "NumpadMultiply") p.invertMarks();
    else if (ctrl && k.toLowerCase() === "a") p.markAll();
    else if (e.code === "NumpadAdd") markByMask(true);
    else if (e.code === "NumpadSubtract" && ctrl) p.marked = new Set();
    else if (e.code === "NumpadSubtract") markByMask(false);
    else if (ctrl && k.toLowerCase() === "d") void openFavorites();
    else if (ctrl && k.toLowerCase() === "l") side.editingPath = true;
    else if (ctrl && !shift && k.toLowerCase() === "s") side.editingFilter = true;
    else if (k === "Escape" && p.filter) p.setFilter("");
    else if (ctrl && !alt && !shift && k === "F1") side.brief = !side.brief;
    else if (k === "F1") helpOpen = true;
    else if (ctrl && shift && k.toLowerCase() === "f") void disconnect();
    else if (ctrl && k.toLowerCase() === "f") connOpen = true;
    else if (ctrl && k.toLowerCase() === "t") side.newTab();
    else if (ctrl && k.toLowerCase() === "w") side.closeTab();
    else if (ctrl && k.toLowerCase() === "r") p.refresh();
    else if (ctrl && k.toLowerCase() === "h") {
      p.toggleHidden();
      notify(p.showHidden ? t("Pokazuję ukryte i systemowe") : t("Ukrywam ukryte i systemowe"));
    } else if (ctrl && k.toLowerCase() === "u") sides = [sides[1], sides[0]];
    else if (ctrl && !alt && !shift && k.toLowerCase() === "m") multiRename();
    else if (ctrl && k === "F3") p.setSort("name");
    else if (ctrl && k === "F4") p.setSort("ext");
    else if (ctrl && k === "F5") p.setSort("date");
    else if (ctrl && k === "F6") p.setSort("size");
    else if (ctrl && alt && k.toLowerCase() === "f") void p.loadFake(100_000);
    else if (ctrl && alt && k.toLowerCase() === "m") void p.loadFake(1_000_000);
    else if (ctrl && !alt && k === "F2") compare();
    else if (plain && k === "F3") void runTool("viewer");
    else if (plain && shift && k === "F4") newFile();
    else if (plain && k === "F4") void runTool("editor");
    else if (alt && !ctrl && k === "F5") pack();
    else if (plain && shift && k === "F5") copyHere();
    else if (plain && k === "F5") copyMove("copy");
    else if (plain && (k === "F2" || (shift && k === "F6"))) rename();
    else if (plain && k === "F6") copyMove("move");
    else if (plain && k === "F7") makeDir();
    else if (plain && (k === "F8" || k === "Delete")) remove(shift);
    else if (plain && k.length === 1 && k !== " ") quickSearch(k);
    else handled = false;

    if (handled) e.preventDefault();
  }

  const fkeys: [string, string, () => void][] = $derived([
    ["F2", t("Zmień nazwę"), rename],
    ["F3", t("Podgląd"), () => runTool("viewer")],
    ["F4", t("Edycja"), () => runTool("editor")],
    ["F5", t("Kopiuj"), () => copyMove("copy")],
    ["F6", t("Przenieś"), () => copyMove("move")],
    ["F7", t("Katalog"), makeDir],
    ["F8", t("Usuń"), () => remove(false)],
    ["^F2", t("Porównaj"), compare],
  ]);
</script>

<svelte:window onkeydown={onKey} oncontextmenu={(e) => e.preventDefault()} />

<div class="app" class:no-fbar={fbarHidden}>
  <TitleBar
    {dark}
    ontoggletheme={() => (theme = dark ? "light" : "dark")}
    onsettings={settings}
    onhelp={() => (helpOpen = true)}
    subtitle={active?.path ?? ""}
    actions={toolbar.filter((a) => !toolbarHidden.has(a.id))}
  />

  <main class="panels">
    {#each sides as s, i (s)}
      <Panel side={s} active={i === activeIdx} {drives} onfocus={() => (activeIdx = i)} onftp={() => ((activeIdx = i), (connOpen = true))}
        ondisconnect={() => disconnect(i)}
        index={i}
        ondrive={(root) => gotoDrive(i, root)}
        oncontext={contextMenu}
        ondragstart={(paths, names, x, y) => startDrag(i, paths, names, x, y)}
        dropHere={(drag?.side === i && !drag.dir && drag.from !== i) || (externalDrop?.side === i && !externalDrop.dir)}
        dropDir={drag?.side === i ? drag.dir : externalDrop?.side === i ? externalDrop.dir : null}
      />
    {/each}
  </main>

  {#if !fbarHidden}
    <nav class="fbar">
      {#each fkeys as [key, label, run] (key)}
        <button onclick={run} tabindex="-1"><kbd>{key}</kbd>{label}</button>
      {/each}
    </nav>
  {/if}

  {#if quick}<div class="quick">{t("Szukaj:")} <b>{quick}</b></div>{/if}
  {#if drag}
    <div class="drag-badge" style:left="{drag.x + 14}px" style:top="{drag.y + 10}px">
      {drag.move ? t("Przenieś") : t("Kopiuj")}
      {describe(drag.names)}{#if drag.side !== null && (drag.dir || drag.side !== drag.from)}
        → {baseName(drag.dir ?? sides[drag.side].current.path)}{/if}
      <small>{drag.move ? "" : t("Shift = przenieś") + " · "}{t("poza okno = do Eksploratora")}</small>
    </div>
  {/if}
  {#if toast}<div class="toast">{toast}</div>{/if}
</div>

<JobsView {jobs} />

{#if searchOpen}
  <Search
    root={active.path}
    onopen={gotoFound}
    onview={(f) => (viewerPath = f)}
    onedit={(f) =>
      (isRemote(f) || inZip(f) ? api.editRemote(f).then((l) => api.runTool("editor", [l])) : api.runTool("editor", [f])).catch((e) =>
        notify(String(e)),
      )}
    onclose={() => (searchOpen = false)}
  />
{/if}
{#if syncOpen}
  <SyncView
    left={sides[0].current.path}
    right={sides[1].current.path}
    {jobs}
    onclose={() => (syncOpen = false)}
    ondone={() => sides.forEach((s) => void s.current.reload())}
  />
{/if}
{#if renameItems}
  {@const p = active}
  <MultiRename
    dir={p.path}
    items={renameItems}
    others={p.listing?.names ?? []}
    onclose={() => (renameItems = null)}
    ondone={(focus) => {
      void p.reload(focus ?? undefined);
      reloadOthers(p);
    }}
  />
{/if}
{#if settingsOpen}
  <Settings
    actions={toolbar}
    hidden={toolbarHidden}
    {theme}
    {fbarHidden}
    onfbar={(h) => {
      fbarHidden = h;
      store.set("fbarHidden", h ? "1" : "");
    }}
    ontoolbar={setToolbarHidden}
    ontheme={(th) => {
      theme = th;
      if (!th) store.set("theme", "");
    }}
    onconfig={() => api.openConfig(colorTemplate()).catch((e) => notify(String(e)))}
    onclose={() => (settingsOpen = false)}
  />
{/if}
{#if helpOpen}
  <Help onclose={() => (helpOpen = false)} />
{/if}
{#if historyOpen}
  <History list={historyList} onopen={openHistory} onclose={() => (historyOpen = false)} />
{/if}
{#if viewerPath}
  <Viewer path={viewerPath} onclose={() => (viewerPath = null)} />
{/if}
{#if diffPair}
  <DiffView left={diffPair[0]} right={diffPair[1]} onclose={() => (diffPair = null)} />
{/if}

{#if connOpen}
  <Connections current={remoteId(active.path)} onconnect={(id) => connect(id)} onclose={() => (connOpen = false)} />
{/if}

{#if favOpen}
  <Favorites
    list={favorites}
    currentPath={active.path}
    onopen={openFavorite}
    onchange={saveFavorites}
    onclose={() => (favOpen = false)}
  />
{/if}

{#if dialog}
  {@const d = dialog}
  <Modal
    title={d.title}
    onclose={cancelDialog}
    buttons={[
      { label: d.okLabel, primary: true, danger: d.kind === "confirm" && d.danger, run: submitDialog },
      ...(d.kind === "confirm" && d.info ? [] : [{ label: t("Anuluj"), run: cancelDialog }]),
    ]}
  >
    {#if d.kind === "input"}
      <label class="field">
        <span>{d.label}</span>
        <input
          bind:value={d.value}
          use:initInput={!!d.selectStem}
          type={d.password ? "password" : "text"}
          spellcheck="false"
          autocomplete={d.password ? "current-password" : "off"}
        />
      </label>
    {:else}
      <p class="msg">{d.message}</p>
    {/if}
    {#if d.check}
      <label class="check"><input type="checkbox" bind:checked={d.check.value} /> {d.check.label}</label>
    {/if}
    {#if dialogError}<p class="dlg-error">{dialogError}</p>{/if}
  </Modal>
{/if}

<style>
  .app {
    height: 100vh;
    display: grid;
    grid-template-rows: auto 1fr auto;
    grid-template-columns: minmax(0, 1fr); /* long title-bar path must not widen the window content */
    gap: 6px;
    padding: 0 6px 6px;
    box-sizing: border-box;
  }
  .app.no-fbar {
    grid-template-rows: auto 1fr;
  }
  .panels {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
    min-height: 0;
  }
  .fbar {
    display: grid;
    grid-template-columns: repeat(8, 1fr);
    gap: 6px;
  }
  .fbar button {
    font: inherit;
    font-size: 12px;
    color: var(--fg);
    background: var(--panel-bg);
    border: 1px solid var(--border);
    border-radius: 5px;
    padding: 4px;
    cursor: pointer;
  }
  .fbar button:hover {
    border-color: var(--border-strong);
  }
  kbd {
    font: inherit;
    font-weight: 700;
    color: var(--accent);
    margin-right: 6px;
  }
  .quick,
  .toast {
    position: fixed;
    left: 50%;
    transform: translateX(-50%);
    padding: 6px 14px;
    border-radius: 6px;
    background: var(--overlay-bg);
    color: var(--overlay-fg);
    font-size: 13px;
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.25);
    pointer-events: none;
    z-index: 95;
  }
  .quick {
    bottom: 56px;
  }
  .drag-badge {
    position: fixed;
    z-index: 96;
    pointer-events: none;
    padding: 5px 10px;
    border-radius: 6px;
    background: var(--overlay-bg);
    color: var(--overlay-fg);
    font-size: 12px;
    box-shadow: 0 4px 16px rgb(0 0 0 / 0.3);
    white-space: nowrap;
  }
  .drag-badge small {
    display: block;
    opacity: 0.7;
    font-size: 10.5px;
  }
  .toast {
    top: 44px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field span {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .field input {
    font: inherit;
    padding: 6px 8px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  .field input:focus {
    border-color: var(--accent);
  }
  .msg,
  .field span {
    margin: 0;
    white-space: pre-line;
    overflow-wrap: anywhere;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 10px;
    font-size: 12px;
  }
  .dlg-error {
    margin: 10px 0 0;
    color: var(--danger);
    font-size: 12px;
  }
</style>
