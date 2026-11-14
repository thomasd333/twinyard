<script lang="ts" module>
  export interface ModalButton {
    /**
     * `&` marks the single-letter shortcut ("&Nadpisz", "S&kip all"); it
     * works when focus is not in a text field. Part of the (translated)
     * label, so every language picks its own letters.
     */
    label: string;
    primary?: boolean;
    danger?: boolean;
    run: () => void;
  }
</script>

<script lang="ts">
  import { onMount, type Snippet } from "svelte";

  interface Props {
    title: string;
    buttons: ModalButton[];
    onclose: () => void;
    children: Snippet;
    wide?: boolean;
    /**
     * For questions that pop up by themselves (a background job): keys and
     * clicks are ignored for this many ms, and held-down (repeating) keys
     * always — Enter pressed for the panel must not answer "Overwrite".
     */
    armDelay?: number;
  }
  let { title, buttons, onclose, children, wide = false, armDelay = 0 }: Props = $props();

  let card: HTMLDivElement;
  const shownAt = performance.now();
  const armed = (e: Event) => !armDelay || (performance.now() - shownAt >= armDelay && !(e instanceof KeyboardEvent && e.repeat));

  onMount(() => {
    // Focus the first input if any, else the card itself (for hotkeys).
    // An explicitly marked field wins over the first input in document order.
    const input = card.querySelector<HTMLElement>("[data-autofocus]") ?? card.querySelector<HTMLElement>("input, textarea");
    (input ?? card).focus();
  });

  function onkeydown(e: KeyboardEvent) {
    e.stopPropagation();
    if (/^F\d{1,2}$/.test(e.key)) e.preventDefault();
    if (e.key !== "Tab" && !armed(e)) {
      e.preventDefault();
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      onclose();
    } else if (e.key === "Enter") {
      e.preventDefault();
      buttons.find((b) => b.primary)?.run();
    } else if (e.key === "Tab") {
      // Keep focus inside the dialog.
      const els = [...card.querySelectorAll<HTMLElement>("input, textarea, select, button, [data-autofocus]")].filter(
        (el) => el.offsetParent !== null, // visible only
      );
      if (!els.length) return;
      const i = els.indexOf(document.activeElement as HTMLElement);
      const next = els[(i + (e.shiftKey ? -1 : 1) + els.length) % els.length];
      e.preventDefault();
      next.focus();
    } else if (!(e.target instanceof HTMLInputElement) && e.key.length === 1 && !e.ctrlKey) {
      const b = buttons.find((b) => labelParts(b)[1].toLowerCase() === e.key.toLowerCase());
      if (b) {
        e.preventDefault();
        b.run();
      }
    }
  }

  function labelParts(b: ModalButton): [string, string, string] {
    const i = b.label.indexOf("&");
    if (i < 0 || i === b.label.length - 1) return [b.label, "", ""];
    return [b.label.slice(0, i), b.label[i + 1], b.label.slice(i + 2)];
  }
</script>

<div class="overlay" role="presentation" onmousedown={(e) => e.target === e.currentTarget && onclose()}>
  <div class="card" class:wide bind:this={card} role="dialog" aria-modal="true" aria-label={title} tabindex="-1" {onkeydown}>
    <h2>{title}</h2>
    <div class="body">{@render children()}</div>
    <div class="buttons">
      {#each buttons as b (b.label)}
        {@const [pre, key, post] = labelParts(b)}
        <button class:primary={b.primary} class:danger={b.danger} onclick={(e) => armed(e) && b.run()}
          >{pre}{#if key}<u>{key}</u>{/if}{post}</button
        >
      {/each}
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    background: rgb(0 0 0 / 0.35);
    display: grid;
    place-items: center;
    z-index: 90; /* dialogs above everything, incl. job questions during sync */
    animation: fade 0.12s ease-out;
  }
  @keyframes fade {
    from {
      opacity: 0;
    }
  }
  .card {
    width: min(520px, calc(100vw - 32px));
    background: var(--panel-bg);
    color: var(--fg);
    border: 1px solid var(--border-strong);
    border-radius: 10px;
    box-shadow: 0 16px 48px rgb(0 0 0 / 0.35);
    padding: 16px 18px 14px;
    outline: none;
  }
  .card.wide {
    width: min(680px, calc(100vw - 32px));
  }
  h2 {
    margin: 0 0 12px;
    font-size: 14px;
    font-weight: 600;
  }
  .body {
    font-size: 13px;
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: 6px;
    margin-top: 16px;
  }
  button {
    font: inherit;
    font-size: 12px;
    padding: 5px 12px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    background: var(--hover);
    color: var(--fg);
    cursor: pointer;
  }
  button:hover {
    border-color: var(--accent);
  }
  button.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  button.danger.primary {
    background: var(--danger);
    border-color: var(--danger);
  }
  u {
    text-underline-offset: 2px;
  }
</style>
