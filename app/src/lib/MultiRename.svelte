<script lang="ts" module>
  export interface RenameItem {
    name: string;
    dir: boolean;
    mtime: number;
  }

  // Last settings survive closing the window (like TC's Ctrl+M).
  const saved = { mask: "[N].[E]", find: "", replace: "", regex: false, cased: "keep", start: 1, step: 1, digits: 1 };
</script>

<script lang="ts">
  import { api, joinPath } from "./api";
  import { splitExt } from "./format";
  import { t } from "./i18n.svelte";
  import Modal from "./Modal.svelte";

  interface Props {
    dir: string;
    items: RenameItem[];
    /** Every name in the folder (taken names outside the selection). */
    others: string[];
    onclose: () => void;
    /** After renaming: focus this name (the first renamed) and refresh. */
    ondone: (focus: string | null) => void;
  }
  let { dir, items, others, onclose, ondone }: Props = $props();

  let mask = $state(saved.mask);
  let find = $state(saved.find);
  let replace = $state(saved.replace);
  let regex = $state(saved.regex);
  let cased = $state(saved.cased as "keep" | "lower" | "upper" | "first");
  let start = $state(saved.start);
  let step = $state(saved.step);
  let digits = $state(saved.digits);
  let running = $state(false);
  let errors = $state<string[]>([]);

  /** Typing replaces the remembered mask (like TC); arrows keep it. */
  function selectAll(el: HTMLInputElement) {
    setTimeout(() => el.select());
  }

  const pad = (n: number) => String(n).padStart(2, "0");
  const parentName = $derived(dir.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? "");

  function render(it: RenameItem, k: number): string {
    const [stem, ext] = it.dir ? [it.name, ""] : splitExt(it.name);
    const d = new Date(it.mtime);
    const counter = String(Number(start) + k * Number(step)).padStart(Math.max(1, Number(digits) || 1), "0");
    let out = mask.replace(/\[(N|E|C|P|Y|M|D)\]/g, (_, c: string) =>
      c === "N" ? stem : c === "E" ? ext : c === "C" ? counter : c === "P" ? parentName : c === "Y" ? String(d.getFullYear()) : c === "M" ? pad(d.getMonth() + 1) : pad(d.getDate()),
    );
    if (find) {
      if (regex) {
        try {
          out = out.replace(new RegExp(find, "g"), replace);
        } catch {
          /* invalid regex: shown below, name unchanged */
        }
      } else out = out.split(find).join(replace);
    }
    if (cased === "lower") out = out.toLowerCase();
    else if (cased === "upper") out = out.toUpperCase();
    else if (cased === "first") out = out.charAt(0).toUpperCase() + out.slice(1).toLowerCase();
    return out.trim().replace(/[. ]+$/, ""); // "x." for a file without extension; Windows drops trailing dots anyway
  }

  const regexError = $derived.by(() => {
    if (!regex || !find) return null;
    try {
      new RegExp(find);
      return null;
    } catch (e) {
      return String(e);
    }
  });

  /** Preview rows with a problem per row (null = fine). */
  const rows = $derived.by(() => {
    const out = items.map((it, k) => ({ from: it.name, to: render(it, k), problem: null as string | null }));
    const count = new Map<string, number>();
    for (const r of out) count.set(r.to.toLowerCase(), (count.get(r.to.toLowerCase()) ?? 0) + 1);
    // Names of files that are not being renamed stay taken.
    const mine = new Set(items.map((it) => it.name.toLowerCase()));
    const taken = new Set(others.map((n) => n.toLowerCase()).filter((n) => !mine.has(n)));
    for (const r of out) {
      if (!r.to) r.problem = t("pusta nazwa");
      else if (/[\\/:*?"<>|]/.test(r.to)) r.problem = t("niedozwolony znak");
      else if ((count.get(r.to.toLowerCase()) ?? 0) > 1) r.problem = t("powtórzona nazwa");
      else if (taken.has(r.to.toLowerCase())) r.problem = t("nazwa zajęta");
    }
    return out;
  });
  const changed = $derived(rows.filter((r) => r.from !== r.to));
  const blocked = $derived(rows.some((r) => r.problem) || !!regexError);

  async function apply() {
    if (running || blocked || !changed.length) return;
    Object.assign(saved, { mask, find, replace, regex, cased, start, step, digits });
    running = true;
    errors = [];
    const todo = changed.map((r) => ({ ...r, orig: r.from }));
    // A new name that is another item's old name (a↔b, 1→2 / 2→3): first
    // move every item to a temporary name, then to the final one.
    const olds = new Set(todo.map((r) => r.from.toLowerCase()));
    const twoPhase = todo.some((r) => olds.has(r.to.toLowerCase()) && r.to.toLowerCase() !== r.from.toLowerCase());
    let first: string | null = null;
    try {
      if (twoPhase) {
        const stamp = Date.now().toString(36);
        for (const [k, r] of todo.entries()) {
          const tmp = `${r.from}.~fcmd${stamp}${k}`;
          try {
            await api.renamePath(joinPath(dir, r.from), joinPath(dir, tmp));
            r.from = tmp;
          } catch (e) {
            errors.push(`${r.from}: ${e}`);
            r.to = ""; // not moved: leave it
          }
        }
      }
      for (const r of todo) {
        if (!r.to) continue;
        try {
          await api.renamePath(joinPath(dir, r.from), joinPath(dir, r.to));
          first ??= r.to;
        } catch (e) {
          errors.push(`${r.orig} → ${r.to}: ${e}`);
          // Never leave a file under its temporary name: back to the old one.
          if (r.from !== r.orig) {
            try {
              await api.renamePath(joinPath(dir, r.from), joinPath(dir, r.orig));
            } catch (e2) {
              errors.push(`${r.from} → ${r.orig}: ${e2}`);
            }
          }
        }
      }
    } finally {
      running = false;
    }
    if (!errors.length) {
      ondone(first);
      onclose();
    } else ondone(first); // keep the window open with the errors
  }
</script>

<Modal
  title={t("Zmiana nazw: {0}", items.length)}
  wide
  {onclose}
  buttons={[
    { label: running ? t("Zmieniam…") : t("Zmień nazwy ({0})", changed.length), primary: true, run: apply },
    { label: t("Zamknij"), run: onclose },
  ]}
>
  <div class="form">
    <label class="wide">
      <span>{t("Maska: [N] nazwa, [E] rozszerzenie, [C] licznik, [P] katalog, [Y] [M] [D] data pliku")}</span>
      <input bind:value={mask} data-autofocus use:selectAll spellcheck="false" />
    </label>
    <label><span>{t("Znajdź")}</span><input bind:value={find} spellcheck="false" /></label>
    <label><span>{t("Zamień na")}</span><input bind:value={replace} spellcheck="false" placeholder={regex ? "$1" : ""} /></label>
    <div class="opts wide">
      <label class="check"><input type="checkbox" bind:checked={regex} /> {t("Wyrażenie regularne")}</label>
      <label class="sel"
        >{t("Wielkość liter")}
        <select bind:value={cased}>
          <option value="keep">{t("bez zmian")}</option>
          <option value="lower">{t("małe")}</option>
          <option value="upper">{t("WIELKIE")}</option>
          <option value="first">{t("Pierwsza wielka")}</option>
        </select>
      </label>
      <label class="num">{t("Licznik od")} <input type="number" bind:value={start} /></label>
      <label class="num">{t("krok")} <input type="number" bind:value={step} /></label>
      <label class="num">{t("cyfr")} <input type="number" min="1" max="9" bind:value={digits} /></label>
    </div>
  </div>
  {#if regexError}<p class="err">{regexError}</p>{/if}
  <div class="preview">
    {#each rows as r (r.from)}
      <div class="row" class:same={r.from === r.to} class:bad={r.problem}>
        <span class="from" title={r.from}>{r.from}</span>
        <span class="arrow">→</span>
        <span class="to" title={r.to}>{r.to}</span>
        {#if r.problem}<span class="problem">{r.problem}</span>{/if}
      </div>
    {/each}
  </div>
  {#if errors.length}
    <div class="err">
      <b>{t("Błędy: {0}", errors.length)}</b>
      {#each errors.slice(0, 20) as e, k (k)}<div>{e}</div>{/each}
    </div>
  {/if}
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
  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
  }
  label > span {
    color: var(--fg-dim);
  }
  input,
  select {
    font: inherit;
    padding: 5px 8px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  input:focus,
  select:focus {
    border-color: var(--accent);
  }
  .opts {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    align-items: center;
  }
  .opts label {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  .num input {
    width: 56px;
    padding: 3px 6px;
  }
  .sel select {
    padding: 3px 6px;
  }
  .preview {
    margin-top: 10px;
    max-height: 44vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px;
    font-size: 12px;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 18px minmax(0, 1fr) auto;
    gap: 6px;
    padding: 2px 4px;
  }
  .row span {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .row.same {
    opacity: 0.5;
  }
  .row .to {
    font-weight: 600;
  }
  .row.bad .to,
  .problem,
  .err {
    color: var(--danger);
  }
  .arrow {
    color: var(--fg-dim);
  }
  .err {
    margin: 8px 0 0;
    font-size: 12px;
    overflow-wrap: anywhere;
  }
</style>
