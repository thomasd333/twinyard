<script lang="ts">
  import { t } from "./i18n.svelte";
  import Modal from "./Modal.svelte";

  interface Props {
    /** Most recent first. */
    list: string[];
    onopen: (path: string) => void;
    onclose: () => void;
  }
  let { list, onopen, onclose }: Props = $props();
  let sel = $state(0);

  function onkeydown(e: KeyboardEvent) {
    let handled = true;
    if (e.key === "ArrowDown") sel = Math.min(list.length - 1, sel + 1);
    else if (e.key === "ArrowUp") sel = Math.max(0, sel - 1);
    else if (e.key === "Enter" && list[sel]) onopen(list[sel]);
    else if (/^[1-9]$/.test(e.key) && list[+e.key - 1]) onopen(list[+e.key - 1]);
    else handled = false;
    if (handled) (e.preventDefault(), e.stopPropagation());
  }
</script>

<Modal title={t("Ostatnio odwiedzone katalogi")} {onclose} buttons={[{ label: t("Zamknij"), run: onclose }]}>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <ul data-autofocus tabindex="0" {onkeydown} role="listbox" aria-label={t("Historia katalogów")}>
    {#each list as p, i (p)}
      <li class:sel={i === sel} onclick={() => onopen(p)} onkeydown={() => {}} role="option" aria-selected={i === sel}>
        <span class="n">{i < 9 ? i + 1 : ""}</span>{p}
      </li>
    {:else}
      <li class="empty">{t("Historia jest pusta.")}</li>
    {/each}
  </ul>
</Modal>

<style>
  ul {
    list-style: none;
    margin: 0;
    padding: 4px;
    max-height: 50vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    outline: none;
  }
  ul:focus {
    border-color: var(--accent);
  }
  li {
    padding: 4px 6px;
    border-radius: 4px;
    font-size: 12px;
    cursor: default;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  li.sel {
    background: var(--cursor);
    color: var(--cursor-fg);
  }
  .n {
    display: inline-block;
    width: 18px;
    font-weight: 700;
    color: var(--accent);
  }
  li.sel .n {
    color: inherit;
  }
  .empty {
    color: var(--fg-dim);
  }
</style>
