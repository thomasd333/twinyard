/**
 * UI state (tabs, theme, toolbar…) in `data\state.json` via the backend.
 * WebView2's localStorage is flushed to disk lazily and lost when the window
 * closes right after a change — a plain file written at once is reliable.
 */
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

let cache: Record<string, string> = {};
let timer: ReturnType<typeof setTimeout> | undefined;
let dirty = false;
/** Asked before the window closes; false = stay open (e.g. jobs are running). */
let closeGuard: (() => boolean) | null = null;

export function setCloseGuard(guard: () => boolean) {
  closeGuard = guard;
}

/** Call once before mounting the app. */
export async function loadUiState() {
  try {
    cache = await invoke<Record<string, string>>("load_state");
  } catch {
    cache = {};
  }
  // Closing the window (button, Alt+F4) writes pending changes first.
  // (One handler only: each onCloseRequested listener closes the window by
  // itself unless *it* called preventDefault.)
  void getCurrentWindow().onCloseRequested(async (e) => {
    if (closeGuard && !closeGuard()) return e.preventDefault();
    if (dirty) await flush();
  });
}

export async function flush() {
  clearTimeout(timer);
  dirty = false;
  try {
    await invoke("save_state", { state: cache });
  } catch {
    dirty = true;
  }
}

export const store = {
  get(key: string): string | null {
    return cache[key] ?? null;
  },
  set(key: string, value: string) {
    if (cache[key] === value) return;
    cache[key] = value;
    dirty = true;
    clearTimeout(timer);
    timer = setTimeout(() => void flush(), 300);
  },
};
