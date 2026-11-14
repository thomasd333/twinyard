<script lang="ts">
  import { getLang, LANGS, setLang, t } from "./i18n.svelte";
  import Modal from "./Modal.svelte";
  import type { ToolAction } from "./TitleBar.svelte";

  interface Props {
    actions: ToolAction[];
    hidden: Set<string>;
    theme: "dark" | "light" | null;
    fbarHidden: boolean;
    onfbar: (hidden: boolean) => void;
    ontoolbar: (hidden: Set<string>) => void;
    ontheme: (t: "dark" | "light" | null) => void;
    onconfig: () => void;
    onclose: () => void;
  }
  let { actions, hidden, theme, fbarHidden, onfbar, ontoolbar, ontheme, onconfig, onclose }: Props = $props();

  function toggle(a: ToolAction) {
    const h = new Set(hidden);
    if (h.has(a.id)) h.delete(a.id);
    else h.add(a.id);
    ontoolbar(h);
  }
</script>

<Modal title={t("Konfiguracja")} wide {onclose} buttons={[{ label: t("Zamknij"), primary: true, run: onclose }]}>
  <div class="grid">
    <section>
      <h3>{t("Pasek narzędzi")}</h3>
      <p class="hint">{t("Zaznaczone przyciski są widoczne na górze okna. Każda funkcja ma też skrót (F1).")}</p>
      <div class="list">
        {#each actions as a (a.id)}
          <label class="item">
            <input type="checkbox" checked={!hidden.has(a.id)} onchange={() => toggle(a)} />
            <svg viewBox="0 0 16 16" style:color={a.color}><path d={a.icon} /></svg>
            <span>{a.title}</span>
          </label>
        {/each}
      </div>
      <div class="row">
        <button onclick={() => ontoolbar(new Set())}>{t("Pokaż wszystkie")}</button>
        <button onclick={() => ontoolbar(new Set(actions.map((a) => a.id)))}>{t("Ukryj wszystkie")}</button>
      </div>
    </section>
    <section>
      <h3>Język / Language</h3>
      {#each LANGS as l (l.id)}
        <label class="radio"><input type="radio" name="lang" checked={getLang() === l.id} onchange={() => setLang(l.id)} /> {l.name}</label>
      {/each}

      <h3>{t("Motyw")}</h3>
      <label class="radio"><input type="radio" name="theme" checked={theme === null} onchange={() => ontheme(null)} /> {t("Jak w systemie")}</label>
      <label class="radio"><input type="radio" name="theme" checked={theme === "light"} onchange={() => ontheme("light")} /> {t("Jasny")}</label>
      <label class="radio"><input type="radio" name="theme" checked={theme === "dark"} onchange={() => ontheme("dark")} /> {t("Ciemny")}</label>

      <h3>{t("Dolny pasek")}</h3>
      <label class="radio"
        ><input type="checkbox" checked={!fbarHidden} onchange={() => onfbar(!fbarHidden)} /> {t("Pokaż przyciski F2–F8 na dole okna")}</label
      >

      <h3>{t("Programy zewnętrzne i wyszukiwanie")}</h3>
      <!-- Our own constant texts (with <b>), never user data. -->
      <p class="hint">
        {@html t("Edytor (F4), podgląd (F3), porównywarka (Ctrl+F2), lista rozszerzeń „tekstowych” dla wyszukiwania i kolory interfejsu są w pliku <b>config.toml</b> (zmiany działają od razu).")}
      </p>
      <button onclick={onconfig}>{t("Otwórz config.toml")}</button>

      <h3>{t("Dane programu")}</h3>
      <p class="hint">
        {@html t("Wszystko (ustawienia, połączenia FTP, zaszyfrowane hasła, logi) jest w katalogu <b>data\\</b> obok twinyard.exe — program jest w pełni przenośny.")}
      </p>
    </section>
  </div>
</Modal>

<style>
  .grid {
    display: grid;
    grid-template-columns: 1.2fr 1fr;
    gap: 24px;
  }
  h3 {
    margin: 0 0 6px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--accent);
  }
  section h3:not(:first-child) {
    margin-top: 16px;
  }
  .hint {
    margin: 0 0 8px;
    font-size: 12px;
    color: var(--fg-dim);
  }
  .list {
    max-height: 46vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 4px;
  }
  .item {
    display: grid;
    grid-template-columns: 18px 18px 1fr;
    gap: 8px;
    align-items: center;
    padding: 3px 4px;
    font-size: 12px;
    border-radius: 4px;
    cursor: pointer;
  }
  .item:hover {
    background: var(--hover);
  }
  .item svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
    color: var(--fg-dim);
  }
  .row {
    display: flex;
    gap: 6px;
    margin-top: 8px;
  }
  .radio {
    display: flex;
    gap: 8px;
    align-items: center;
    font-size: 12px;
    padding: 2px 0;
  }
  button {
    font: inherit;
    font-size: 12px;
    padding: 4px 10px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    background: var(--hover);
    color: var(--fg);
    cursor: pointer;
  }
  button:hover {
    border-color: var(--accent);
  }
</style>
