import { listen } from "@tauri-apps/api/event";
import { api, type Ask, type OpSpec, type Outcome, type Progress } from "./api";

export interface Job {
  /** Stable key for the UI (the backend id only exists once the job runs). */
  key: number;
  /** Backend job id; 0 while waiting in the queue. */
  id: number;
  title: string;
  spec: OpSpec;
  progress: Progress | null;
  /** Bytes per second, smoothed. */
  speed: number;
  outcome: Outcome | null;
  started: number;
  /** Doesn't touch the panels when done (downloads to %TEMP% for viewing). */
  keepPanels?: boolean;
  /** Takes part in the queue: one such job runs at a time. */
  queued: boolean;
  /** In the queue, not started yet. */
  waiting: boolean;
}

export interface JobOpts {
  keepPanels?: boolean;
  /** Skip the queue (the user waits for the result, e.g. a file to view). */
  now?: boolean;
}

interface Sample {
  t: number;
  bytes: number;
}

/** Long transfers go through the queue; quick things (trash, delete) don't. */
const QUEUED_KINDS = new Set<OpSpec["kind"]>(["copy", "move", "pack"]);

/**
 * Background jobs as seen by the UI. Copy / move / pack run one after another
 * (one FTP connection, predictable order, no two jobs racing for the same
 * files); a waiting job can be started out of order with `startNow`.
 */
export class JobsState {
  jobs: Job[] = $state([]);
  /** Questions waiting for the user, oldest first. */
  asks: Ask[] = $state([]);
  /** Queue paused: the running job goes on, the next ones don't start. */
  paused = $state(false);
  #nextKey = 1;
  #samples = new Map<number, Sample>();
  /** Events that arrived before `start()` learned the job id (tiny jobs). */
  #early = new Map<number, { progress?: Progress; outcome?: Outcome }>();
  #onDone: (job: Job) => void;
  #waiters = new Map<number, (o: Outcome) => void>();

  constructor(onDone: (job: Job) => void) {
    this.#onDone = onDone;
  }

  async init() {
    await listen<Progress & { id: number }>("job-progress", (e) => {
      const { id, ...progress } = e.payload;
      const job = this.#byId(id);
      if (job) this.#applyProgress(job, progress);
      else this.#early.set(id, { ...this.#early.get(id), progress });
    });
    await listen<Ask>("job-ask", (e) => {
      this.asks = [...this.asks, e.payload];
    });
    await listen<{ id: number; outcome: Outcome }>("job-done", (e) => {
      const job = this.#byId(e.payload.id);
      if (job) this.#finish(job, e.payload.outcome);
      else this.#early.set(e.payload.id, { ...this.#early.get(e.payload.id), outcome: e.payload.outcome });
    });
  }

