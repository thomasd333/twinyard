// Logic test of the job queue (app/src/lib/jobs.svelte.ts) with a mocked backend.
// Run: node scripts/test-queue.mjs
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import { pathToFileURL } from "node:url";
import assert from "node:assert/strict";

const app = join(import.meta.dirname, "..", "app") + "/";
const tmp = mkdtempSync(join(tmpdir(), "twinyard-queue-"));
const req = createRequire(app + "package.json");
const ts = req("typescript");
const cm = await import(pathToFileURL(req.resolve("svelte/compiler"))); const compileModule = cm.compileModule ?? cm.default.compileModule;

let src = readFileSync(app + "src/lib/jobs.svelte.ts", "utf8");
src = ts.transpileModule(src, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText;
let js = compileModule(src, { generate: "client", filename: "jobs.svelte.js" }).js.code;
const here = pathToFileURL(tmp + "/").href;
js = js
  .replace(/from ["']svelte\/internal\/client["']/, `from "${pathToFileURL(req.resolve("svelte/internal/client")).href}"`)
  .replace(/import ["']svelte\/internal\/disclose-version["'];?/, "")
  .replace(/from ["']@tauri-apps\/api\/event["']/, `from "${here}queue-mock.mjs"`)
  .replace(/from ["']\.\/api["']/, `from "${here}queue-mock.mjs"`);
writeFileSync(tmp + "/queue-jobs.gen.mjs", js);

writeFileSync(
  tmp + "/queue-mock.mjs",
  `export const handlers = {};
export const started = [];
export let failNext = false;
export function setFail(v) { failNext = v; }
let next = 100;
export async function listen(name, fn) { handlers[name] = fn; }
export const api = {
  async startJob(spec) { await 0; if (failNext) { failNext = false; throw "brak dostępu"; } const id = next++; started.push({ id, spec }); return id; },
  async jobCancel(id) { queueMicrotask(() => handlers["job-done"]({ payload: { id, outcome: { cancelled: true, doneFiles: 0, skipped: 0, errors: [] } } })); },
  async jobAnswer() {},
};`,
);

const mock = await import(here + "queue-mock.mjs");
const { JobsState } = await import(here + "queue-jobs.gen.mjs");
const tick = () => new Promise((r) => setTimeout(r, 5));
const done = (id, errors = []) => mock.handlers["job-done"]({ payload: { id, outcome: { cancelled: false, doneFiles: 1, skipped: 0, errors } } });
const spec = (kind, n) => ({ kind, sources: ["C:\\a\\" + n], dest: "D:\\" });

const finished = [];
const jobs = new JobsState((j) => finished.push(j.title));
await jobs.init();

// 1. Two copies + a move: only the first starts; trash bypasses the queue.
await jobs.start(spec("copy", 1), "c1");
await jobs.start(spec("copy", 2), "c2");
await jobs.start(spec("move", 3), "m3");
await jobs.start(spec("trash", 4), "t4");
await tick();
assert.deepEqual(mock.started.map((s) => s.spec.sources[0]), ["C:\\a\\1", "C:\\a\\4"], "only c1 and trash run");
assert.equal(jobs.waitingCount, 2);
assert.equal(jobs.busy, true);

// 2. c1 fails -> stays with error, c2 starts anyway.
done(100, [{ path: "x", message: "zła" }]);
await tick();
assert.equal(mock.started.length, 3);
assert.equal(mock.started[2].spec.sources[0], "C:\\a\\2");
assert.ok(jobs.jobs.find((j) => j.title === "c1").outcome.errors.length, "failed card stays");

// 3. Pause: c2 finishes, m3 waits; resume starts it.
jobs.setPaused(true);
done(102);
await tick();
assert.equal(mock.started.length, 3, "paused: m3 waits");
jobs.setPaused(false);
await tick();
assert.equal(mock.started[3].spec.sources[0], "C:\\a\\3");

// 4. Start now: a waiting job runs beside the current one, the queue still holds the next.
await jobs.start(spec("copy", 5), "c5");
await jobs.start(spec("copy", 6), "c6");
await tick();
assert.equal(mock.started.length, 4);
jobs.startNow(jobs.jobs.find((j) => j.title === "c6"));
await tick();
assert.equal(mock.started[4].spec.sources[0], "C:\\a\\6", "c6 jumped");
done(104); // c6 done: c5 must still wait for m3
await tick();
assert.equal(mock.started.length, 5);

// 5. Cancel a waiting job: gone, never started, no panel reload.
const before = finished.length;
await jobs.start(spec("copy", 7), "c7");
const c7 = jobs.jobs.find((j) => j.title === "c7");
jobs.cancel(c7);
await tick();
assert.ok(!jobs.jobs.some((j) => j.title === "c7"), "cancelled waiting card disappears");
assert.equal(finished.length, before, "no onDone for a never-started job");

// 6. m3 done -> c5 starts; c5's start is refused -> error card, queue goes on.
await jobs.start(spec("copy", 8), "c8");
mock.setFail(true);
done(103);
await tick();
await tick();
const c5 = jobs.jobs.find((j) => j.title === "c5");
assert.equal(c5.outcome?.errors[0]?.message, "brak dostępu", "refused start becomes an error card");
assert.equal(mock.started.at(-1).spec.sources[0], "C:\\a\\8", "c8 started after refused c5");

// 7. run() resolves only when its queued job finishes.
let resolved = null;
const p = jobs.run(spec("copy", 9), "c9").then((o) => (resolved = o));
await tick();
assert.equal(resolved, null);
assert.equal(mock.started.at(-1).spec.sources[0], "C:\\a\\8", "c9 waits behind c8");
done(mock.started.at(-1).id);
await tick();
done(mock.started.at(-1).id); // c9
await p;
assert.equal(resolved.cancelled, false);

// 8. Immediate refusal still throws (dialog shows it) and leaves no card.
mock.setFail(true);
await assert.rejects(jobs.start(spec("copy", 10), "c10"));
assert.ok(!jobs.jobs.some((j) => j.title === "c10"));

// 9. keepPanels (F3 download) and now:true bypass the queue.
await jobs.start(spec("copy", 11), "c11");
await jobs.start(spec("copy", 12), "c12");
await jobs.start(spec("copy", 13), "f3", { keepPanels: true });
await tick();
assert.equal(mock.started.at(-1).spec.sources[0], "C:\\a\\13");
assert.ok(jobs.jobs.find((j) => j.title === "c12").waiting);

rmSync(tmp, { recursive: true, force: true });
console.log("queue: all OK");
process.exit(0);
