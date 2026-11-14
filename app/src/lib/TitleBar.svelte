<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import { t } from "./i18n.svelte";

  export interface ToolAction {
    /** Stable id (independent of the language) — hidden buttons are stored by it. */
    id: string;
    title: string;
    /** SVG path data, 16×16, stroked. */
    icon: string;
    run: () => void;
    /** Separator before this button. */
    sep?: boolean;
    active?: boolean;
    /** Icon colour (CSS), e.g. the folder colour for "compare directories". */
    color?: string;
  }

  interface Props {
    dark: boolean;
    ontoggletheme: () => void;
    onsettings: () => void;
    onhelp: () => void;
    subtitle: string;
    actions: ToolAction[];
  }
  let { dark, ontoggletheme, onsettings, onhelp, subtitle, actions }: Props = $props();

  const win = getCurrentWindow();
  let maximized = $state(false);

  onMount(() => {
    const sync = () => win.isMaximized().then((m) => (maximized = m)).catch(() => {});
    void sync();
    const un = win.onResized(sync);
    return () => void un.then((f) => f());
  });

  function drag(e: MouseEvent) {
    if (e.button === 0) void win.startDragging();
  }
</script>

<header class="titlebar" data-tauri-drag-region>
  <span class="brand" data-tauri-drag-region>
    <svg viewBox="0 0 16 16" aria-hidden="true" data-tauri-drag-region
      ><rect x="1.5" y="2.5" width="6" height="11" rx="1.5" class="l" /><rect x="8.5" y="2.5" width="6" height="11" rx="1.5" class="r" /></svg
    >
    twinyard
  </span>
  <span class="subtitle" data-tauri-drag-region title={subtitle}>{subtitle}</span>

  <div class="tools">
    {#each actions as a (a.id)}
      {#if a.sep}<span class="sep"></span>{/if}
      <button class="icon" class:on={a.active} onclick={a.run} title={a.title} tabindex="-1" style:color={a.color}>
        <svg viewBox="0 0 16 16"><path d={a.icon} /></svg>
      </button>
    {/each}
    <span class="sep"></span>
    <button class="icon" onclick={onhelp} title={t("Skróty klawiszowe (F1)")} tabindex="-1">
      <svg viewBox="0 0 16 16"><circle cx="8" cy="8" r="6.5" /><path d="M6.2 6.2a1.9 1.9 0 1 1 2.6 1.8c-.6.3-.8.7-.8 1.3v.4M8 11.6v.2" /></svg>
    </button>
    <button class="icon" onclick={onsettings} title={t("Konfiguracja")} tabindex="-1">
      <!-- sliders -->
      <svg viewBox="0 0 16 16"><path d="M2 4h3M8.5 4H14M2 8h7.5M13 8h1M2 12h1.5M7 12h7M5 2.5v3M11.5 6.5v3M5.5 10.5v3" /></svg>
    </button>
    <button class="icon" onclick={ontoggletheme} title={dark ? t("Jasny motyw") : t("Ciemny motyw")} tabindex="-1">
      {#if dark}
        <!-- moon -->
        <svg viewBox="0 0 16 16"><path d="M13.5 9.8A5.8 5.8 0 0 1 6.2 2.5a5.8 5.8 0 1 0 7.3 7.3z" /></svg>
      {:else}
        <!-- sun -->
        <svg viewBox="0 0 16 16"
          ><circle cx="8" cy="8" r="3" /><path
            d="M8 1v1.6M8 13.4V15M1 8h1.6M13.4 8H15M3 3l1.1 1.1M11.9 11.9 13 13M3 13l1.1-1.1M11.9 4.1 13 3"
          /></svg
        >
      {/if}
    </button>
    <button class="icon grip" onmousedown={drag} title={t("Przesuń okno")} tabindex="-1">
      <svg viewBox="0 0 16 16"
        ><path d="M8 1.5v13M1.5 8h13M8 1.5 6.3 3.2M8 1.5l1.7 1.7M8 14.5l-1.7-1.7M8 14.5l1.7-1.7M1.5 8l1.7-1.7M1.5 8l1.7 1.7M14.5 8l-1.7-1.7M14.5 8l-1.7 1.7" /></svg
      >
    </button>
  </div>

  <div class="win">
    <button onclick={() => win.minimize()} title={t("Minimalizuj")} tabindex="-1">
      <svg viewBox="0 0 10 10"><path d="M0 5h10" /></svg>
    </button>
    <button onclick={() => win.toggleMaximize()} title={maximized ? t("Przywróć") : t("Maksymalizuj")} tabindex="-1">
      {#if maximized}
        <svg viewBox="0 0 10 10"><path d="M2.5 2.5V.5h7v7h-2M.5 2.5h7v7h-7z" /></svg>
      {:else}
        <svg viewBox="0 0 10 10"><rect x=".5" y=".5" width="9" height="9" /></svg>
      {/if}
    </button>
    <button class="close" onclick={() => win.close()} title={t("Zamknij")} tabindex="-1">
      <svg viewBox="0 0 10 10"><path d="M0 0l10 10M10 0 0 10" /></svg>
    </button>
  </div>
</header>

<style>
  .titlebar {
    height: 34px;
    display: flex;
    align-items: center;
    gap: 10px;
    padding-left: 12px;
    user-select: none;
    background: var(--bg);
  }
  .brand {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    font-weight: 700;
    font-size: 12px;
    letter-spacing: 0.02em;
  }
  .brand svg {
    width: 15px;
    height: 15px;
  }
  .brand .l {
    fill: var(--accent);
  }
  .brand .r {
    fill: none;
    stroke: var(--fg-dim);
  }
  .subtitle {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    color: var(--fg-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    /* No flex here: a flex container's text can't shrink, so a long path
       pushed the window buttons off-screen. */
    line-height: 34px;
  }
  .tools {
    display: flex;
    gap: 2px;
    flex: none;
  }
  .icon {
    width: 30px;
    height: 26px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--fg-dim);
    cursor: pointer;
  }
  .icon:hover {
    background: var(--hover);
    color: var(--fg);
  }
  .icon svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .icon.on {
    color: var(--accent);
    background: var(--hover);
  }
  .tools {
    align-items: center;
  }
  .sep {
    width: 1px;
    height: 16px;
    margin: 0 4px;
    background: var(--border-strong);
  }
  .grip {
    cursor: move;
  }
  .win {
    display: flex;
    align-self: stretch;
    flex: none;
  }
  .win button {
    width: 46px;
    border: 0;
    background: transparent;
    color: var(--fg);
    display: grid;
    place-items: center;
    cursor: default;
  }
  .win button:hover {
    background: var(--hover);
  }
  .win button.close:hover {
    background: #c42b1c;
    color: #fff;
  }
  .win svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1;
  }
</style>
