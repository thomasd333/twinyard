<script lang="ts">
  import { convertFileSrc, invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { api, baseName, type TextFile } from "./api";
  import { splitLines } from "./diff";
  import { formatBytes } from "./format";
  import { t } from "./i18n.svelte";

  interface Props {
    path: string;
    onclose: () => void;
  }
  let { path, onclose }: Props = $props();

  const LIMIT = 8 * 1024 * 1024;
  const HEX_LIMIT = 256 * 1024;
  const ROW = 18;
  const IMAGE = /\.(png|jpe?g|gif|webp|bmp|ico|svg|avif)$/i;

  let file: TextFile | null = $state(null);
  let lines = $state.raw<string[]>([]);
  let hex = $state.raw<Uint8Array | null>(null);
  let image: string | null = $state(null);
  let mode = $state<"text" | "hex" | "image">("text");
  let wrap = $state(false);
  let error: string | null = $state(null);

  let scroller: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewH = $state(600);
  let card: HTMLDivElement;

  // find
  let findOpen = $state(false);
  let query = $state("");
  let hit = $state(-1);
  let findInput: HTMLInputElement | undefined = $state();

  onMount(async () => {
    card.focus();
    try {
      if (IMAGE.test(path)) {
        image = convertFileSrc(await invoke<string>("local_path", { path }));
        mode = "image";
        return;
      }
      file = await api.readText(path, LIMIT);
      if (file.binary) await showHex();
      else lines = splitLines(file.text);
    } catch (e) {
      error = String(e);
    }
  });

  async function showHex() {
    if (!hex) hex = new Uint8Array(await invoke<ArrayBuffer>("read_bytes", { path, limit: HEX_LIMIT }));
    mode = "hex";
  }

  const rowCount = $derived(mode === "hex" ? Math.ceil((hex?.length ?? 0) / 16) : lines.length);
  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 10));
  const last = $derived(Math.min(rowCount, Math.ceil((scrollTop + viewH) / ROW) + 10));
  const gutter = $derived(String(Math.max(1, lines.length)).length);

  function hexRow(i: number): [string, string, string] {
    const b = hex!.subarray(i * 16, i * 16 + 16);
    const h = [...b].map((x) => x.toString(16).padStart(2, "0")).join(" ");
    const a = [...b].map((x) => (x >= 0x20 && x < 0x7f ? String.fromCharCode(x) : "·")).join("");
    return [(i * 16).toString(16).padStart(8, "0"), h.padEnd(47), a];
  }

  function find(dir: 1 | -1) {
    if (!query || mode !== "text") return;
    const q = query.toLowerCase();
    const n = lines.length;
    for (let k = 1; k <= n; k++) {
      const i = (((hit < 0 ? (dir > 0 ? -1 : 0) : hit) + dir * k) % n + n) % n;
      if (lines[i].toLowerCase().includes(q)) {
        hit = i;
        if (scroller) scroller.scrollTop = Math.max(0, i * ROW - viewH / 3);
        return;
      }
    }
    hit = -1;
  }

  function highlight(line: string): string {
    const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;");
    if (!query) return esc(line);
    const q = query.toLowerCase();
    let out = "",
      pos = 0;
    const low = line.toLowerCase();
    for (let i = low.indexOf(q); i >= 0; i = low.indexOf(q, pos)) {
      out += esc(line.slice(pos, i)) + "<mark>" + esc(line.slice(i, i + q.length)) + "</mark>";
      pos = i + q.length;
    }
    return out + esc(line.slice(pos));
  }

  function onkeydown(e: KeyboardEvent) {
    const k = e.key;
    e.stopPropagation();
    if (e.target === findInput) {
      if (k === "Enter") (e.preventDefault(), find(e.shiftKey ? -1 : 1));
      else if (k === "Escape") (e.preventDefault(), (findOpen = false), card.focus());
      return;
    }
    let handled = true;
    if (k === "Escape" || k === "F3" || k.toLowerCase() === "q") onclose();
    else if ((e.ctrlKey && k.toLowerCase() === "f") || k === "F7" || k === "/") {
      findOpen = true;
      setTimeout(() => findInput?.select());
    } else if (k.toLowerCase() === "n" || (k === "F7" && e.shiftKey)) find(e.shiftKey ? -1 : 1);
    else if (k.toLowerCase() === "w") wrap = !wrap;
    else if (k.toLowerCase() === "h" && file) {
      if (mode === "hex" && !file.binary) mode = "text";
      else void showHex();
    } else if (scroller && k === "ArrowDown") scroller.scrollTop += ROW;
    else if (scroller && k === "ArrowUp") scroller.scrollTop -= ROW;
    else if (scroller && (k === "PageDown" || k === " ")) scroller.scrollTop += viewH - ROW;
    else if (scroller && k === "PageUp") scroller.scrollTop -= viewH - ROW;
    else if (scroller && k === "Home") scroller.scrollTop = 0;
    else if (scroller && k === "End") scroller.scrollTop = scroller.scrollHeight;
    else handled = false;
    if (handled || /^F\d/.test(k) || e.ctrlKey) e.preventDefault();
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="viewer" bind:this={card} tabindex="-1" {onkeydown} role="dialog" aria-label={t("Podgląd")}>
  <header>
    <b title={path}>{baseName(path)}</b>
    {#if file}<span>{formatBytes(file.size)} B · {mode === "hex" ? "hex" : file.encoding}{file.truncated ? " · " + t("pokazano pierwsze {0} B", formatBytes(mode === "hex" ? HEX_LIMIT : LIMIT)) : ""}</span>{/if}
    <span class="spacer"></span>
    <span class="keys">{t("Ctrl+F/F7 szukaj · N następny · W zawijanie · H hex · Esc zamknij")}</span>
    <button onclick={onclose} title={t("Zamknij (Esc)")}>✕</button>
  </header>
  {#if findOpen}
    <div class="find">
      <input bind:this={findInput} bind:value={query} placeholder={t("Szukaj…  (Enter następny, Shift+Enter poprzedni)")} spellcheck="false" />
      {#if query && hit < 0}<span class="nf">{t("nie znaleziono")}</span>{/if}
    </div>
  {/if}
  {#if error}
    <p class="msg err">{error}</p>
  {:else if mode === "image" && image}
    <div class="img"><img src={image} alt={baseName(path)} /></div>
  {:else if mode === "text" && wrap}
    <pre class="wrap">{#each lines as l, i (i)}<span class="ln">{String(i + 1).padStart(gutter)}</span>{@html highlight(l)}{"\n"}{/each}</pre>
  {:else}
    <div class="scroller" bind:this={scroller} bind:clientHeight={viewH} onscroll={() => (scrollTop = scroller!.scrollTop)}>
      <div style:height="{rowCount * ROW}px" class="space">
        {#each Array.from({ length: Math.max(0, last - first) }, (_, k) => first + k) as i (i)}
          <div class="row" class:hit={i === hit && mode === "text"} style:top="{i * ROW}px">
            {#if mode === "hex"}
              {@const [off, h, a] = hexRow(i)}
              <span class="ln">{off}</span><span>{h}</span><span class="ascii">{a}</span>
            {:else}
              <span class="ln">{String(i + 1).padStart(gutter)}</span><span>{@html highlight(lines[i])}</span>
            {/if}
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .viewer {
    position: fixed;
    inset: 34px 0 0 0; /* below the title bar: window stays movable/closable */
    z-index: 70;
    display: flex;
    flex-direction: column;
    background: var(--panel-bg);
    color: var(--fg);
    outline: none;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
  }
  header span {
    color: var(--fg-dim);
  }
  .spacer {
    flex: 1;
  }
  .keys {
    font-size: 11px;
  }
  header button {
    border: 0;
    background: transparent;
    color: var(--fg);
    cursor: pointer;
    font-size: 13px;
  }
  .find {
    display: flex;
    gap: 10px;
    align-items: center;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  .find input {
    flex: 1;
    font: inherit;
    padding: 4px 8px;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  .nf {
    color: var(--danger);
    font-size: 12px;
  }
  .scroller {
    flex: 1;
    overflow: auto;
    position: relative;
  }
  .space {
    position: relative;
    min-width: 100%;
  }
  .row,
  pre {
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 13px;
    white-space: pre;
  }
  .row {
    position: absolute;
    left: 0;
    height: 18px;
    line-height: 18px;
    padding-right: 20px;
  }
  .row.hit {
    background: var(--hover);
  }
  .ln {
    display: inline-block;
    color: var(--fg-dim);
    opacity: 0.6;
    padding: 0 12px 0 8px;
    user-select: none;
  }
  .ascii {
    margin-left: 16px;
    color: var(--fg-dim);
  }
  pre.wrap {
    flex: 1;
    overflow: auto;
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    line-height: 18px;
  }
  :global(.viewer mark) {
    background: #f5c518;
    color: #000;
  }
  .img {
    flex: 1;
    display: grid;
    place-items: center;
    overflow: auto;
    background: repeating-conic-gradient(var(--hover) 0 25%, transparent 0 50%) 0 0 / 16px 16px;
  }
  .img img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .msg {
    padding: 20px;
  }
  .err {
    color: var(--danger);
  }
</style>
