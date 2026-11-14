<script lang="ts">
  import { baseName, isRemote, type Favorite } from "./api";
  import { t } from "./i18n.svelte";
  import Modal from "./Modal.svelte";

  interface Props {
    list: Favorite[];
    currentPath: string;
    onopen: (path: string, newTab: boolean) => void;
    onchange: (list: Favorite[]) => void;
    onclose: () => void;
  }
  let { list, currentPath, onopen, onchange, onclose }: Props = $props();

  let sel = $state(0);

  function add() {
    if (!currentPath || currentPath.startsWith("::")) return;
    if (list.some((f) => f.path.toLowerCase() === currentPath.toLowerCase())) return;
    onchange([...list, { name: baseName(currentPath) || currentPath, path: currentPath }]);
    sel = list.length; // new item (list updates after onchange)
  }

  // Manual add / edit (e.g. a network share that is not open anywhere).
  let form: { index: number | null; name: string; path: string } | null = $state(null);
  let formError: string | null = $state(null);
  let listEl: HTMLUListElement;

  function openForm(index: number | null) {
    const f = index !== null ? list[index] : null;
    form = { index, name: f?.name ?? "", path: f?.path ?? "" };
    formError = null;
  }

  function closeForm() {
    form = null;
    setTimeout(() => listEl?.focus());
  }

  function saveForm() {
    if (!form) return;
    let path = form.path.trim().replace(/^"|"$/g, "");
    if (!isRemote(path)) path = path.replace(/\//g, "\\");
    if (!path) return void (formError = t("Podaj ścieżkę"));
    const dup = list.findIndex((f) => f.path.toLowerCase() === path.toLowerCase());
    if (dup >= 0 && dup !== form.index) return void (formError = t("Już jest na liście: {0}", list[dup].name));
    const item = { name: form.name.trim() || baseName(path) || path, path };
    if (form.index === null) {
      onchange([...list, item]);
      sel = list.length;
    } else {
      onchange(list.map((f, k) => (k === form!.index ? item : f)));
    }
    closeForm();
  }

  function formKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      closeForm();
    }
  }

  function initField(el: HTMLInputElement) {
    el.focus();
    el.select();
  }

  function removeAt(i: number) {
    onchange(list.filter((_, k) => k !== i));
    sel = Math.max(0, Math.min(sel, list.length - 2));
  }

  function move(i: number, d: number) {
    const j = i + d;
    if (j < 0 || j >= list.length) return;
    const l = [...list];
    [l[i], l[j]] = [l[j], l[i]];
    onchange(l);
    sel = j;
  }

  function onkeydown(e: KeyboardEvent) {
    const k = e.key;
    let handled = true;
    if (k === "ArrowDown" && e.ctrlKey) move(sel, 1);
    else if (k === "ArrowUp" && e.ctrlKey) move(sel, -1);
    else if (k === "ArrowDown") sel = Math.min(list.length - 1, sel + 1);
    else if (k === "ArrowUp") sel = Math.max(0, sel - 1);
    else if (k === "Enter" && list[sel]) onopen(list[sel].path, e.ctrlKey);
    else if (k === "Delete" && list[sel]) removeAt(sel);
    else if (k === "Insert" || k.toLowerCase() === "a" || (e.ctrlKey && k.toLowerCase() === "d")) add();
    else if (k.toLowerCase() === "r") openForm(null);
    else if ((k === "F2" || k.toLowerCase() === "e") && list[sel]) openForm(sel);
    else if (/^[1-9]$/.test(k) && list[+k - 1]) onopen(list[+k - 1].path, false);
    else handled = false;
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  }
</script>

<Modal
  title={t("Ulubione katalogi")}
  {onclose}
  buttons={form
    ? [
        { label: form.index === null ? t("Dodaj") : t("Zapisz"), primary: true, run: saveForm },
        { label: t("Anuluj"), run: closeForm },
      ]
    : [
        // The letters (A, R, E) are the same in every language: see onkeydown.
        { label: t("Dod&aj bieżący"), run: add },
        { label: t("Dodaj &ręcznie"), run: () => openForm(null) },
        { label: t("&Edytuj"), run: () => list[sel] && openForm(sel) },
        { label: t("Zamknij"), run: onclose },
      ]}
>
  {#if form}
    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <div class="form" onkeydown={formKey} role="group">
      <label>
        <span>{t("Ścieżka (np. \\\\serwer\\udział\\katalog):")}</span>
        <input bind:value={form.path} use:initField spellcheck="false" autocomplete="off" />
      </label>
      <label>
        <span>{t("Nazwa (puste = nazwa katalogu):")}</span>
        <input bind:value={form.name} spellcheck="false" autocomplete="off" />
      </label>
      {#if formError}<p class="err">{formError}</p>{/if}
    </div>
  {/if}
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <ul class="favs" class:hidden={form} bind:this={listEl} data-autofocus tabindex="0" {onkeydown} role="listbox" aria-label={t("Ulubione")}>
    {#each list as f, i (f.path)}
      <li
        class:sel={i === sel}
        onclick={() => (sel = i)}
        ondblclick={() => onopen(f.path, false)}
        onkeydown={() => {}}
        role="option"
        aria-selected={i === sel}
        title={f.path}
      >
        <span class="num">{i < 9 ? i + 1 : ""}</span>
        <span class="name">{f.name}</span>
        <span class="path">{f.path}</span>
        <button class="del" onclick={(e) => (e.stopPropagation(), removeAt(i))} tabindex="-1" title={t("Usuń z ulubionych")}>✕</button>
      </li>
    {:else}
      <li class="empty">{t("Brak ulubionych. Naciśnij A, aby dodać bieżący katalog.")}</li>
    {/each}
  </ul>
  {#if !form}
    <p class="hint">{t("Enter otwórz · Ctrl+Enter w nowej zakładce · 1–9 szybki wybór · E/F2 edytuj · Del usuń · Ctrl+↑↓ kolejność")}</p>
  {/if}
</Modal>

<style>
  .favs {
    list-style: none;
    margin: 0;
    padding: 4px;
    max-height: 50vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    outline: none;
  }
  .favs.hidden {
    display: none;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .form label {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .form span {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .form input {
    font: inherit;
    padding: 6px 8px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  .form input:focus {
    border-color: var(--accent);
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: 12px;
  }
  .favs:focus {
    border-color: var(--accent);
  }
  li {
    display: grid;
    grid-template-columns: 18px minmax(80px, auto) 1fr 20px;
    gap: 10px;
    align-items: center;
    padding: 4px 6px;
    border-radius: 4px;
    cursor: default;
  }
  li.sel {
    background: var(--cursor);
    color: var(--cursor-fg);
  }
  .num {
    color: var(--accent);
    font-weight: 700;
    font-size: 12px;
  }
  li.sel .num,
  li.sel .path {
    color: inherit;
  }
  .name {
    font-weight: 600;
  }
  .path {
    color: var(--fg-dim);
    font-size: 12px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .del {
    border: 0;
    background: transparent;
    color: inherit;
    opacity: 0.5;
    cursor: pointer;
    font-size: 10px;
  }
  .del:hover {
    opacity: 1;
  }
  .empty {
    display: block;
    color: var(--fg-dim);
    padding: 10px;
  }
  .hint {
    margin: 8px 0 0;
    font-size: 11px;
    color: var(--fg-dim);
  }
</style>
