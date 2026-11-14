<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { api, inArchive, isRemote } from "./api";
  import { formatBytes, formatDate } from "./format";
  import { t, tn } from "./i18n.svelte";
  import type { JobsState } from "./jobs.svelte";

  interface Meta {
    size: number;
    mtime: number;
  }
  interface Entry {
    rel: string;
    dir: boolean;
    left: Meta | null;
    right: Meta | null;
    state: "same" | "leftOnly" | "rightOnly" | "leftNewer" | "rightNewer" | "differ";
  }
  type Dir = "→" | "←" | "";

  interface Props {
    left: string;
    right: string;
    jobs: JobsState;
    onclose: () => void;
    /** After a sync: refresh the panels. */
    ondone: () => void;
  }
  let { left, right, jobs, onclose, ondone }: Props = $props();

  const ROW = 22;
  let entries = $state.raw<Entry[]>([]);
  let dirs = $state<Dir[]>([]);
  let subdirs = $state(true);
  let ignoreDates = $state(false);
  /** Equal-size files compared byte by byte (both sides plain local folders). */
  let byContent = $state(false);
  const contentPossible = $derived(![left, right].some((p) => isRemote(p) || inArchive(p)));
  let showSame = $state(false);
  let loading = $state(false);
  let syncing = $state(false);
  let error: string | null = $state(null);
  let info = $state("");
  let skipped = $state.raw<string[]>([]);
  let sel = $state(0);

  let scroller: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewH = $state(500);
  let card: HTMLDivElement;

  const defaultDir: Record<Entry["state"], Dir> = {
    same: "",
    leftOnly: "→",
    rightOnly: "←",
    leftNewer: "→",
    rightNewer: "←",
    differ: "",
  };

  async function run() {
    loading = true;
    error = null;
    try {
      const t0 = performance.now();
      const r = await invoke<{ entries: Entry[]; leftFiles: number; rightFiles: number; skipped: string[]; coarseDates: boolean }>("compare_dirs", {
        left,
        right,
        subdirs,
        ignoreDates,
        content: byContent && contentPossible,
      });
      entries = r.entries;
      // Server dates without MLSD (local time, no seconds): "newer" is a
      // guess — no direction proposed, the user picks (→ / ←).
      const guess = (e: Entry) => r.coarseDates && (e.state === "leftNewer" || e.state === "rightNewer");
      dirs = r.entries.map((e) => (guess(e) ? "" : defaultDir[e.state]));
      sel = 0;
      const diff = r.entries.filter((e) => e.state !== "same").length;
      info =
        `${diff ? `${diff} ${tn(diff, "różnica|różnice|różnic")}` : t("Katalogi zgodne")} · ${t("{0} / {1} plików", r.leftFiles, r.rightFiles)} · ${((performance.now() - t0) / 1000).toFixed(1)} s` +
        (r.skipped.length ? " · " + t("pominięto {0} katalogów bez dostępu", r.skipped.length) : "") +
        (r.coarseDates ? " · " + t("daty z serwera niedokładne — kierunek dla nowszych plików wybierz sam") : "");
      skipped = r.skipped;
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    card.focus();
    void run();
  });

  const visible = $derived(entries.map((_, i) => i).filter((i) => showSame || entries[i].state !== "same"));
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 8));
  const last = $derived(Math.min(visible.length, Math.ceil((scrollTop + viewH) / ROW) + 8));
  const counts = $derived({
    toRight: dirs.filter((d) => d === "→").length,
    toLeft: dirs.filter((d) => d === "←").length,
  });

  function joinRel(root: string, rel: string) {
    return isRemote(root) ? root.replace(/\/+$/, "") + "/" + rel : root.replace(/\\+$/, "") + "\\" + rel.replaceAll("/", "\\");
  }

  function setDir(row: number, d: Dir) {
    const i = visible[row];
    if (i === undefined) return;
    const e = entries[i];
    if (d === "→" && !e.left) return; // nothing to copy from that side
    if (d === "←" && !e.right) return;
    dirs[i] = d;
  }

  function cycle(row: number) {
    const i = visible[row];
    const order: Dir[] = ["→", "←", ""];
    let d = dirs[i];
    for (let k = 0; k < 3; k++) {
      d = order[(order.indexOf(d) + 1) % 3];
      const e = entries[i];
      if ((d === "→" && e.left) || (d === "←" && e.right) || d === "") break;
    }
    dirs[i] = d;
  }

  async function sync() {
    if (syncing) return;
    const groups = new Map<string, { dir: Dir; parent: string; rels: string[] }>();
    entries.forEach((e, i) => {
      const d = dirs[i];
      if (!d) return;
      const parent = e.rel.includes("/") ? e.rel.slice(0, e.rel.lastIndexOf("/")) : "";
      const key = d + "|" + parent;
      if (!groups.has(key)) groups.set(key, { dir: d, parent, rels: [] });
      groups.get(key)!.rels.push(e.rel);
    });
    if (!groups.size) return;
    syncing = true;
    const failed: string[] = [];
    try {
      for (const g of groups.values()) {
        const [src, dst] = g.dir === "→" ? [left, right] : [right, left];
        const target = g.parent ? joinRel(dst, g.parent) : dst;
        if (g.parent) {
          // Exists = fine. Any other failure: skip the group — a single file
          // copied to a missing folder would be saved under the folder's name.
          const err = await api.createDir(target).then(
            () => null,
            (e) => String(e),
          );
          if (err && !/już istnieje|already exists/i.test(err)) {
            failed.push(err);
            continue;
          }
        }
        const out = await jobs.run(
          { kind: "copy", sources: g.rels.map((r) => joinRel(src, r)), dest: target },
          t("Synchronizacja {0} {1}: {2} el.", g.dir, g.parent || t("(główny)"), g.rels.length),
        );
        if (out.cancelled) break;
      }
    } finally {
      syncing = false;
      ondone();
      await run();
      if (failed.length) error = failed.join("\n"); // after run(), which clears it
    }
  }

  function keepVisible() {
    if (!scroller) return;
    const top = sel * ROW;
    if (top < scroller.scrollTop) scroller.scrollTop = top;
    else if (top + ROW > scroller.scrollTop + viewH) scroller.scrollTop = top + ROW - viewH;
  }

  function onkeydown(e: KeyboardEvent) {
    const k = e.key;
    if ((e.target as HTMLElement).tagName === "INPUT") return;
    e.stopPropagation();
    let handled = true;
    if (k === "Escape") onclose();
    else if (k === "ArrowDown") sel = Math.min(visible.length - 1, sel + 1);
    else if (k === "ArrowUp") sel = Math.max(0, sel - 1);
    else if (k === "PageDown") sel = Math.min(visible.length - 1, sel + Math.floor(viewH / ROW));
    else if (k === "PageUp") sel = Math.max(0, sel - Math.floor(viewH / ROW));
    else if (k === "Home") sel = 0;
    else if (k === "End") sel = Math.max(0, visible.length - 1);
    else if (k === "ArrowRight") (setDir(sel, "→"), (sel = Math.min(visible.length - 1, sel + 1)));
    else if (k === "ArrowLeft") (setDir(sel, "←"), (sel = Math.min(visible.length - 1, sel + 1)));
    else if (k === " " || k === "Delete") (setDir(sel, ""), (sel = Math.min(visible.length - 1, sel + 1)));
    else if (k === "F5" || k === "F2" || k.toLowerCase() === "r") void run();
    else if (k.toLowerCase() === "s" && !e.ctrlKey) void sync();
    else handled = false;
    if (handled || /^F\d/.test(k) || e.ctrlKey) e.preventDefault();
    keepVisible();
  }

  const stateLabel: Record<Entry["state"], string> = $derived({
    same: "=",
    leftOnly: t("tylko lewy"),
    rightOnly: t("tylko prawy"),
    leftNewer: t("nowszy lewy"),
    rightNewer: t("nowszy prawy"),
    differ: t("różne"),
  });
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="sync" bind:this={card} tabindex="-1" {onkeydown} role="dialog" aria-label={t("Porównanie katalogów")}>
  <header>
    <b>{t("Porównanie katalogów")}</b>
    <label><input type="checkbox" bind:checked={subdirs} onchange={run} /> {t("podkatalogi")}</label>
    <label title={t("Pomocne przy FTP, gdzie daty po wysłaniu bywają czasem wysłania")}><input type="checkbox" bind:checked={ignoreDates} onchange={run} /> {t("ignoruj daty (tylko rozmiar)")}</label>
    <label title={contentPossible ? t("Pliki o tym samym rozmiarze porównywane bajt po bajcie (wolniej)") : t("Tylko dla katalogów na dysku (nie FTP / archiwum)")}
      ><input type="checkbox" bind:checked={byContent} onchange={run} disabled={!contentPossible} /> {t("po zawartości")}</label
    >
    <label><input type="checkbox" bind:checked={showSame} /> {t("pokaż identyczne")}</label>
    <span class="spacer"></span>
    <span class="info" title={skipped.join("\n")}>{loading ? t("Porównuję…") : info}</span>
    <button onclick={run} disabled={loading || syncing} title={t("Porównaj ponownie (F5 / R)")}>↻</button>
    <button onclick={onclose} title={t("Zamknij (Esc)")}>✕</button>
  </header>
  <div class="roots">
    <span title={left}>{left}</span>
    <span></span>
    <span title={right}>{right}</span>
  </div>
  {#if error}
    <p class="msg err">{error}</p>
  {:else if !loading && !visible.length}
    <p class="msg">{entries.length ? t("Wszystkie pliki są identyczne.") : t("Brak plików.")}</p>
  {:else}
    <div class="scroller" bind:this={scroller} bind:clientHeight={viewH} onscroll={() => (scrollTop = scroller!.scrollTop)}>
      <div style:height="{visible.length * ROW}px" class="space">
        {#each Array.from({ length: Math.max(0, last - first) }, (_, k) => first + k) as row (row)}
          {@const i = visible[row]}
          {@const e = entries[i]}
          <div class="row {e.state}" class:sel={row === sel} style:top="{row * ROW}px" onclick={() => (sel = row)} onkeydown={() => {}} role="row" tabindex="-1">
            <span class="name" class:dir={e.dir} title={e.rel}>{e.rel}{e.dir ? "\\" : ""}</span>
            <span class="meta l">{e.left ? (e.dir ? "<DIR>" : formatBytes(e.left.size)) + " · " + formatDate(e.left.mtime) : ""}</span>
            <button class="arrow" class:on={dirs[i]} onclick={() => ((sel = row), cycle(row))} title={t("Kierunek (→ ← spacja)")} tabindex="-1"
              >{dirs[i] || "·"}</button
            >
            <span class="meta r">{e.right ? (e.dir ? "<DIR>" : formatBytes(e.right.size)) + " · " + formatDate(e.right.mtime) : ""}</span>
            <span class="state">{stateLabel[e.state]}</span>
          </div>
        {/each}
      </div>
    </div>
  {/if}
  <footer>
    <span class="keys">{t("↑↓ wybór · → kopiuj w prawo · ← w lewo · spacja pomiń · klik w strzałkę zmienia · F5 porównaj ponownie · S synchronizuj")}</span>
    <span class="spacer"></span>
    <span>→ {counts.toRight} · ← {counts.toLeft}</span>
    <button class="primary" onclick={sync} disabled={syncing || !(counts.toRight + counts.toLeft)}>{syncing ? t("Synchronizuję…") : t("Synchronizuj (S)")}</button>
  </footer>
</div>

<style>
  .sync {
    position: fixed;
    inset: 34px 0 0 0;
    z-index: 70;
    display: flex;
    flex-direction: column;
    background: var(--panel-bg);
    color: var(--fg);
    outline: none;
  }
  header,
  footer {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 6px 10px;
    font-size: 12px;
  }
  header {
    border-bottom: 1px solid var(--border);
  }
  footer {
    border-top: 1px solid var(--border);
  }
  header label {
    display: flex;
    gap: 5px;
    align-items: center;
    color: var(--fg-dim);
  }
  .spacer {
    flex: 1;
  }
  .info,
  .keys {
    color: var(--fg-dim);
  }
  .keys {
    font-size: 11px;
  }
  button {
    font: inherit;
    border: 1px solid var(--border);
    border-radius: 4px;
    background: transparent;
    color: var(--fg);
    cursor: pointer;
    min-width: 26px;
    padding: 2px 8px;
  }
  button:disabled {
    opacity: 0.45;
    cursor: default;
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .roots {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 40px minmax(0, 1fr);
    padding: 4px 10px;
    font-size: 12px;
    font-weight: 600;
    border-bottom: 1px solid var(--border);
  }
  .roots span {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .scroller {
    flex: 1;
    overflow-y: auto;
  }
  .space {
    position: relative;
  }
  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 22px;
    display: grid;
    grid-template-columns: minmax(0, 2fr) minmax(0, 1fr) 40px minmax(0, 1fr) 90px;
    gap: 8px;
    align-items: center;
    padding: 0 10px;
    font-size: 12px;
    cursor: default;
  }
  .row span {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .row.sel {
    background: var(--hover);
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .name.dir {
    font-weight: 600;
  }
  .meta {
    color: var(--fg-dim);
    font-variant-numeric: tabular-nums;
  }
  .meta.l {
    text-align: right;
  }
  .state {
    font-size: 11px;
    color: var(--fg-dim);
  }
  .leftOnly .meta.l,
  .leftNewer .meta.l {
    color: #3fa35b;
  }
  .rightOnly .meta.r,
  .rightNewer .meta.r {
    color: #3fa35b;
  }
  .differ .state {
    color: var(--danger);
  }
  .arrow {
    border: 0;
    padding: 0;
    font-size: 15px;
    font-weight: 700;
    color: var(--fg-dim);
    background: none;
  }
  .arrow.on {
    color: var(--accent);
  }
  .msg {
    padding: 20px;
    color: var(--fg-dim);
  }
  .err {
    color: var(--danger);
  }
</style>
