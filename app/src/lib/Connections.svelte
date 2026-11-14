<script lang="ts">
  import { onMount } from "svelte";
  import { ftp, type Connection, type TlsMode } from "./api";
  import { t } from "./i18n.svelte";
  import Modal from "./Modal.svelte";

  interface Props {
    /** Connection to preselect (e.g. the one of the active panel). */
    current: string | null;
    onconnect: (id: string) => void;
    onclose: () => void;
  }
  let { current, onconnect, onclose }: Props = $props();

  let list: Connection[] = $state([]);
  let sel = $state(0);
  let mode = $state<"list" | "edit" | "import">("list");
  let error: string | null = $state(null);
  let info: string | null = $state(null);
  let confirmDel = $state(false);
  let listEl: HTMLUListElement | undefined = $state();

  // edit form
  let form: Connection = $state(blank());
  let password = $state("");
  let importPath = $state("");

  function blank(): Connection {
    return { id: "", name: "", host: "", port: 21, tls: "explicit", user: "", initialDir: "", passive: true, mlsd: true, keepalive: 0, encoding: "auto" };
  }

  async function load(selectId?: string | null) {
    try {
      list = await ftp.connections();
      const i = selectId ? list.findIndex((c) => c.id === selectId) : -1;
      sel = i >= 0 ? i : Math.min(sel, Math.max(0, list.length - 1));
    } catch (e) {
      error = String(e);
    }
  }

  onMount(async () => {
    await load(current);
    focusList(); // the list renders after the async load
    if (!list.length) info = t("Brak połączeń. Dodaj nowe (N) albo zaimportuj z Total Commandera (I).");
  });

  function focusList() {
    setTimeout(() => listEl?.focus());
  }

  function edit(c: Connection | null) {
    form = c ? { ...c } : blank();
    password = "";
    error = null;
    mode = "edit";
  }

  function setTls(t: TlsMode) {
    if (t === "implicit" && form.port === 21) form.port = 990;
    if (t !== "implicit" && form.port === 990) form.port = 21;
    form.tls = t;
  }

  async function save() {
    error = null;
    try {
      const { savedPassword: _, ...conn } = form;
      const id = await ftp.save({ ...conn, port: Number(conn.port) || 21 }, password ? password : null);
      mode = "list";
      await load(id);
      focusList();
    } catch (e) {
      error = String(e);
    }
  }

  async function forgetPassword() {
    try {
      await ftp.forgetPassword(form.id);
      form.savedPassword = false;
      info = t("Zapisane hasło usunięte");
    } catch (e) {
      error = String(e);
    }
  }

  async function removeSel() {
    const c = list[sel];
    if (!c) return;
    if (!confirmDel) {
      confirmDel = true;
      info = t("Naciśnij Del jeszcze raz, aby usunąć „{0}”", c.name);
      return;
    }
    confirmDel = false;
    info = null;
    try {
      await ftp.remove(c.id);
      await load();
    } catch (e) {
      error = String(e);
    }
  }

  async function startImport() {
    error = null;
    importPath = importPath || (await ftp.tcDefaultPath().catch(() => ""));
    mode = "import";
  }

  async function doImport() {
    error = null;
    try {
      const r = await ftp.importTc(importPath);
      mode = "list";
      info =
        t("Zaimportowano: {0}", r.added.length) +
        (r.skipped.length ? ", " + t("pominięto istniejące: {0}", r.skipped.length) : "") +
        (r.pins ? ", " + t("zaufane certyfikaty: {0}", r.pins) : "") +
        ". " +
        t("Hasła nie są importowane — program zapyta przy pierwszym połączeniu.");
      await load(list[sel]?.id);
      focusList();
    } catch (e) {
      error = String(e);
    }
  }

  function connectSel() {
    const c = list[sel];
    if (c) onconnect(c.id);
  }

  function back() {
    mode = "list";
    error = null;
    focusList();
  }

  function listKey(e: KeyboardEvent) {
    const k = e.key;
    let handled = true;
    if (k !== "Delete") confirmDel = false;
    if (k === "ArrowDown") sel = Math.min(list.length - 1, sel + 1);
    else if (k === "ArrowUp") sel = Math.max(0, sel - 1);
    else if (k === "Home") sel = 0;
    else if (k === "End") sel = Math.max(0, list.length - 1);
    else if (k === "Enter") connectSel();
    else if (k === "Delete") void removeSel();
    else if (k === "Insert" || k.toLowerCase() === "n") edit(null);
    else if (k === "F2" || k.toLowerCase() === "e") list[sel] && edit(list[sel]);
    else if (k.toLowerCase() === "i") void startImport();
    else if (k.toLowerCase() === "l") ftp.openLog().catch((err) => (error = String(err)));
    else handled = false;
    if (handled) {
      e.preventDefault();
      e.stopPropagation();
    }
  }

  function formKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      back();
    }
  }

  function initField(el: HTMLInputElement) {
    setTimeout(() => {
      el.focus();
      el.select();
    });
  }

  const tlsLabel: Record<TlsMode, string> = $derived({
    none: t("FTP (bez szyfrowania)"),
    explicit: t("FTPS — jawne TLS (AUTH TLS, zwykle port 21)"),
    implicit: t("FTPS — niejawne TLS (zwykle port 990)"),
  });
  const tlsShort: Record<TlsMode, string> = { none: "FTP", explicit: "FTPES", implicit: "FTPS" };

  const buttons = $derived(
    mode === "edit"
      ? [
          { label: form.id ? t("Zapisz") : t("Dodaj"), primary: true, run: save },
          { label: t("Anuluj"), run: back },
        ]
      : mode === "import"
        ? [
            { label: t("Importuj"), primary: true, run: doImport },
            { label: t("Anuluj"), run: back },
          ]
        : [
            // The letters (N, E, I, L) are the same in every language: see listKey.
            { label: t("Połącz"), primary: true, run: connectSel },
            { label: t("&Nowe"), run: () => edit(null) },
            { label: t("&Edytuj"), run: () => list[sel] && edit(list[sel]) },
            { label: t("&Import z TC"), run: startImport },
            { label: "&Log", run: () => ftp.openLog().catch((e) => (error = String(e))) },
            { label: t("Zamknij"), run: onclose },
          ],
  );
