import { SvelteMap } from "svelte/reactivity";
import { api, inArchive, isRemote } from "./api";

/**
 * Explorer's own icons for files that carry one (programs, shortcuts, icon
 * files). Fetched lazily, only for rows the panel draws, in small batches;
 * cached by path + modification time. Everything else keeps the built-in icons.
 */
const OWN_ICON = /\.(exe|lnk|ico|url|scr|cpl|msc)$/i;
const MAX_CACHE = 4000;

const icons = new SvelteMap<string, string | null>();
const pending = new Map<string, string>(); // key -> path, waiting for the next batch
const inFlight = new Set<string>();
let timer: ReturnType<typeof setTimeout> | null = null;

export function hasOwnIcon(name: string): boolean {
  return OWN_ICON.test(name);
}

/**
 * Data URL of the icon, or null (none / not loaded yet — draw the built-in one).
 * Safe to call while rendering: the request goes out later.
 */
export function shellIcon(path: string, mtime: number): string | null {
  if (isRemote(path) || inArchive(path) || path.startsWith("::")) return null;
  const key = `${path.toLowerCase()}|${mtime}`;
  const v = icons.get(key);
  if (v !== undefined) return v;
  if (!inFlight.has(key) && !pending.has(key)) {
    pending.set(key, path);
    timer ??= setTimeout(flush, 16);
  }
  return null;
}

async function flush() {
  timer = null;
  const batch = [...pending].slice(0, 64);
  for (const [k] of batch) pending.delete(k);
  if (pending.size) timer = setTimeout(flush, 0);
  if (!batch.length) return;
  batch.forEach(([k]) => inFlight.add(k));
  const size = Math.min(64, Math.round(16 * (window.devicePixelRatio || 1)));
  let res: (string | null)[] = [];
  try {
    res = await api.fileIcons(batch.map(([, p]) => p), size);
  } catch {
    // leave them as null: the built-in icons stay
  }
  if (icons.size > MAX_CACHE) icons.clear();
  batch.forEach(([k], i) => {
    inFlight.delete(k);
    icons.set(k, res[i] ?? null);
  });
}
