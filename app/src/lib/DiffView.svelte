<script lang="ts">
  import { onMount } from "svelte";
  import { api, baseName } from "./api";
  import { diffRows, hunks, type Row } from "./diff";
  import { t, tn } from "./i18n.svelte";

  interface Props {
    left: string;
    right: string;
    onclose: () => void;
  }
  let { left, right, onclose }: Props = $props();

  const LIMIT = 16 * 1024 * 1024;
  const ROW = 18;

  let rows: Row[] = $state.raw([]);
  let starts: number[] = $state.raw([]);
  let info = $state("");
  let error: string | null = $state(null);
  let current = $state(-1);
  let loading = $state(true);

  let scroller: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewH = $state(600);
  let card: HTMLDivElement;

  onMount(async () => {
    card.focus();
    try {
      const [a, b] = await Promise.all([api.readText(left, LIMIT), api.readText(right, LIMIT)]);
      if (a.binary || b.binary) {
        const same = a.size === b.size;
        error = t("Pliki binarne — porównanie tekstowe niemożliwe ({0}). Użyj zewnętrznego narzędzia (config.toml → diff).", same ? t("ten sam rozmiar") : t("różne rozmiary"));
        return;
      }
      const t0 = performance.now();
      rows = diffRows(a.text, b.text);
      starts = hunks(rows);
      info =
        (starts.length ? `${starts.length} ${tn(starts.length, "różnica|różnice|różnic")}` : t("Pliki identyczne")) +
        ` · ${a.encoding} / ${b.encoding}` +
        (a.truncated || b.truncated ? " · " + t("porównano tylko początek (16 MB)") : "") +
        ` · ${(performance.now() - t0).toFixed(0)} ms`;
      if (starts.length) setTimeout(() => go(0));
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  });

  const first = $derived(Math.max(0, Math.floor(scrollTop / ROW) - 10));
  const last = $derived(Math.min(rows.length, Math.ceil((scrollTop + viewH) / ROW) + 10));

  function go(k: number) {
    if (!starts.length || !scroller) return;
    current = (k + starts.length) % starts.length;
    scroller.scrollTop = Math.max(0, starts[current] * ROW - viewH / 3);
  }

  function onkeydown(e: KeyboardEvent) {
    const k = e.key;
    e.stopPropagation();
    let handled = true;
    if (k === "Escape" || k.toLowerCase() === "q") onclose();
    else if (k === "F7" || k.toLowerCase() === "n" || (e.altKey && k === "ArrowDown")) go(e.shiftKey ? current - 1 : current + 1);
    else if (k.toLowerCase() === "p" || (e.altKey && k === "ArrowUp")) go(current - 1);
    else if (scroller && k === "ArrowDown") scroller.scrollTop += ROW;
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
<div class="diff" bind:this={card} tabindex="-1" {onkeydown} role="dialog" aria-label={t("Porównanie plików")}>
  <header>
    <b>{t("Porównanie plików")}</b>
    <span>{loading ? t("Wczytywanie…") : info}</span>
    <span class="spacer"></span>
    {#if starts.length}<span class="pos">{current + 1} / {starts.length}</span>{/if}
    <button onclick={() => go(current - 1)} title={t("Poprzednia różnica (P, Alt+↑)")} disabled={!starts.length}>↑</button>
    <button onclick={() => go(current + 1)} title={t("Następna różnica (N, F7, Alt+↓)")} disabled={!starts.length}>↓</button>
    <button onclick={onclose} title={t("Zamknij (Esc)")}>✕</button>
  </header>
  <div class="names">
    <span title={left}>{baseName(left)} <small>{left}</small></span>
    <span title={right}>{baseName(right)} <small>{right}</small></span>
  </div>
  {#if error}
    <p class="msg">{error}</p>
  {:else}
    <div class="scroller" bind:this={scroller} bind:clientHeight={viewH} onscroll={() => (scrollTop = scroller!.scrollTop)}>
      <div style:height="{rows.length * ROW}px" class="space">
        {#each Array.from({ length: Math.max(0, last - first) }, (_, k) => first + k) as i (i)}
          {@const r = rows[i]}
          <div class="row {r.kind}" class:cur={current >= 0 && i === starts[current]} style:top="{i * ROW}px">
            <span class="ln">{r.ln ?? ""}</span><span class="l">{r.l}</span>
            <span class="ln">{r.rn ?? ""}</span><span class="r">{r.r}</span>
          </div>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .diff {
    position: fixed;
    inset: 34px 0 0 0; /* below the title bar: window stays movable/closable */
    z-index: 70;
    display: flex;
    flex-direction: column;
    background: var(--panel-bg);
    color: var(--fg);
    outline: none;
    --del: rgb(220 60 60 / 0.18);
    --add: rgb(40 170 80 / 0.18);
    --chg: rgb(230 170 20 / 0.18);
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
  header button {
    border: 1px solid var(--border);
    border-radius: 4px;
    background: transparent;
    color: var(--fg);
    cursor: pointer;
    min-width: 26px;
  }
  header button:disabled {
    opacity: 0.4;
  }
  .names {
    display: grid;
    grid-template-columns: 1fr 1fr;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
  }
  .names span {
    padding: 4px 10px;
    font-weight: 600;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .names small {
    font-weight: 400;
    color: var(--fg-dim);
    margin-left: 6px;
  }
  .scroller {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .space {
    position: relative;
  }
  .row {
    position: absolute;
    left: 0;
    right: 0;
    height: 18px;
    line-height: 18px;
    display: grid;
    grid-template-columns: 52px minmax(0, 1fr) 52px minmax(0, 1fr);
    font-family: "Cascadia Mono", Consolas, monospace;
    font-size: 13px;
    white-space: pre;
  }
  .row span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ln {
    text-align: right;
    padding-right: 8px;
    color: var(--fg-dim);
    opacity: 0.6;
    user-select: none;
  }
  .l {
    border-right: 1px solid var(--border);
  }
  .del .l,
  .del .ln:first-child {
    background: var(--del);
  }
  .add .r,
  .add .ln:nth-child(3) {
    background: var(--add);
  }
  .chg .l,
  .chg .r {
    background: var(--chg);
  }
  .row.cur {
    box-shadow: inset 3px 0 0 var(--accent);
  }
  .msg {
    padding: 20px;
    color: var(--fg-dim);
  }
</style>
