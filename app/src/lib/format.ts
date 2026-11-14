import { getLang } from "./i18n.svelte";

const pad = (n: number) => (n < 10 ? "0" + n : "" + n);

/** dd.mm.yyyy hh:mm — hand-rolled, much cheaper than Intl for many rows. */
export function formatDate(ms: number): string {
  const d = new Date(ms);
  return `${pad(d.getDate())}.${pad(d.getMonth() + 1)}.${d.getFullYear()} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 12 345 678 (thin-space grouped bytes, like TC). */
export function formatBytes(n: number): string {
  return n.toString().replace(/\B(?=(\d{3})+(?!\d))/g, " ");
}

/** Short human size for summaries: 1,4 GB (1.4 GB in English). */
export function formatHuman(n: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return (i === 0 ? v.toString() : getLang() === "pl" ? v.toFixed(1).replace(".", ",") : v.toFixed(1)) + " " + units[i];
}

/** Splits "archive.tar.gz" -> ["archive.tar", "gz"]; dotfiles keep their name. */
export function splitExt(name: string): [string, string] {
  const i = name.lastIndexOf(".");
  if (i <= 0) return [name, ""];
  return [name.slice(0, i), name.slice(i + 1)];
}
