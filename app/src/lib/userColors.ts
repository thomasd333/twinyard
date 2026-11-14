/**
 * Interface colour overrides from config.toml ([colors.light] / [colors.dark]).
 * The defaults stay in app.css; the overrides go into one <style> element added
 * after it (same selector weight, later = wins). The template for config.toml
 * is read from the loaded CSS itself, so it always shows the real defaults.
 */
import { t } from "./i18n.svelte";

export type ColorOverrides = { light: Record<string, string>; dark: Record<string, string> };

const NAME = /^[a-z][a-z0-9-]*$/;
/** Anything that could end the declaration / rule is refused (a typo must not break the whole UI). */
const BAD_VALUE = /[;{}<>\\]/;

/** Custom properties defined by app.css for :root (light) and [data-theme="dark"]. */
export function defaultColors(): { light: [string, string][]; dark: [string, string][] } {
  const light: [string, string][] = [];
  const dark: [string, string][] = [];
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try {
      rules = sheet.cssRules;
    } catch {
      continue;
    }
    for (const r of Array.from(rules)) {
      if (!(r instanceof CSSStyleRule)) continue;
      const sel = r.selectorText.replace(/\s|'/g, "").replace(/"/g, "");
      const target = sel === ":root" ? light : sel === ":root[data-theme=dark]" ? dark : null;
      if (!target) continue;
      for (let k = 0; k < r.style.length; k++) {
        const prop = r.style[k];
        // (lightningcss-*: the minifier's own helpers, not ours)
        if (prop.startsWith("--") && !prop.startsWith("--lightningcss") && !target.some(([n]) => n === prop.slice(2))) {
          target.push([prop.slice(2), r.style.getPropertyValue(prop).trim()]);
        }
      }
    }
  }
  return { light, dark };
}

/** Commented section appended to config.toml (by the backend, only when it has none). */
export function colorTemplate(): string {
  const d = defaultColors();
  const block = (list: [string, string][]) => list.map(([n, v]) => `# ${n} = "${v}"`).join("\n");
  return (
    `# ${t("Kolory interfejsu. Usuń # przed wierszem i zmień wartość (dowolny kolor CSS: #rrggbb, rgb(), nazwa).")}\n` +
    `# ${t("Zmiany widać po powrocie do okna twinyard. Wiersze z # = wartość domyślna.")}\n` +
    `# ${t("list-* = lista plików, icon-* / folder-* = ikony, cursor = kursor, mark = zaznaczone pliki.")}\n` +
    `[colors.light]\n${block(d.light)}\n\n[colors.dark]\n${block(d.dark)}\n`
  );
}

let styleEl: HTMLStyleElement | null = null;

/** Applies the overrides; returns the names that were refused (bad name or value). */
export function applyColors(c: ColorOverrides): string[] {
  const bad: string[] = [];
  const decls = (m: Record<string, string>) =>
    Object.entries(m ?? {})
      .filter(([n, v]) => {
        const ok = NAME.test(n) && typeof v === "string" && !BAD_VALUE.test(v) && v.trim() !== "";
        if (!ok) bad.push(n);
        return ok;
      })
      .map(([n, v]) => `--${n}: ${v.trim()};`)
      .join(" ");
  const css = `:root[data-theme="light"] { ${decls(c.light)} }\n:root[data-theme="dark"] { ${decls(c.dark)} }`;
  if (!styleEl) {
    styleEl = document.createElement("style");
    styleEl.id = "user-colors";
    document.head.appendChild(styleEl);
  }
  if (styleEl.textContent !== css) styleEl.textContent = css;
  return bad;
}
