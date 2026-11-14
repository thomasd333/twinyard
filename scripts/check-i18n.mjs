// Checks the English dictionary against the code:
//   node scripts/check-i18n.mjs
// Lists texts used in t("…") / tn(n, "…") / tool("…") without an English entry
// (they would show in Polish) and entries nothing uses any more.
// Exit code 1 when something is missing.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const src = join(dirname(fileURLToPath(import.meta.url)), "..", "app", "src");
const dictFile = join(src, "lib", "i18n.en.ts");

function walk(dir) {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? walk(p) : /\.(svelte|ts)$/.test(n) ? [p] : [];
  });
}

// A double-quoted JS string literal -> its value.
const value = (lit) => JSON.parse(`"${lit}"`);
const STRING = String.raw`"((?:[^"\\]|\\.)*)"`;

const used = new Map(); // text -> first file
for (const file of walk(src)) {
  if (file === dictFile) continue;
  const text = readFileSync(file, "utf8");
  for (const m of text.matchAll(new RegExp(String.raw`\b(?:(?:t|tool)\(\s*|tn\([^,"]+,\s*)` + STRING, "g"))) {
    const key = value(m[1]);
    if (!used.has(key)) used.set(key, file.slice(src.length + 1));
  }
}

const dict = new Set();
for (const m of readFileSync(dictFile, "utf8").matchAll(new RegExp(String.raw`^\s*` + STRING + String.raw`\s*:`, "gm"))) dict.add(value(m[1]));

const missing = [...used].filter(([k]) => !dict.has(k));
const unused = [...dict].filter((k) => !used.has(k));
for (const [k, f] of missing) console.log(`MISSING  ${f}: ${JSON.stringify(k)}`);
for (const k of unused) console.log(`unused   ${JSON.stringify(k)}`);
console.log(`${used.size} texts, ${missing.length} missing, ${unused.length} unused`);
process.exit(missing.length ? 1 : 0);
