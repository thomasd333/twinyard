<script lang="ts" module>
  import type { SearchHit } from "./api";

  // Last search survives closing the window (Alt+F7 again shows it).
  const saved = {
    root: "",
    mask: "",
    text: "",
    subdirs: true,
    caseSensitive: false,
    skipNoise: true,
    hits: [] as SearchHit[],
    summary: "",
  };
</script>

<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onDestroy, onMount } from "svelte";
  import { api, baseName, type SearchBatch } from "./api";
  import { formatBytes, formatDate } from "./format";
  import { t } from "./i18n.svelte";
  import Modal from "./Modal.svelte";

  interface Props {
    root: string;
    onopen: (path: string) => void;
    onview: (path: string) => void;
    onedit: (path: string) => void;
    onclose: () => void;
  }
  let { root: initialRoot, onopen, onview, onedit, onclose }: Props = $props();

  const MAX_HITS = 20000;
  const ROW = 22;

  // Initial value only: the window is re-created each time it opens.
  // svelte-ignore state_referenced_locally
  let root = $state(saved.root && saved.hits.length ? saved.root : initialRoot);
  let mask = $state(saved.mask);
  let text = $state(saved.text);
  let subdirs = $state(saved.subdirs);
  let caseSensitive = $state(saved.caseSensitive);
  let skipNoise = $state(saved.skipNoise);
  let hits = $state.raw<SearchHit[]>(saved.hits);
  let summary = $state(saved.summary);
  let running: number | null = $state(null);
  let current = $state("");
  let error: string | null = $state(null);
  let sel = $state(0);

  let listEl: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewH = $state(300);
  let unlisten: (() => void) | null = null;
  /** Batches that arrive before searchStart() returned the id (fast searches). */
  const early = new Map<number, SearchBatch[]>();

  onMount(async () => {
    unlisten = await listen<SearchBatch>("search-batch", (e) => {
      const b = e.payload;
      if (running === null || b.id > running) {
        early.set(b.id, [...(early.get(b.id) ?? []), b]);
        return;
      }
      apply(b);
    });
  });

  function apply(b: SearchBatch) {
    {
      if (b.id !== running) return;
      if (b.hits.length && hits.length < MAX_HITS) hits = hits.concat(b.hits).slice(0, MAX_HITS);
      current = b.stats.current;
      summary = t("{0} wyników · {1} plików w {2} katalogach", b.stats.hits, b.stats.files, b.stats.dirs) + (text ? " · " + t("przeczytano {0}", b.stats.read) : "");
      if (b.done) {
        running = null;
        if (/^(Błąd|Error): /.test(b.stats.current)) error = b.stats.current; // backend's failure note
        current = "";
        if (hits.length >= MAX_HITS) summary += " · " + t("pokazano pierwsze {0}", MAX_HITS);
        persist();
      }
    }
  }

  onDestroy(() => {
    unlisten?.();
    if (running) void api.searchCancel(running);
    persist();
  });

  function persist() {
    Object.assign(saved, { root, mask, text, subdirs, caseSensitive, skipNoise, hits, summary });
  }

  async function start() {
    if (running) await api.searchCancel(running);
    error = null;
    hits = [];
    sel = 0;
    summary = t("Szukam…");
    try {
      const id = await api.searchStart({ root: root.trim(), mask: mask.trim(), text, subdirs, caseSensitive, skipNoise });
      running = id;
      for (const b of early.get(id) ?? []) apply(b);
      early.clear();
    } catch (e) {
      error = String(e);
      summary = "";
    }
  }

  function stop() {
    if (running) void api.searchCancel(running);
  }

  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 5));
  const last = $derived(Math.min(hits.length, Math.ceil((scrollTop + viewH) / ROW) + 5));

  function keepVisible() {
    if (!listEl) return;
    const top = sel * ROW;
    if (top < listEl.scrollTop) listEl.scrollTop = top;
    else if (top + ROW > listEl.scrollTop + viewH) listEl.scrollTop = top + ROW - viewH;
  }

  function listKey(e: KeyboardEvent) {
    const h = hits[sel];
    let handled = true;
    if (e.key === "ArrowDown") sel = Math.min(hits.length - 1, sel + 1);
    else if (e.key === "ArrowUp") sel = Math.max(0, sel - 1);
    else if (e.key === "PageDown") sel = Math.min(hits.length - 1, sel + Math.floor(viewH / ROW));
    else if (e.key === "PageUp") sel = Math.max(0, sel - Math.floor(viewH / ROW));
    else if (e.key === "Home") sel = 0;
    else if (e.key === "End") sel = Math.max(0, hits.length - 1);
    else if (e.key === "Enter" && h) onopen(h.path);
    else if (e.key === "F3" && h && !h.dir) onview(h.path);
    else if (e.key === "F4" && h && !h.dir) onedit(h.path);
    else handled = false;
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
      keepVisible();
    }
  }

  function formKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      e.stopPropagation();
      void start();
    } else if (e.key === "ArrowDown" && hits.length) {
      e.preventDefault();
      e.stopPropagation();
      listEl?.focus();
    }
  }

  function dirOf(p: string) {
    const i = Math.max(p.lastIndexOf("\\"), p.lastIndexOf("/"));
    return i > 0 ? p.slice(0, i) : p;
  }

  function rel(p: string) {
    const d = dirOf(p);
    const r = root.replace(/[\\/]+$/, "");
    return d.toLowerCase().startsWith(r.toLowerCase()) ? d.slice(r.length).replace(/^[\\/]/, "") || "." : d;
  }
