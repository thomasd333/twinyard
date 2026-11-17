import type { ColorOverrides } from "./userColors";
import { invoke } from "@tauri-apps/api/core";
import { t } from "./i18n.svelte";

export interface Listing {
  path: string;
  names: string[];
  sizes: number[];
  mtimes: number[];
  attrs: number[];
  nameRank: number[];
  errors: number;
  elapsedMs: number;
}

export interface Drive {
  root: string;
  letter: string;
  kind: "fixed" | "removable" | "network" | "cdrom" | "ramdisk" | "unknown";
}

export interface TextFile {
  text: string;
  encoding: string;
  binary: boolean;
  truncated: boolean;
  size: number;
}

export interface SearchQuery {
  root: string;
  mask: string;
  text: string;
  subdirs: boolean;
  caseSensitive: boolean;
  skipNoise: boolean;
}

export interface SearchHit {
  path: string;
  dir: boolean;
  size: number;
  mtime: number;
  line: number | null;
  snippet: string | null;
}

export interface SearchBatch {
  id: number;
  hits: SearchHit[];
  stats: { dirs: number; files: number; read: number; hits: number; current: string };
  done: boolean;
}

export interface Favorite {
  name: string;
  path: string;
}

export type OpKind = "copy" | "move" | "trash" | "delete" | "pack";

export interface OpSpec {
  kind: OpKind;
  sources: string[];
  dest?: string;
}

export interface Progress {
  phase: "scan" | "run";
  totalBytes: number;
  doneBytes: number;
  totalFiles: number;
  doneFiles: number;
  current: string;
}

export interface Conflict {
  source: string;
  target: string;
  sourceSize: number;
  sourceMtime: number;
  targetSize: number;
  targetMtime: number;
  /** Target is a shorter, partial copy that can be continued (FTP transfers). */
  canResume: boolean;
}

export interface OpError {
  path: string;
  message: string;
}

export interface Outcome {
  cancelled: boolean;
  doneFiles: number;
  skipped: number;
  errors: OpError[];
}

export type ConflictAction = "overwrite" | "overwriteOlder" | "skip" | "rename" | "resume" | "cancel";
export type ErrorAction = "retry" | "skip" | "skipAll" | "cancel";

export type Ask =
  | { kind: "conflict"; id: number; conflict: Conflict }
  | { kind: "error"; id: number; error: OpError }
  | { kind: "password"; id: number; archive: string; wrong: boolean };

// ---------- encrypted archives ----------
// The backend fails with "Wymagane|Złe hasło do archiwum: <path>"; every
// call then asks for the password (via the UI's prompt) and retries.
// This text is a protocol: it stays Polish whatever the interface language.
const PASSWORD_RE = /(Wymagane|Złe) hasło do archiwum: (.+)$/;

/** Error text for display: the password protocol message in the UI language. */
export function errorText(e: unknown): string {
  return String(e).replace(/(Wymagane|Złe) hasło do archiwum: /, (_, w) => (w === "Złe" ? t("Złe hasło do archiwum:") : t("Wymagane hasło do archiwum:")) + " ");
}
type PasswordPrompt = (archive: string, wrong: boolean) => Promise<string | null>;
let passwordPrompt: PasswordPrompt | null = null;

export function setPasswordPrompt(p: PasswordPrompt) {
  passwordPrompt = p;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  for (;;) {
    try {
      return await invoke<T>(cmd, args);
    } catch (e) {
      const m = PASSWORD_RE.exec(String(e));
      if (!m || !passwordPrompt) throw e;
      const archive = m[2].trim();
      const pw = await passwordPrompt(archive, m[1] === "Złe");
      if (pw === null) throw errorText(e);
      await invoke("archive_password", { archive, password: pw });
    }
  }
}

export const ATTR_DIR = 0x10;
export const ATTR_HIDDEN = 0x02;
export const ATTR_SYSTEM = 0x04;

