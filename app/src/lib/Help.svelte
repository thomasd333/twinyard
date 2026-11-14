<script lang="ts">
  import Modal from "./Modal.svelte";
  import { t } from "./i18n.svelte";
  import { shortcuts } from "./shortcuts";

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  let q = $state("");
  const groups = $derived(
    shortcuts().map((g) => ({
      group: g.group,
      items: g.items.filter(([k, d]) => !q || (k + " " + d).toLowerCase().includes(q.toLowerCase())),
    })).filter((g) => g.items.length),
  );
</script>

<Modal title={t("Skróty klawiszowe")} wide {onclose} buttons={[{ label: t("Zamknij"), primary: true, run: onclose }]}>
  <input class="q" bind:value={q} placeholder={t("Filtruj… (np. kopiuj, F5, zakładka)")} spellcheck="false" />
  <div class="list">
    {#each groups as g (g.group)}
      <h3>{g.group}</h3>
      {#each g.items as [k, d] (k)}
        <div class="item"><kbd>{k}</kbd><span>{d}</span></div>
      {/each}
    {:else}
      <p class="none">{t("Nic nie pasuje.")}</p>
    {/each}
  </div>
</Modal>

<style>
  .q {
    width: 100%;
    box-sizing: border-box;
    font: inherit;
    padding: 6px 8px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    background: var(--bg);
    color: var(--fg);
    outline: none;
    margin-bottom: 8px;
  }
  .q:focus {
    border-color: var(--accent);
  }
  .list {
    max-height: 60vh;
    overflow: auto;
    padding-right: 4px;
  }
  h3 {
    margin: 12px 0 4px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--accent);
  }
  .item {
    display: grid;
    grid-template-columns: 220px 1fr;
    gap: 12px;
    padding: 3px 0;
    font-size: 12px;
    border-bottom: 1px solid var(--border);
  }
  kbd {
    font: inherit;
    font-weight: 600;
  }
  .none {
    color: var(--fg-dim);
  }
</style>