</script>

<Modal title={mode === "edit" ? (form.id ? t("Edycja: {0}", form.name) : t("Nowe połączenie")) : mode === "import" ? t("Import z Total Commandera") : t("Połączenia FTP")} wide {onclose} {buttons}>
  {#if mode === "list"}
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <ul class="conns" bind:this={listEl} data-autofocus tabindex="0" onkeydown={listKey} role="listbox" aria-label={t("Połączenia FTP")}>
      {#each list as c, i (c.id)}
        <li
          class:sel={i === sel}
          onclick={() => ((sel = i), (confirmDel = false))}
          ondblclick={() => onconnect(c.id)}
          onkeydown={() => {}}
          role="option"
          aria-selected={i === sel}
        >
          <span class="badge" class:secure={c.tls !== "none"}>{tlsShort[c.tls]}</span>
          <span class="name">{c.name}</span>
          <span class="host">{c.user ? c.user + "@" : ""}{c.host}:{c.port}</span>
          <span class="key" title={c.savedPassword ? t("Hasło zapamiętane (zaszyfrowane DPAPI)") : t("Hasło: pytaj przy łączeniu")}
            >{c.savedPassword ? "🔑" : ""}</span
          >
        </li>
      {/each}
    </ul>
    <p class="hint">{t("Enter połącz · N nowe · E/F2 edytuj · Del usuń · I import z TC · L log FTP")}</p>
  {:else if mode === "edit"}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="form" onkeydown={formKey}>
      <label class="wide"><span>{t("Nazwa")}</span><input bind:value={form.name} use:initField spellcheck="false" /></label>
      <label class="wide"><span>{t("Serwer (adres lub IP)")}</span><input bind:value={form.host} spellcheck="false" placeholder="ftp.example.com" /></label>
      <label><span>Port</span><input type="number" min="1" max="65535" bind:value={form.port} /></label>
      <label class="wide"
        ><span>{t("Szyfrowanie")}</span>
        <select value={form.tls} onchange={(e) => setTls(e.currentTarget.value as TlsMode)}>
          {#each Object.entries(tlsLabel) as [v, l] (v)}<option value={v}>{l}</option>{/each}
        </select>
      </label>
      <label><span>{t("Użytkownik")}</span><input bind:value={form.user} spellcheck="false" placeholder="anonymous" autocomplete="off" /></label>
      <label
        ><span>{form.savedPassword ? t("Hasło (zapisane — wpisz, by zmienić)") : t("Hasło (puste = pytaj)")}</span>
        <input type="password" bind:value={password} autocomplete="new-password" />
      </label>
      <label
        ><span>{t("Kodowanie nazw plików")}</span>
        <select bind:value={form.encoding}>
          <option value="auto">{t("Automatycznie (UTF-8, a dla starych serwerów wykrywanie)")}</option>
          <option value="utf8">UTF-8</option>
          <option value="cp1250">{t("Windows-1250 (stare polskie serwery)")}</option>
        </select>
      </label>
      <label><span>{t("Katalog startowy (puste = domowy)")}</span><input bind:value={form.initialDir} spellcheck="false" placeholder="/public_html" /></label>
      <label class="check"><input type="checkbox" bind:checked={form.passive} /> {t("Tryb pasywny (zalecany)")}</label>
      <label class="check"><input type="checkbox" bind:checked={form.mlsd} /> {t("MLSD (dokładne daty/rozmiary, jeśli serwer umie)")}</label>
      {#if form.savedPassword}
        <button class="link" onclick={forgetPassword} type="button">{t("Usuń zapisane hasło")}</button>
      {/if}
      <p class="note wide">{t("Hasło jest zapisywane w katalogu programu (data\\passwords.json), zaszyfrowane przez Windows dla Twojego konta — na innym komputerze program zapyta o nie ponownie.")}</p>
    </div>
  {:else}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="form" onkeydown={formKey}>
      <label class="wide"><span>{t("Plik wcx_ftp.ini")}</span><input bind:value={importPath} use:initField spellcheck="false" /></label>
      <p class="note wide">
        {t("Importowane są połączenia (adres, port, szyfrowanie, użytkownik, katalog, tryb pasywny) i zaufane certyfikaty. Hasła nie — przy pierwszym połączeniu program zapyta i może je zapamiętać. Istniejące połączenia o tej samej nazwie są pomijane.")}
      </p>
    </div>
  {/if}
  {#if error}<p class="err">{error}</p>{/if}
  {#if info && mode === "list"}<p class="info">{info}</p>{/if}
</Modal>

<style>
  .conns {
    list-style: none;
    margin: 0;
    padding: 4px;
    min-height: 120px;
    max-height: 50vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    outline: none;
  }
  .conns:focus {
    border-color: var(--accent);
  }
  li {
    display: grid;
    grid-template-columns: 52px minmax(120px, auto) 1fr 20px;
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
  .badge {
    font-size: 10px;
    font-weight: 700;
    text-align: center;
    padding: 1px 4px;
    border-radius: 3px;
    border: 1px solid var(--border-strong);
    color: var(--fg-dim);
  }
  .badge.secure {
    color: var(--accent);
    border-color: var(--accent);
  }
  li.sel .badge {
    color: inherit;
    border-color: currentColor;
  }
  .name {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .host {
    color: var(--fg-dim);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  li.sel .host {
    color: inherit;
  }
  .hint,
  .note {
    margin: 8px 0 0;
    font-size: 11px;
    color: var(--fg-dim);
  }
  .form {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px 14px;
  }
  .form .wide {
    grid-column: 1 / -1;
  }
  .form label {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .form label > span {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .form input:not([type="checkbox"]),
  .form select {
    font: inherit;
    padding: 6px 8px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  .form input:focus,
  .form select:focus {
    border-color: var(--accent);
  }
  .form label.check {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    font-size: 12px;
  }
  .link {
    justify-self: start;
    border: 0;
    background: none;
    color: var(--danger);
    font: inherit;
    font-size: 12px;
    cursor: pointer;
    padding: 0;
  }
  .err {
    margin: 10px 0 0;
    color: var(--danger);
    font-size: 12px;
  }
  .info {
    margin: 10px 0 0;
    font-size: 12px;
    color: var(--accent);
  }
</style>