  #byId(id: number): Job | undefined {
    return this.jobs.find((j) => j.id === id && !j.waiting);
  }

  #applyProgress(job: Job, p: Progress) {
    const now = performance.now();
    const prev = this.#samples.get(job.key);
    if (!prev) this.#samples.set(job.key, { t: now, bytes: p.doneBytes });
    else if (now - prev.t > 250) {
      const inst = ((p.doneBytes - prev.bytes) * 1000) / (now - prev.t);
      job.speed = job.speed ? job.speed * 0.7 + inst * 0.3 : inst;
      this.#samples.set(job.key, { t: now, bytes: p.doneBytes });
    }
    job.progress = p;
  }

  #finish(job: Job, outcome: Outcome, notify = true) {
    if (job.outcome) return;
    if (job.id) this.asks = this.asks.filter((a) => a.id !== job.id);
    this.#samples.delete(job.key);
    job.outcome = outcome;
    job.waiting = false;
    if (notify) {
      try {
        this.#onDone(job);
      } catch (e) {
        console.error(e);
      }
    }
    this.#waiters.get(job.key)?.(outcome);
    this.#waiters.delete(job.key);
    // Clean results disappear by themselves; errors stay until dismissed.
    if (!outcome.errors.length) setTimeout(() => this.dismiss(job.key), 2500);
    // A failed job doesn't hold up the rest: it keeps its card, the next one starts.
    this.#pump();
  }

  /** Starts a job and resolves when it is finished (for sequential batches). */
  async run(spec: OpSpec, title: string, opts: JobOpts = {}): Promise<Outcome> {
    const job = await this.start(spec, title, opts);
    if (job.outcome) return job.outcome;
    return new Promise((resolve) => this.#waiters.set(job.key, resolve));
  }

  /**
   * Adds a job. Out-of-queue jobs and a queued job with nothing ahead start
   * right away (a refused start throws, so the dialog can show it); the rest
   * wait for their turn.
   */
  async start(spec: OpSpec, title: string, opts: JobOpts = {}): Promise<Job> {
    const queued = !opts.now && !opts.keepPanels && QUEUED_KINDS.has(spec.kind);
    this.jobs.push({
      key: this.#nextKey++,
      id: 0,
      title,
      spec,
      progress: null,
      speed: 0,
      outcome: null,
      started: Date.now(),
      keepPanels: opts.keepPanels,
      queued,
      waiting: true,
    });
    const job = this.jobs[this.jobs.length - 1]; // the reactive proxy
    if (!queued || this.#canStartNext(job)) {
      try {
        await this.#launch(job);
      } catch (e) {
        this.jobs = this.jobs.filter((j) => j.key !== job.key);
        this.#pump();
        throw e;
      }
    }
    return job;
  }

  /** Nothing queued is running, the queue isn't paused and `job` is first in line. */
  #canStartNext(job: Job): boolean {
    if (this.paused) return false;
    if (this.jobs.some((j) => j.queued && !j.waiting && !j.outcome)) return false;
    return this.jobs.find((j) => j.queued && j.waiting && !j.outcome) === job;
  }

  async #launch(job: Job) {
    job.waiting = false;
    job.started = Date.now();
    const id = await api.startJob($state.snapshot(job.spec) as OpSpec);
    job.id = id;
    const early = this.#early.get(id);
    if (early) {
      this.#early.delete(id);
      if (early.progress) this.#applyProgress(job, early.progress);
      if (early.outcome) this.#finish(job, early.outcome);
    }
  }

  /** Starts the next waiting job when its turn has come. */
  #pump() {
    const next = this.jobs.find((j) => j.queued && j.waiting && !j.outcome);
    if (!next || !this.#canStartNext(next)) return;
    this.#launch(next).catch((e) => {
      this.#finish(next, { cancelled: false, doneFiles: 0, skipped: 0, errors: [{ path: "", message: String(e) }] });
    });
  }

  /** A waiting job starts at once, next to whatever is running. */
  startNow(job: Job) {
    if (!job.waiting || job.outcome) return;
    job.queued = false;
    this.#launch(job).catch((e) => {
      this.#finish(job, { cancelled: false, doneFiles: 0, skipped: 0, errors: [{ path: "", message: String(e) }] });
    });
  }

  setPaused(paused: boolean) {
    this.paused = paused;
    if (!paused) this.#pump();
  }

  answer(ask: Ask, value: unknown) {
    this.asks = this.asks.filter((a) => a !== ask);
    void api.jobAnswer(ask.id, value);
  }

  cancel(job: Job) {
    if (job.outcome) return;
    if (job.waiting) {
      // Never started: nothing changed on disk, the panels need no reload.
      this.#finish(job, { cancelled: true, doneFiles: 0, skipped: 0, errors: [] }, false);
      this.dismiss(job.key);
      return;
    }
    if (!job.id) return; // still being started; its card gets the cancel button in a moment
    this.asks = this.asks.filter((a) => a.id !== job.id);
    void api.jobCancel(job.id);
  }

  /**
   * Closing the program: stop everything and wait until the jobs have
   * cleaned up (partial files removed, old versions kept). false = some job
   * didn't finish within `timeoutMs`.
   */
  async cancelAll(timeoutMs: number): Promise<boolean> {
    this.paused = true;
    const end = Date.now() + timeoutMs;
    while (this.busy && Date.now() < end) {
      for (const j of [...this.jobs]) this.cancel(j);
      await new Promise((r) => setTimeout(r, 100));
    }
    return !this.busy;
  }

  dismiss(key: number) {
    this.jobs = this.jobs.filter((j) => j.key !== key);
  }

  get busy(): boolean {
    return this.jobs.some((j) => !j.outcome);
  }

  /** Jobs waiting for their turn. */
  get waitingCount(): number {
    return this.jobs.filter((j) => j.waiting && !j.outcome).length;
  }
}
