<script lang="ts">
  import { api, joinPath, type Drive } from "./api";
  import { formatBytes, formatDate, formatHuman, splitExt } from "./format";
  import { FOLDER_SVG, fileIconSvg } from "./fileIcons";
  import { hasOwnIcon, shellIcon } from "./shellIcons.svelte";
  import { t } from "./i18n.svelte";
  import { PARENT, type Side, type SortKey } from "./panelState.svelte";

  interface Props {
    side: Side;
    active: boolean;
    drives: Drive[];
    onfocus: () => void;
    onftp: () => void;
    ondisconnect: () => void;
    /** Explorer context menu for these paths. */
    oncontext: (paths: string[]) => void;
    /** Drive button: the app decides where to go (same dir as the other panel…). */
    ondrive: (root: string) => void;
    /** Index of this panel (0 left, 1 right) — drop target lookup. */
    index: number;
    /** A drag left the start threshold: these entries are being dragged. */
    ondragstart: (paths: string[], names: string[], x: number, y: number) => void;
    /** Drop highlight: the whole panel, or one folder row (its full path). */
    dropHere?: boolean;
    dropDir?: string | null;
  }
  let { side, active, drives, onfocus, onftp, ondisconnect, oncontext, ondrive, index, ondragstart, dropHere = false, dropDir = null }: Props = $props();

  // ---------- drag start (mouse) ----------
  function rowDown(row: number, e: MouseEvent) {
    if (e.button === 2) return rightDown(row, e);
    if (e.button !== 0) return;
    const sx = e.clientX,
      sy = e.clientY;
    const move = (m: MouseEvent) => {
      if (Math.abs(m.clientX - sx) + Math.abs(m.clientY - sy) < 6) return;
      cleanup();
      const i = panel.entryAt(row);
      if (i === PARENT || i === undefined || !panel.listing || panel.isVirtual) return;
      // Dragging a marked entry drags the whole selection, else just this one.
      let names: string[], paths: string[];
      if (panel.marked.has(i)) ({ names, paths } = panel.selection());
      else {
        panel.cursor = row;
        names = [panel.listing.names[i]];
        paths = [joinPath(panel.path, names[0])];
      }
      ondragstart(paths, names, m.clientX, m.clientY);
    };
    const cleanup = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", cleanup);
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", cleanup);
  }
  const panel = $derived(side.current);

  function tabMouse(i: number, e: MouseEvent) {
    if (e.button === 1) {
      e.preventDefault();
      side.closeTab(i);
    }
  }

  const ROW = 22;
  const OVERSCAN = 8;

  let scroller: HTMLDivElement;
  let scrollTop = $state(0);
  let scrollLeft = $state(0);
  let viewH = $state(400);
  let viewW = $state(600);

  const rowCount = $derived(panel.rowCount);
  const brief = $derived(side.brief);

  // ---------- brief view: names only, in columns (Ctrl+F1) ----------
  /** Rows per column. The brief scroller always shows its horizontal bar, so this is stable. */
  const rpc = $derived(Math.max(1, Math.floor(viewH / ROW)));

  let measureCtx: CanvasRenderingContext2D | null = null;
  /** Width of the longest visible name (icon and padding included), in px. */
  const longest = $derived.by(() => {
    const l = panel.listing;
    const order = panel.order; // dependency: filter / hidden change what is visible
    if (!brief || !l) return 0;
    measureCtx ??= document.createElement("canvas").getContext("2d");
    if (!measureCtx) return 0;
    const family = getComputedStyle(document.documentElement).fontFamily;
    // Huge folders: measuring every name costs; only the longest ones can win.
    let maxLen = 0;
    for (let k = 0; k < order.length; k++) maxLen = Math.max(maxLen, l.names[order[k]].length);
    const minLen = order.length > 5000 ? maxLen - 8 : 0;
    let w = 0;
    for (const bold of [false, true]) {
      measureCtx.font = `${bold ? 600 : 400} 13px ${family}`;
      for (let k = 0; k < order.length; k++) {
        const i = order[k];
        const n = l.names[i];
        if (n.length < minLen || panel.isDir(i) !== bold) continue;
        w = Math.max(w, measureCtx.measureText(n).width);
      }
    }
    return Math.ceil(w) + 16 + 6 + 16 + 2; // icon + gap + padding
  });
  const colCount = $derived(Math.ceil(rowCount / rpc));
  /** Column width: fits the longest name, but at most half the panel (else brief is pointless);
      a single column (few files) may use the whole width. */
  const colW = $derived(Math.max(90, Math.min(longest, colCount > 1 ? Math.floor(viewW / 2) : viewW)));

  const rows = $derived.by(() => {
    let first: number, last: number;
    if (brief) {
      const c0 = Math.max(0, Math.floor(scrollLeft / colW) - 1);
      const c1 = Math.ceil((scrollLeft + viewW) / colW) + 1;
      first = c0 * rpc;
      last = Math.min(rowCount, c1 * rpc);
    } else {
      first = Math.max(0, Math.floor(scrollTop / ROW) - OVERSCAN);
      last = Math.min(rowCount, Math.ceil((scrollTop + viewH) / ROW) + OVERSCAN);
    }
    return Array.from({ length: Math.max(0, last - first) }, (_, k) => first + k);
  });
  $effect(() => {
    panel.colRows = rpc;
    panel.pageRows = brief
      ? Math.max(1, rpc * Math.max(1, Math.floor(viewW / colW)) - 1)
      : Math.max(1, Math.floor(viewH / ROW) - 1);
  });

  // Keep the cursor in view; on a fresh listing centre it (e.g. after "..").
  let shownListing: unknown = null;
  $effect(() => {
    const l = panel.listing;
    if (!scroller) return;
    const fresh = l !== shownListing;
    shownListing = l;
    if (brief) {
      const left = Math.floor(panel.cursor / rpc) * colW;
      if (fresh) scroller.scrollLeft = Math.max(0, left - (viewW - colW) / 2);
      else if (left < scroller.scrollLeft) scroller.scrollLeft = left;
      else if (left + colW > scroller.scrollLeft + viewW) scroller.scrollLeft = left + colW - viewW;
    } else {
      const top = panel.cursor * ROW;
      if (fresh) scroller.scrollTop = Math.max(0, top - viewH / 2);
      else if (top < scroller.scrollTop) scroller.scrollTop = top;
      else if (top + ROW > scroller.scrollTop + viewH) scroller.scrollTop = top + ROW - viewH;
    }
    scrollTop = scroller.scrollTop;
    scrollLeft = scroller.scrollLeft;
  });

  /** Brief view: the mouse wheel scrolls sideways, a column at a time (like TC). */
  function onWheel(e: WheelEvent) {
    if (!brief || e.ctrlKey || e.deltaX !== 0 || e.deltaY === 0) return;
    e.preventDefault();
    scroller.scrollLeft += e.deltaY > 0 ? colW : -colW;
  }

  // Free space of the volume, refreshed with every listing (cheap; off the UI thread).
  let space: { free: number; total: number } | null = $state(null);
  $effect(() => {
    const path = panel.path;
    if (!panel.listing || !path) return;
    let stale = false;
    api
      .diskSpace(path)
      .then((s) => !stale && (space = s))
      .catch(() => !stale && (space = null));
    return () => (stale = true);
  });

  /** Root of the current location (drive, UNC share or FTP server) and the top-level folder to focus there. */
  const rootTarget = $derived.by(() => {
    const p = panel.path;
    let m = /^(ftp:\/\/[^/]+\/)([^/]*)/i.exec(p);
    if (m) return { root: m[1], first: m[2] };
    m = /^(\\\\[^\\]+\\[^\\]+\\?)([^\\]*)/.exec(p);
    if (m) return { root: m[1], first: m[2] };
    m = /^([a-z]:\\?)([^\\]*)/i.exec(p);
    if (m) return { root: m[1].length === 2 ? m[1] + "\\" : m[1], first: m[2] };
    return null;
  });
  function goRoot() {
    if (rootTarget?.first) void panel.navigate(rootTarget.root, rootTarget.first);
  }
  const currentDrive = $derived(panel.path.length >= 2 && panel.path[1] === ":" ? panel.path[0].toUpperCase() : "");

  const cols: { key: SortKey; label: string }[] = $derived([
    { key: "name", label: t("Nazwa") },
    { key: "ext", label: t("Rozsz.") },
    { key: "size", label: t("Rozmiar") },
    { key: "date", label: t("Data") },
  ]);

  function rowClick(row: number, e: MouseEvent) {
    onfocus();
    if (e.ctrlKey) panel.toggleMark(row);
    panel.cursor = row;
  }

  // ---------- editable path bar ----------
  let pathInput = $state("");
  let pathError: string | null = $state(null);

  function startEdit() {
    onfocus();
    side.editingPath = true;
  }

  function initPathInput(el: HTMLInputElement) {
    pathInput = panel.isVirtual ? "" : panel.path;
    pathError = null;
    el.value = pathInput; // the binding updates later — select would hit an empty field
    el.focus();
    el.select();
  }

  async function pathKey(e: KeyboardEvent) {
    e.stopPropagation(); // keep panel shortcuts away from the text field
    if (e.key === "Escape") {
      e.preventDefault();
      side.editingPath = false;
    } else if (e.key === "Enter") {
      e.preventDefault();
      const target = panel;
      try {
        const r = await api.resolvePath(pathInput, target.isVirtual ? "C:\\" : target.path);
        await target.navigate(r.dir, r.focus ?? undefined);
        if (target.error) throw target.error;
        side.editingPath = false;
      } catch (err) {
        pathError = String(err);
      }
    } else if (/^F\d{1,2}$/.test(e.key) || (e.ctrlKey && "rpjsoghkdtwnu".includes(e.key.toLowerCase()))) {
      e.preventDefault(); // browser shortcuts
    } else pathError = null;
  }

  // ---------- quick filter ----------
  function focusFilter(el: HTMLInputElement) {
    el.focus();
    el.select();
  }

  function filterKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      panel.setFilter("");
      side.editingFilter = false;
    } else if (e.key === "Enter" || e.key === "ArrowDown" || e.key === "Tab") {
      e.preventDefault();
      side.editingFilter = false; // keep the filter, back to the list
    }
  }

  function mouseNav(e: MouseEvent) {
    if (e.button === 3) (e.preventDefault(), void panel.goBack());
    else if (e.button === 4) (e.preventDefault(), void panel.goForward());
  }

  // ---------- right-button drag: mark the rows swept over (TC) ----------
  /** Set when a right drag marked something: the context menu that follows is swallowed. */
  let rightDragged = false;

  /** Row under a viewport point (may lie outside the list: clamped). */
  function rowAt(x: number, y: number): number {
    const r = scroller.getBoundingClientRect();
    let row: number;
    if (brief) {
      const col = Math.floor((x - r.left + scroller.scrollLeft) / colW);
      const inCol = Math.max(0, Math.min(rpc - 1, Math.floor((y - r.top) / ROW)));
      row = col * rpc + inCol;
    } else row = Math.floor((y - r.top + scroller.scrollTop) / ROW);
    return Math.max(0, Math.min(rowCount - 1, row));
  }

  function rightDown(start: number, e: MouseEvent) {
    rightDragged = false;
    const p = panel;
    const base = p.marked;
    const i = p.entryAt(start);
    // Like TC: the sweep sets the opposite of the first entry's state.
    const on = i === PARENT || i === undefined ? true : !base.has(i);
    let last = start;
    let px = e.clientX,
      py = e.clientY;

    const apply = (row: number) => {
      if (row === last && !rightDragged) return;
      last = row;
      rightDragged = true;
      p.markRange(start, row, on, base);
      p.cursor = row;
    };
    const move = (m: MouseEvent) => {
      if (!(m.buttons & 2)) return cleanup();
      px = m.clientX;
      py = m.clientY;
      apply(rowAt(px, py));
    };
    // Pointer held above/below (left/right in brief) the list: keep scrolling it.
    const timer = setInterval(() => {
      const r = scroller.getBoundingClientRect();
      const out = brief ? (px < r.left ? -rpc : px > r.right ? rpc : 0) : py < r.top ? -1 : py > r.bottom ? 1 : 0;
      if (out) apply(Math.max(0, Math.min(rowCount - 1, last + out)));
    }, 40);
    const cleanup = () => {
      clearInterval(timer);
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", up);
    };
    const up = (m: MouseEvent) => {
      if (m.button === 2) cleanup();
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", up);
  }

  /** Right click: Explorer's context menu for the selection (or this entry). */
  function rowContext(row: number, e: MouseEvent) {
    e.preventDefault();
    if (rightDragged) return void (rightDragged = false);
    onfocus();
    const i = panel.entryAt(row);
    if (i === PARENT || i === undefined || !panel.listing) return;
    if (!panel.marked.has(i)) panel.cursor = row;
    oncontext(panel.marked.has(i) ? panel.selection().paths : [joinPath(panel.path, panel.listing.names[i])]);
  }
</script>

<section class="panel" class:active class:drop-here={dropHere} data-side={index} onmousedown={onfocus} onmouseup={mouseNav} role="presentation">
  <header class="drives">
    {#each drives as d (d.root)}
      <button
        class="drive"
        class:current={d.letter === currentDrive}
        title={`${d.root} (${d.kind})`}
        onclick={() => ondrive(d.root)}
        tabindex="-1">{d.letter}</button
      >
    {/each}
    <button class="drive ftp" class:current={panel.path.toLowerCase().startsWith("ftp://")} title={t("Połączenia FTP (Ctrl+F) · rozłącz: Ctrl+Shift+F")} onclick={onftp} tabindex="-1">FTP</button>
    {#if panel.path.toLowerCase().startsWith("ftp://")}
      <button class="drive disconnect" title={t("Rozłącz z serwerem (Ctrl+Shift+F)")} onclick={ondisconnect} tabindex="-1">
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 3.5 3.5 6a2.5 2.5 0 0 0 0 3.5l1 1M10 12.5 12.5 10a2.5 2.5 0 0 0 0-3.5l-1-1M5.5 5.5l5 5M2 14l2-2M12 4l2-2" /></svg>
        {t("Rozłącz")}
      </button>
    {/if}
    <span class="spacer"></span>
    {#if panel.loading}<span class="spinner" title={t("Wczytywanie…")}></span>{/if}
    <button class="drive root" title={t("Katalog główny dysku")} disabled={!rootTarget?.first} onclick={goRoot} tabindex="-1">\</button>
    <span class="view-mode">
      <button class="drive" class:current={!side.brief} title={t("Widok pełny (Ctrl+F1)")} onclick={() => (side.brief = false)} tabindex="-1">
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 4h12M2 8h12M2 12h12" /></svg>
      </button>
      <button class="drive" class:current={side.brief} title={t("Widok skrócony — same nazwy w kolumnach (Ctrl+F1)")} onclick={() => (side.brief = true)} tabindex="-1">
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 4h4.5M2 8h4.5M2 12h4.5M9.5 4H14M9.5 8H14M9.5 12H14" /></svg>
      </button>
    </span>
  </header>

  {#if side.tabs.length > 1}
    <div class="tabs">
      {#each side.tabs as tab, i (tab)}
        <div
          class="tab"
          class:current={i === side.index}
          title={tab.path}
          onclick={() => (side.index = i)}
          onauxclick={(e) => tabMouse(i, e)}
          onmousedown={(e) => e.button === 1 && e.preventDefault()}
          onkeydown={() => {}}
          role="tab"
          aria-selected={i === side.index}
          tabindex="-1"
        >
          <span class="tab-name">{tab.title}</span>
          <button class="tab-x" onclick={(e) => (e.stopPropagation(), side.closeTab(i))} tabindex="-1" title={t("Zamknij (Ctrl+W)")}>✕</button>
        </div>
      {/each}
    </div>
  {/if}

  <!-- rtl container = long paths are cut on the left; bdi keeps "C:\" in order -->
  {#if side.editingPath}
    <div class="path-edit">
      <input
        bind:value={pathInput}
        use:initPathInput
        onkeydown={pathKey}
        onblur={() => (side.editingPath = false)}
        class:bad={pathError}
        title={pathError ?? t("Enter — przejdź · Esc — anuluj · można wpisać ścieżkę do pliku, %ZMIENNE%, ścieżkę względną lub \\\\serwer\\udział")}
        spellcheck="false"
        autocomplete="off"
      />
      {#if pathError}<div class="path-error">{pathError}</div>{/if}
    </div>
  {:else}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div class="path" title={t("{0} — kliknij, aby wpisać ścieżkę (Ctrl+L)", panel.path)} onclick={startEdit} role="textbox" tabindex="-1">
      <bdi>{panel.path}</bdi>
    </div>
  {/if}

  {#if side.editingFilter || panel.filter}
    <div class="filter" class:editing={side.editingFilter}>
      <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2 3h12l-4.5 5.5V13l-3 1.5V8.5z" /></svg>
      {#if side.editingFilter}
        <input
          value={panel.filter}
          use:focusFilter
          oninput={(e) => panel.setFilter(e.currentTarget.value)}
          onkeydown={filterKey}
          onblur={() => (side.editingFilter = false)}
          placeholder={t("*.php;*.js albo fragment nazwy — Enter zatwierdź, Esc wyczyść")}
          spellcheck="false"
        />
      {:else}
        <button class="ftext" onclick={() => (side.editingFilter = true)} tabindex="-1" title={t("Zmień filtr (Ctrl+S)")}>{panel.filter}</button>
      {/if}
      <span class="fcount">{Math.max(0, panel.rowCount - (panel.hasParent ? 1 : 0))} / {panel.listing?.names.length ?? 0}</span>
      <button class="fx" onclick={() => (panel.setFilter(""), (side.editingFilter = false))} tabindex="-1" title={t("Wyłącz filtr")}>✕</button>
    </div>
  {/if}

  <div class="grid head">
    {#each cols as c (c.key)}
      <button class="col col-{c.key}" class:sorted={panel.sortKey === c.key} onclick={() => panel.setSort(c.key)} tabindex="-1">
        {c.label}{#if panel.sortKey === c.key}<span class="arrow">{panel.sortDesc ? "▼" : "▲"}</span>{/if}
      </button>
    {/each}
  </div>

  <div
    class="scroller"
    class:brief
    bind:this={scroller}
    bind:clientHeight={viewH}
    bind:clientWidth={viewW}
    onscroll={() => ((scrollTop = scroller.scrollTop), (scrollLeft = scroller.scrollLeft))}
    onwheel={onWheel}
  >
    <div class="spacer-rows" style:height="{brief ? rpc * ROW : rowCount * ROW}px" style:width={brief ? `${colCount * colW}px` : undefined}>
      {#if panel.listing}
        {@const l = panel.listing}
        {#each rows as row (row)}
          {@const i = panel.entryAt(row)}
          {@const dir = panel.isDir(i)}
          {@const [stem, ext] = i === PARENT || dir ? [i === PARENT ? ".." : l.names[i], ""] : splitExt(l.names[i])}
          <div
            class={brief ? "row brief-row" : "grid row"}
            class:cursor={row === panel.cursor}
            class:marked={i !== PARENT && panel.marked.has(i)}
            class:dir
            class:hidden-entry={i !== PARENT && (l.attrs[i] & 0x06) !== 0}
            class:drop-dir={dir && i !== PARENT && dropDir === joinPath(panel.path, l.names[i])}
            data-drop-dir={dir && i !== PARENT && !panel.isVirtual ? joinPath(panel.path, l.names[i]) : undefined}
            style:top="{brief ? (row % rpc) * ROW : row * ROW}px"
            style:left={brief ? `${Math.floor(row / rpc) * colW}px` : undefined}
            style:width={brief ? `${colW}px` : undefined}
            onmousedown={(e) => rowDown(row, e)}
            onclick={(e) => rowClick(row, e)}
            ondblclick={() => panel.activate()}
            oncontextmenu={(e) => rowContext(row, e)}
            onkeydown={() => {}}
            role="row"
            tabindex="-1"
          >
            <span class="col col-name">
              <span class="icon" aria-hidden="true">{#if i === PARENT}↰{:else if dir}{@html FOLDER_SVG}{:else}{@const own = hasOwnIcon(l.names[i]) ? shellIcon(joinPath(panel.path, l.names[i]), l.mtimes[i]) : null}{#if own}<img src={own} alt="" draggable="false" />{:else}{@html fileIconSvg(l.names[i])}{/if}{/if}</span>
              <span class="name">{brief ? (i === PARENT ? ".." : l.names[i]) : stem}</span>
            </span>
            {#if !brief}
              <span class="col col-ext">{ext}</span>
              <span class="col col-size"
                >{i === PARENT
                  ? ""
                  : dir
                    ? panel.sizing === l.names[i].toLowerCase()
                      ? "…"
                      : (panel.dirSize(i) !== undefined ? formatBytes(panel.dirSize(i)!) : "<DIR>")
                    : formatBytes(l.sizes[i])}</span
              >
              <span class="col col-date">{i === PARENT ? "" : formatDate(l.mtimes[i])}</span>
            {/if}
          </div>
        {/each}
      {/if}
    </div>
  </div>

  <footer class="status">
    {#if panel.error}
      <span class="error" title={panel.error}>⚠ {panel.error}</span>
    {:else}
      {@const s = panel.summary}
      <span>{t("{0} / {1} w {2} / {3} plikach, {4} / {5} katalogach", formatHuman(s.mBytes), formatHuman(s.bytes), s.mFiles, s.files, s.mDirs, s.dirs)}</span>
      {#if space}
        <span class="free" title={t("Wolne miejsce na dysku")}>{t("wolne {0} z {1}", formatHuman(space.free), formatHuman(space.total))}</span>
      {/if}
      <span class="timing" title={t("backend (Rust) · IPC+JSON · sortowanie (JS)")}>
        {panel.timing.backend.toFixed(0)} · {panel.timing.ipc.toFixed(0)} · {panel.timing.sort.toFixed(0)} ms
      </span>
    {/if}
  </footer>
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--panel-bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    overflow: hidden;
  }
  .panel.active {
    border-color: var(--accent);
  }
  .panel.drop-here {
    border-color: var(--accent);
    box-shadow: inset 0 0 0 2px var(--accent);
  }
  .row.drop-dir {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .drives {
    display: flex;
    gap: 2px;
    padding: 4px 6px;
    align-items: center;
    border-bottom: 1px solid var(--border);
  }
  .drive {
    min-width: 24px;
    padding: 1px 5px;
    font: inherit;
    font-size: 12px;
    color: var(--fg-dim);
    background: transparent;
    border: 1px solid transparent;
    border-radius: 4px;
    cursor: pointer;
  }
  .drive:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .drive.current {
    border-color: var(--border-strong);
    color: var(--fg);
    background: var(--hover);
  }
  .spacer {
    flex: 1;
  }
  .drive.disconnect {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
  }
  .drive.disconnect svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
  }
  .drive.disconnect:hover {
    color: var(--danger);
  }
  .drive.ftp {
    margin-left: 8px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.04em;
  }
  .drive.root {
    margin-left: 6px;
    font-weight: 700;
  }
  .drive.root:disabled {
    opacity: 0.35;
    background: transparent;
    color: var(--fg-dim);
    cursor: default;
  }
  .view-mode {
    display: flex;
    gap: 2px;
    margin-left: 10px;
  }
  .view-mode .drive {
    min-width: 0;
    padding: 1px 4px;
    display: grid;
    place-items: center;
  }
  .view-mode svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
  }
  .spinner {
    width: 12px;
    height: 12px;
    border: 2px solid var(--border-strong);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.7s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 4px 6px 0;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 4px;
    max-width: 180px;
    padding: 3px 4px 3px 10px;
    font-size: 12px;
    color: var(--fg-dim);
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: 6px 6px 0 0;
    cursor: default;
    flex: none;
  }
  .tab:hover {
    background: var(--hover);
  }
  .tab.current {
    color: var(--fg);
    background: var(--path-bg);
    border-color: var(--border);
  }
  .active .tab.current {
    background: var(--path-bg-active);
  }
  .tab-name {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .tab-x {
    font-size: 9px;
    width: 16px;
    height: 16px;
    padding: 0;
    border: 0;
    border-radius: 3px;
    background: transparent;
    color: inherit;
    opacity: 0.5;
    cursor: pointer;
  }
  .tab-x:hover {
    opacity: 1;
    background: var(--border);
  }
  .path {
    padding: 4px 8px;
    font-size: 12px;
    color: var(--fg-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    direction: rtl;
    text-align: left;
    background: var(--path-bg);
  }
  .active .path {
    color: var(--fg);
    background: var(--path-bg-active);
  }
  .path {
    cursor: text;
  }
  .path:hover {
    color: var(--fg);
  }
  .path-edit {
    position: relative;
    background: var(--path-bg-active);
  }
  .path-edit input {
    box-sizing: border-box;
    width: 100%;
    font: inherit;
    font-size: 12px;
    padding: 3px 7px;
    border: 1px solid var(--accent);
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  .path-edit input.bad {
    border-color: var(--danger);
  }
  .path-error {
    position: absolute;
    left: 0;
    right: 0;
    top: 100%;
    z-index: 5;
    padding: 4px 8px;
    font-size: 11px;
    color: #fff;
    background: var(--danger);
  }

  .filter {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 3px 8px;
    font-size: 12px;
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    border-bottom: 1px solid var(--border);
  }
  .filter svg {
    width: 13px;
    height: 13px;
    flex: none;
    fill: none;
    stroke: var(--accent);
    stroke-width: 1.4;
    stroke-linejoin: round;
  }
  .filter input {
    flex: 1;
    min-width: 0;
    font: inherit;
    padding: 2px 6px;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  .ftext {
    flex: 1;
    text-align: left;
    font: inherit;
    font-weight: 600;
    color: var(--fg);
    background: none;
    border: 0;
    padding: 0;
    cursor: text;
  }
  .fcount {
    color: var(--fg-dim);
    font-variant-numeric: tabular-nums;
  }
  .fx {
    border: 0;
    background: none;
    color: var(--fg-dim);
    cursor: pointer;
    font-size: 10px;
  }
  .fx:hover {
    color: var(--danger);
  }
  .grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 64px 104px 128px;
    align-items: center;
  }
  .head {
    border-bottom: 1px solid var(--border);
  }
  .head .col {
    font: inherit;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--fg-dim);
    background: transparent;
    border: 0;
    padding: 4px 8px;
    text-align: left;
    cursor: pointer;
  }
  .head .col:hover {
    color: var(--fg);
  }
  .head .col.sorted {
    color: var(--accent);
  }
  .arrow {
    margin-left: 4px;
    font-size: 9px;
  }
  .head .col-size,
  .row .col-size {
    text-align: right;
  }

  .scroller {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    position: relative;
    background: var(--list-bg);
    color: var(--list-fg);
  }
  .spacer-rows {
    position: relative;
  }
  /* Brief: columns side by side; the bar is always there so the row count per column is stable. */
  .scroller.brief {
    overflow-x: scroll;
    overflow-y: hidden;
  }
  .row.brief-row {
    right: auto;
    display: flex;
    align-items: center;
  }
  .brief-row .col-name {
    flex: 1;
    min-width: 0;
    box-sizing: border-box;
  }
  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 22px;
    font-size: 13px;
    cursor: default;
    user-select: none;
  }
  .row .col {
    padding: 0 8px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .col-name {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row .col-ext,
  .row .col-size,
  .row .col-date {
    color: var(--list-dim);
    font-variant-numeric: tabular-nums;
    font-size: 12px;
  }
  .icon {
    width: 16px;
    flex: none;
    display: inline-flex;
    justify-content: center;
    color: var(--list-dim);
  }
  .icon :global(svg),
  .icon img {
    width: 16px;
    height: 16px;
  }
  /* folders stand out by colour; marked ones take the mark colour from the row */
  .row.dir:not(.marked) .name {
    color: var(--list-dir);
    font-weight: 600;
  }
  .row.hidden-entry {
    opacity: 0.55;
  }
  .row:hover {
    background: var(--list-hover);
  }
  .row.marked {
    color: var(--mark);
  }
  .row.marked .col-ext,
  .row.marked .col-size,
  .row.marked .col-date {
    color: var(--mark);
  }
  .row.cursor {
    background: var(--cursor-inactive);
  }
  .active .row.cursor {
    background: var(--cursor);
    color: var(--cursor-fg);
  }
  .active .row.cursor .col-ext,
  .active .row.cursor .col-size,
  .active .row.cursor .col-date,
  .active .row.cursor .icon {
    color: var(--cursor-fg);
  }
  .active .row.cursor.marked {
    color: var(--mark-on-cursor);
  }

  .status {
    display: flex;
    gap: 12px;
    justify-content: space-between;
    padding: 4px 8px;
    font-size: 11px;
    color: var(--fg-dim);
    border-top: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
  }
  .error {
    color: var(--danger);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .free {
    margin-left: auto; /* right side, next to the timing */
  }
  .timing {
    font-variant-numeric: tabular-nums;
    opacity: 0.7;
  }
</style>
