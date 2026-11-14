/**
 * Interface language: Polish (the source texts) or English.
 *
 * Texts stay in the code in Polish and double as dictionary keys:
 * `t("Nowy katalog")`, `t("Kopiuj {0}", what)`. `i18n.en.ts` maps them to
 * English; a missing entry falls back to Polish (`node scripts/check-i18n.mjs`
 * lists those). `t()` reads reactive state, so templates and `$derived`
 * values switch language at once — no restart.
 *
 * The setting lives in `data\state.json` (key `lang`); without it the system
 * language decides. The backend is told too (its error messages).
 */
import { invoke } from "@tauri-apps/api/core";
import { EN } from "./i18n.en";
import { store } from "./uiState";

export type Lang = "pl" | "en";

export const LANGS: { id: Lang; name: string }[] = [
  { id: "pl", name: "Polski" },
  { id: "en", name: "English" },
];

let lang = $state<Lang>("en");

function apply(l: Lang) {
  lang = l;
  document.documentElement.lang = l;
  return invoke("set_lang", { lang: l }).catch(() => {});
}

/** Call once after `loadUiState()`, before mounting the app. */
export async function initLang() {
  const saved = store.get("lang");
  await apply(saved === "pl" || saved === "en" ? saved : "en");
}

export function getLang(): Lang {
  return lang;
}

export function setLang(l: Lang) {
  store.set("lang", l);
  void apply(l);
}

/** Translates `pl`; `{0}`, `{1}`… are replaced with `args`. */
export function t(pl: string, ...args: (string | number)[]): string {
  const s = lang === "en" ? (EN[pl] ?? pl) : pl;
  return args.length ? s.replace(/\{(\d)\}/g, (m, i) => (+i < args.length ? String(args[+i]) : m)) : s;
}

/**
 * Noun for a count: Polish forms "plik|pliki|plików" (1 / 2–4 / the rest),
 * English "file|files".
 */
export function tn(n: number, forms: string): string {
  const f = t(forms).split("|");
  if (n === 1) return f[0];
  if (lang === "en" || f.length < 3) return f[1] ?? f[0];
  const d = n % 10,
    dd = n % 100;
  return d >= 2 && d <= 4 && (dd < 12 || dd > 14) ? f[1] : f[2];
}