</script>

<Modal
  title={t("Szukaj plików")}
  wide
  {onclose}
  buttons={[
    running ? { label: t("Zatrzymaj"), run: stop } : { label: t("Szukaj"), primary: true, run: start },
    { label: t("Zamknij"), run: onclose },
  ]}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="form" onkeydown={formKey}>
    <label class="wide"><span>{t("Szukaj w")}</span><input bind:value={root} spellcheck="false" /></label>
    <label
      ><span>{t("Nazwa (np. *.php;*.js albo fragment; puste = wszystko)")}</span><input
        bind:value={mask}
        data-autofocus
        spellcheck="false"
        placeholder="*"
      /></label
    >
    <label><span>{t("Tekst w pliku (opcjonalnie)")}</span><input bind:value={text} spellcheck="false" placeholder={t("np. function zapisz")} /></label>
    <div class="checks wide">
      <label><input type="checkbox" bind:checked={subdirs} /> {t("Podkatalogi")}</label>
      <label><input type="checkbox" bind:checked={caseSensitive} /> {t("Wielkość liter")}</label>
      <label title=".git, node_modules, __pycache__, target, .venv, .cache"><input type="checkbox" bind:checked={skipNoise} /> {t("Pomijaj .git, node_modules itp.")}</label>
      <span class="hint">{t("Tekst szukany tylko w plikach tekstowych (lista w config.toml → [search])")}</span>
    </div>
  </div>

  <div class="status">
    {#if running}<span class="spin"></span>{/if}
    <span>{summary}</span>
    {#if current}<span class="cur" title={current}>{current}</span>{/if}
  </div>
  {#if error}<p class="err">{error}</p>{/if}

  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <div class="results" bind:this={listEl} bind:clientHeight={viewH} onscroll={() => (scrollTop = listEl!.scrollTop)} tabindex="0" onkeydown={listKey} role="listbox" aria-label={t("Wyniki")}>
    <div style:height="{hits.length * ROW}px" class="space">
      {#each Array.from({ length: Math.max(0, last - first) }, (_, k) => first + k) as i (i)}
        {@const h = hits[i]}
        <div
          class="hit"
          class:sel={i === sel}
          style:top="{i * ROW}px"
          onclick={() => (sel = i)}
          ondblclick={() => onopen(h.path)}
          onkeydown={() => {}}
          role="option"
          tabindex="-1"
          aria-selected={i === sel}
          title={h.path}
        >
          <span class="name" class:dir={h.dir}>{baseName(h.path)}</span>
          <span class="where">{rel(h.path)}</span>
          {#if h.line}<span class="snip"><b>{h.line}:</b> {h.snippet}</span>{:else}<span class="meta">{h.dir ? "<DIR>" : formatBytes(h.size)} · {formatDate(h.mtime)}</span>{/if}
        </div>
      {/each}
    </div>
  </div>
  <p class="keys">{t("Enter — przejdź do pliku w panelu · F3 podgląd · F4 edycja · ↓ z pól przechodzi do wyników")}</p>
</Modal>

<style>
  .form {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 12px;
  }
  .wide {
    grid-column: 1 / -1;
  }
  .form label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .form label > span {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .form input:not([type="checkbox"]) {
    font: inherit;
    padding: 5px 8px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  .form input:focus {
    border-color: var(--accent);
  }
  .checks {
    display: flex;
    flex-wrap: wrap;
    gap: 6px 16px;
    align-items: center;
    font-size: 12px;
  }
  .checks label {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  .hint {
    color: var(--fg-dim);
    font-size: 11px;
  }
  .status {
    display: flex;
    gap: 10px;
    align-items: center;
    margin: 10px 0 6px;
    font-size: 12px;
    min-height: 18px;
  }
  .cur {
    color: var(--fg-dim);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    direction: rtl;
    flex: 1;
  }
  .spin {
    width: 10px;
    height: 10px;
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
  .results {
    height: min(46vh, 420px);
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    outline: none;
  }
  .results:focus {
    border-color: var(--accent);
  }
  .space {
    position: relative;
  }
  .hit {
    position: absolute;
    left: 0;
    right: 0;
    height: 22px;
    display: grid;
    grid-template-columns: minmax(120px, 1.2fr) minmax(80px, 1fr) minmax(120px, 2fr);
    gap: 10px;
    align-items: center;
    padding: 0 8px;
    font-size: 12px;
    cursor: default;
  }
  .hit span {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .hit.sel {
    background: var(--cursor);
    color: var(--cursor-fg);
  }
  .name {
    font-weight: 600;
  }
  .where,
  .meta {
    color: var(--fg-dim);
  }
  .hit.sel .where,
  .hit.sel .meta {
    color: inherit;
  }
  .snip {
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 11.5px;
  }
  .err {
    color: var(--danger);
    font-size: 12px;
  }
  .keys {
    margin: 6px 0 0;
    font-size: 11px;
    color: var(--fg-dim);
  }
</style>
