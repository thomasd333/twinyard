<script lang="ts">
  import { errorText, type ConflictAction, type ErrorAction } from "./api";
  import { t, tn } from "./i18n.svelte";
  import { formatBytes, formatDate, formatHuman } from "./format";
  import type { JobsState } from "./jobs.svelte";
  import Modal, { type ModalButton } from "./Modal.svelte";

  let { jobs }: { jobs: JobsState } = $props();

  /** Job questions appear on their own, maybe under a key meant for the panel. */
  const ARM = 600;

  const ask = $derived(jobs.asks[0]);
  /** Typed password; cleared for every new question. */
  let pw = $state("");
  $effect(() => {
    if (ask?.kind === "password") pw = "";
  });

  function conflictButtons(): ModalButton[] {
    const a = ask!;
    const c = (action: ConflictAction, all = false) => () => jobs.answer(a, { action, all });
    return [
      { label: t("&Nadpisz"), primary: true, run: c("overwrite") },
      // A shorter file on the other side: continue the broken FTP transfer.
      ...(a.kind === "conflict" && a.conflict.canResume ? [{ label: t("&Dokończ (wznów)"), run: c("resume") }] : []),
      { label: t("Nadpisz &wszystkie"), run: c("overwrite", true) },
      { label: t("Nadpisz &starsze"), run: c("overwriteOlder", true) },
      { label: t("&Pomiń"), run: c("skip") },
      { label: t("Po&miń wszystkie"), run: c("skip", true) },
      { label: t("&Zachowaj oba"), run: c("rename") },
      { label: t("Anuluj"), run: c("cancel") },
    ];
  }

  function errorButtons(): ModalButton[] {
    const a = ask!;
    const r = (action: ErrorAction) => () => jobs.answer(a, action);
    return [
      { label: t("P&onów"), primary: true, run: r("retry") },
      { label: t("&Pomiń"), run: r("skip") },
      { label: t("Po&miń wszystkie"), run: r("skipAll") },
      { label: t("Anuluj"), run: r("cancel") },
    ];
  }

  function pct(j: (typeof jobs.jobs)[number]): number {
    const p = j.progress;
    if (j.outcome) return 100;
    if (!p || p.phase === "scan") return 0;
    if (p.totalBytes > 0) return (p.doneBytes / p.totalBytes) * 100;
    return p.totalFiles ? (p.doneFiles / p.totalFiles) * 100 : 0;
  }

  function eta(j: (typeof jobs.jobs)[number]): string {
    const p = j.progress;
    if (!p || !j.speed || p.totalBytes === 0) return "";
    const s = Math.round((p.totalBytes - p.doneBytes) / j.speed);
    if (!isFinite(s) || s < 1) return "";
    return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${s % 60} s`;
  }

  function fileName(path: string) {
    return path.slice(path.lastIndexOf("\\") + 1);
  }
</script>

{#if jobs.jobs.length}
  <div class="jobs">
    {#if jobs.waitingCount || jobs.paused}
      <div class="queue">
        <span>
          {jobs.paused ? t("Kolejka wstrzymana") : t("Kolejka")}{jobs.waitingCount ? " · " + t("czeka: {0}", jobs.waitingCount) : ""}
        </span>
        <button class="x" onclick={() => jobs.setPaused(!jobs.paused)}>
          {jobs.paused ? t("Wznów kolejkę") : t("Wstrzymaj kolejkę")}
        </button>
      </div>
    {/if}
    {#each jobs.jobs as j (j.key)}
      {@const p = j.progress}
      <div class="job" class:done={j.outcome} class:failed={j.outcome && j.outcome.errors.length} class:waiting={j.waiting && !j.outcome}>
        <div class="top">
          <span class="title">{j.title}</span>
          {#if j.outcome}
            <button class="x" onclick={() => jobs.dismiss(j.key)} title={t("Zamknij")}>✕</button>
          {:else}
            {#if j.waiting}
              <button class="x" onclick={() => jobs.startNow(j)} title={t("Uruchom od razu, obok trwającego zadania")}>{t("Uruchom teraz")}</button>
            {/if}
            <button class="x" onclick={() => jobs.cancel(j)} title={t("Anuluj")}>{t("Anuluj")}</button>
          {/if}
        </div>
        {#if !j.waiting || j.outcome}
          <div class="bar" class:indeterminate={!j.outcome && (!p || p.phase === "scan")}>
            <div style:width="{pct(j)}%"></div>
          </div>
        {/if}
        <div class="meta">
          {#if j.waiting && !j.outcome}
            {jobs.paused ? t("Czeka — kolejka wstrzymana") : t("Czeka w kolejce")}
          {:else if j.outcome}
            {j.outcome.cancelled ? t("Przerwano") : t("Gotowe")} · {j.outcome.doneFiles} OK{j.outcome.skipped ? ", " + t("pominięto {0}", j.outcome.skipped) : ""}
          {:else if !p || p.phase === "scan"}
            {t("Liczenie…")} {p ? `${p.totalFiles} ${tn(p.totalFiles, "plik|pliki|plików")}, ${formatHuman(p.totalBytes)}` : ""}
          {:else}
            {p.doneFiles} / {p.totalFiles}
            {#if p.totalBytes}· {formatHuman(p.doneBytes)} / {formatHuman(p.totalBytes)}{/if}
            {#if j.speed > 0}· {formatHuman(j.speed)}/s{/if}
            {#if eta(j)}· {eta(j)}{/if}
          {/if}
        </div>
        {#if !j.outcome && p?.current}<div class="current" title={p.current}>{fileName(p.current)}</div>{/if}
        {#if j.outcome?.errors.length}
          <details>
            <summary>{t("Błędy: {0}", j.outcome.errors.length)}</summary>
            <ul>
              {#each j.outcome.errors.slice(0, 50) as e, k (k)}
                <li title={e.path}><b>{fileName(e.path) || e.path}</b> — {errorText(e.message)}</li>
              {/each}
            </ul>
          </details>
        {/if}
      </div>
    {/each}
  </div>
{/if}

{#if ask?.kind === "conflict"}
  {@const c = ask.conflict}
  {#key ask}
  <Modal title={t("Plik już istnieje")} wide armDelay={ARM} buttons={conflictButtons()} onclose={() => jobs.answer(ask, { action: "cancel", all: false })}>
    <p class="target" title={c.target}>{c.target}</p>
    <table class="cmp">
      <tbody>
        <tr>
          <th>{t("Nowy")}</th>
          <td class="num">{formatBytes(c.sourceSize)} B</td>
          <td class:newer={c.sourceMtime > c.targetMtime}>{formatDate(c.sourceMtime)}</td>
        </tr>
        <tr>
          <th>{t("Istniejący")}</th>
          <td class="num">{formatBytes(c.targetSize)} B</td>
          <td class:newer={c.targetMtime > c.sourceMtime}>{formatDate(c.targetMtime)}</td>
        </tr>
      </tbody>
    </table>
  </Modal>
  {/key}
{:else if ask?.kind === "password"}
  {#key ask}
    <Modal
      title={ask.wrong ? t("Złe hasło") : t("Archiwum z hasłem")}
      buttons={[
        { label: "OK", primary: true, run: () => jobs.answer(ask, pw) },
        { label: t("Anuluj"), run: () => jobs.answer(ask, null) },
      ]}
      onclose={() => jobs.answer(ask, null)}
    >
      <p class="target" title={ask.archive}>{ask.archive}</p>
      {#if ask.wrong}<p class="err">{t("Złe hasło — spróbuj jeszcze raz.")}</p>{/if}
      <label class="pw">
        <span>{t("Hasło:")}</span>
        <input type="password" bind:value={pw} autocomplete="off" spellcheck="false" data-autofocus />
      </label>
    </Modal>
  {/key}
{:else if ask?.kind === "error"}
  {#key ask}
  <Modal title={t("Błąd operacji")} armDelay={ARM} buttons={errorButtons()} onclose={() => jobs.answer(ask, "cancel")}>
    <p class="target" title={ask.error.path}>{ask.error.path}</p>
    <p class="err">{errorText(ask.error.message)}</p>
  </Modal>
  {/key}
{/if}

<style>
  .jobs {
    position: fixed;
    right: 12px;
    bottom: 48px;
    width: 360px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    z-index: 85; /* job cards above full-screen views (sync runs jobs) */
    max-height: calc(100vh - 120px); /* a long queue scrolls */
    overflow-y: auto;
  }
  .job {
    background: var(--panel-bg);
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    padding: 10px 12px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.25);
    font-size: 12px;
  }
  .job.waiting {
    opacity: 0.75;
  }
  .job.waiting .meta {
    margin-top: 6px;
  }
  .queue {
    display: flex;
    gap: 8px;
    align-items: center;
    justify-content: space-between;
    background: var(--panel-bg);
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    padding: 6px 12px;
    font-size: 12px;
    color: var(--fg-dim);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.25);
  }
  .top {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .title {
    flex: 1;
    font-weight: 600;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .x {
    font: inherit;
    font-size: 11px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--fg-dim);
    border-radius: 4px;
    padding: 1px 7px;
    cursor: pointer;
  }
  .x:hover {
    color: var(--fg);
    border-color: var(--border-strong);
  }
  .bar {
    height: 5px;
    margin: 8px 0 6px;
    background: var(--hover);
    border-radius: 3px;
    overflow: hidden;
  }
  .bar > div {
    height: 100%;
    background: var(--accent);
    transition: width 0.15s linear;
  }
  .done .bar > div {
    background: #2e9d5b;
  }
  .failed .bar > div {
    background: var(--danger);
  }
  .bar.indeterminate > div {
    width: 30% !important;
    animation: slide 1s ease-in-out infinite alternate;
  }
  @keyframes slide {
    from {
      margin-left: 0;
    }
    to {
      margin-left: 70%;
    }
  }
  .meta {
    color: var(--fg-dim);
    font-variant-numeric: tabular-nums;
  }
  .current {
    margin-top: 2px;
    color: var(--fg-dim);
    opacity: 0.8;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  details {
    margin-top: 6px;
  }
  summary {
    cursor: pointer;
    color: var(--danger);
  }
  ul {
    margin: 4px 0 0;
    padding-left: 16px;
    max-height: 160px;
    overflow: auto;
  }
  li {
    margin: 2px 0;
    word-break: break-word;
  }
  .target {
    margin: 0 0 10px;
    font-size: 12px;
    color: var(--fg-dim);
    word-break: break-all;
  }
  .err {
    margin: 0;
    color: var(--danger);
  }
  .pw {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 10px;
  }
  .pw span {
    color: var(--fg-dim);
    font-size: 12px;
  }
  .pw input {
    font: inherit;
    padding: 6px 8px;
    border-radius: 5px;
    border: 1px solid var(--border-strong);
    background: var(--bg);
    color: var(--fg);
    outline: none;
  }
  .pw input:focus {
    border-color: var(--accent);
  }
  .cmp {
    border-collapse: collapse;
    font-variant-numeric: tabular-nums;
  }
  .cmp th {
    text-align: left;
    font-weight: 600;
    padding: 3px 16px 3px 0;
  }
  .cmp td {
    padding: 3px 16px 3px 0;
  }
  .num {
    text-align: right;
  }
  .newer {
    color: var(--accent);
    font-weight: 600;
  }
</style>