export const api = {
  listDir: (path: string) => call<Listing>("list_dir", { path }),
  parentDir: (path: string) => call<string | null>("parent_dir", { path }),
  listDrives: () => call<Drive[]>("list_drives"),
  homeDir: () => call<string>("home_dir"),
  openPath: (path: string) => call<void>("open_path", { path }),
  fakeListing: (count: number) => call<Listing>("debug_fake_listing", { count }),
  createDir: (path: string) => call<void>("create_dir", { path }),
  createFile: (path: string) => call<boolean>("create_file", { path }),
  resolvePath: (input: string, base: string) => call<{ dir: string; focus: string | null }>("resolve_path", { input, base }),
  renamePath:(from: string, to: string) => call<void>("rename_path", { from, to }),
  /** false = tool is 'builtin' (nothing launched; show our own viewer/diff). */
  runTool: (tool: "editor" | "viewer" | "diff", files: string[]) => call<boolean>("run_tool", { tool, files }),
  readText: (path: string, limit: number) => call<TextFile>("read_text", { path, limit }),
  searchStart: (query: SearchQuery) => call<number>("search_start", { query }),
  searchCancel: (id: number) => call<void>("search_cancel", { id }),
  shellContextMenu: (paths: string[]) => call<void>("shell_context_menu", { paths }),
  dragOut: (paths: string[], mode: "copy" | "move") => call<void>("drag_out", { paths, mode }),
  openTerminal: (path: string) => call<void>("open_terminal", { path }),
  openConfig: (colorsTemplate?: string) => call<void>("open_config", { colorsTemplate }),
  getColors: () => call<ColorOverrides>("get_colors"),
  getFavorites: () => call<Favorite[]>("get_favorites"),
  setFavorites: (list: Favorite[]) => call<void>("set_favorites", { list }),
  /** `local` = copy already downloaded (big file, as a job): only watch it. */
  editRemote: (path: string, local?: string) => call<string>("edit_remote", { path, local: local ?? null }),
  remoteTempPath: (path: string) => call<string>("remote_temp_path", { path }),
  diskSpace: (path: string) => call<{ free: number; total: number } | null>("disk_space", { path }),
  dirSize: (path: string) => call<{ bytes: number; files: number; dirs: number; partial: boolean }>("dir_size", { path }),
  dirSizeCancel: () => call<void>("dir_size_cancel"),
  /** Counted so far by the running dirSize. */
  dirSizeProgress: () => call<{ bytes: number; files: number; dirs: number }>("dir_size_progress"),
  /** Explorer's icons of local files (PNG data URLs; null = none). `size` in device pixels. */
  fileIcons: (paths: string[], size: number) => call<(string | null)[]>("file_icons", { paths, size }),
  clipboardSetFiles: (paths: string[], cut: boolean) => call<void>("clipboard_set_files", { paths, cut }),
  clipboardSetText: (text: string) => call<void>("clipboard_set_text", { text }),
  clipboardGetFiles: () => call<{ paths: string[]; cut: boolean } | null>("clipboard_get_files"),
  clipboardClear: () => call<void>("clipboard_clear"),
  editNewRemote: (path: string) => call<string>("edit_new_remote", { path }),
  editSetAuto: (local: string, auto: boolean) => call<void>("edit_set_auto", { local, auto }),
  editUpload: (local: string, remotePath: string) => call<void>("edit_upload", { local, remotePath }),
  watchDirs: (paths: string[]) => call<void>("watch_dirs", { paths }),
  startJob: (spec: OpSpec) => call<number>("start_job", { spec }),
  jobAnswer: (id: number, answer: unknown) => call<void>("job_answer", { id, answer }),
  jobCancel: (id: number) => call<void>("job_cancel", { id }),
};

export function isRemote(path: string): boolean {
  return /^ftp:\/\//i.test(path);
}

/** Archive formats browsed like folders (ZIP read/write; 7z, RAR, .tar, .tar.gz/.tgz, .gz read-only). */
export const ARCHIVE_RE = /\.(zip|7z|rar|tar|tgz|gz)$/i;

/** A panel path inside an archive (its root included), e.g. C:\x\a.zip\dir. */
export function inArchive(path: string): boolean {
  return !isRemote(path) && /\.(zip|7z|rar|tar|tgz|gz)(\\|$)/i.test(path);
}

/** In (or at) an archive we can only read from: 7z, RAR, tar, gz — everything but ZIP. */
export function inReadOnlyArchive(path: string): boolean {
  if (isRemote(path)) return false;
  const m = /\.(zip|7z|rar|tar|tgz|gz)(\\|$)/i.exec(path);
  return !!m && m[1].toLowerCase() !== "zip";
}

/** A file inside a ZIP (the only archive format we write back to). */
export function inZip(path: string): boolean {
  return !isRemote(path) && /\.zip\\/i.test(path);
}

export function joinPath(dir: string, name: string): string {
  const sep = isRemote(dir) ? "/" : "\\";
  return dir.endsWith(sep) ? dir + name : dir + sep + name;
}

export function baseName(path: string): string {
  if (isRemote(path)) {
    const rest = path.slice(6).replace(/\/+$/, "");
    const i = rest.lastIndexOf("/");
    return i >= 0 ? rest.slice(i + 1) : rest; // server root -> connection id
  }
  const p = path.replace(/\\+$/, "");
  const i = p.lastIndexOf("\\");
  return i >= 0 ? p.slice(i + 1) : p;
}

/** Connection id of an `ftp://id/…` path. */
export function remoteId(path: string): string | null {
  return isRemote(path) ? path.slice(6).split("/")[0] : null;
}

// ---------- FTP ----------

export type TlsMode = "none" | "explicit" | "implicit";

export interface Connection {
  id: string;
  name: string;
  host: string;
  port: number;
  tls: TlsMode;
  user: string;
  initialDir: string;
  passive: boolean;
  mlsd: boolean;
  keepalive: number;
  encoding: "auto" | "utf8" | "cp1250";
  savedPassword?: boolean;
}

export type ConnectResult =
  | { kind: "ok"; path: string }
  | { kind: "needPassword"; name: string; user: string; error: string | null }
  | { kind: "cert"; name: string; host: string; sha256: string; sha1: string; reason: string };

export interface ImportReport {
  added: string[];
  skipped: string[];
  pins: number;
}

export const ftp = {
  connections: () => call<Connection[]>("ftp_connections"),
  save: (conn: Connection, password: string | null) => call<string>("ftp_save_connection", { conn, password }),
  remove: (id: string) => call<void>("ftp_delete_connection", { id }),
  connect: (id: string, password: string | null, remember: boolean) =>
    call<ConnectResult>("ftp_connect", { id, password, remember }),
  trust: (id: string, sha256: string) => call<void>("ftp_trust", { id, sha256 }),
  forgetPassword: (id: string) => call<void>("ftp_forget_password", { id }),
  disconnect: (id: string) => call<void>("ftp_disconnect", { id }),
  tcDefaultPath: () => call<string>("ftp_tc_default_path"),
  importTc: (path: string) => call<ImportReport>("ftp_import_tc", { path }),
  openLog: () => call<void>("ftp_open_log"),
};
