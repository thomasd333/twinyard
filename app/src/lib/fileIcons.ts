/** File-type icons: extension -> kind -> 16×16 filled SVG. */

export type IconKind =
  | "file"
  | "code"
  | "web"
  | "style"
  | "archive"
  | "image"
  | "video"
  | "audio"
  | "text"
  | "pdf"
  | "doc"
  | "sheet"
  | "slides"
  | "exe"
  | "script"
  | "config"
  | "db"
  | "font"
  | "disk";

const EXT: Record<string, IconKind> = {};
const add = (kind: IconKind, list: string) => list.split(" ").forEach((e) => (EXT[e] = kind));
add("code", "php phtml inc js mjs cjs ts tsx jsx vue svelte py rb pl java kt cs c h cpp hpp cc rs go swift dart lua r scala sql");
add("web", "html htm xhtml twig tpl blade");
add("style", "css scss sass less styl");
add("archive", "zip 7z rar tar gz tgz bz2 xz zst cab iso jar war apk");
add("image", "png jpg jpeg gif webp bmp ico svg avif heic tif tiff psd ai eps raw cr2 nef");
add("video", "mp4 mkv avi mov wmv webm m4v mpg mpeg flv 3gp ts_");
add("audio", "mp3 wav flac ogg m4a aac wma opus mid midi");
add("text", "txt md log rtf nfo srt csv_ readme");
add("pdf", "pdf");
add("doc", "doc docx odt pages");
add("sheet", "xls xlsx xlsm ods csv tsv numbers");
add("slides", "ppt pptx odp key");
add("exe", "exe msi com scr appx msix");
add("script", "bat cmd ps1 psm1 sh vbs reg");
add("config", "json xml yaml yml toml ini cfg conf config env properties htaccess lock editorconfig gitignore");
add("db", "db sqlite sqlite3 mdb accdb dbf bak");
add("font", "ttf otf woff woff2 eot fon");
add("disk", "vhd vhdx vmdk img dmg");

/* Filled, muted icons in the style of Windows/Total Commander: a grey page with a coloured
   type badge for documents and code, solid pictograms for the rest. Colours of the page and
   the folder come from CSS tokens (--icon-*, --folder-*), so they follow the theme.
   Preview of the design: docs/design/file-list-preview.html */

const PAGE = `<path fill="var(--icon-page)" d="M3 1h6.5L13 4.5V15H3z"/><path fill="var(--icon-fold)" d="M9.5 1v3.5H13z"/>`;
const LINES = `<path fill="none" stroke="var(--icon-lines)" stroke-width="1" d="M5 6.5h6M5 8.5h6M5 10.5h6M5 12.5h4"/>`;

function badge(color: string, text: string): string {
  const fs = text.length === 1 ? 6.6 : text.length === 2 ? 5.6 : 4.8;
  return (
    `<rect x="1" y="8.5" width="12.5" height="6" rx="1" fill="${color}"/>` +
    `<text x="7.25" y="13.3" fill="#fff" font-family="Segoe UI,Arial" font-weight="700" font-size="${fs}" text-anchor="middle" letter-spacing="-.2">${text}</text>`
  );
}

/** Badge colour + fixed label; a null label means "show the extension" (max 3 letters). */
const BADGE: Partial<Record<IconKind, [string, string | null]>> = {
  pdf: ["#b8463c", "PDF"],
  doc: ["#3e6fb6", "W"],
  sheet: ["#2f7d4f", "X"],
  slides: ["#c25a2e", "P"],
  font: ["#7f6f62", "Aa"],
  code: ["#4c79ad", null],
  web: ["#c0703a", null],
  style: ["#7b62ad", null],
  config: ["#8a7a52", null],
  script: ["#6b7d3a", null],
};

const SHAPES: Partial<Record<IconKind, string>> = {
  image: `<rect x="1" y="2.5" width="14" height="11" rx="1" fill="#d6dcd2"/><path fill="#5e9461" d="M1 13.5V11l4-4 3.5 3.5 2-2L15 13v.5z"/><circle cx="11" cy="5.5" r="1.4" fill="#d1a83f"/>`,
  video: `<rect x="1" y="3" width="14" height="10" rx="1.5" fill="#76619e"/><path fill="#fff" d="M6.5 5.5v5l4-2.5z"/>`,
  audio: `<path fill="#b0668a" d="M6 3.5 13.5 2v9a2 2 0 1 1-1.5-1.94V5L7.5 6v6.5a2 2 0 1 1-1.5-1.94z"/>`,
  archive: `<rect x="1.5" y="4" width="13" height="10.5" rx="1" fill="#a9844a"/><rect x="1" y="1.5" width="14" height="3.5" rx="1" fill="#c9a466"/><path fill="#5a4320" d="M7 5h2v1H7zM7 7h2v1H7zM7 9h2v1H7zM6.5 11h3v2h-3z"/>`,
  exe: `<rect x="1" y="2" width="14" height="12" rx="1.2" fill="#d4d7dc"/><path fill="#4a76b3" d="M1 3.2C1 2.5 1.5 2 2.2 2h11.6c.7 0 1.2.5 1.2 1.2V5H1z"/>`,
  db: `<path fill="#a98d3f" d="M2.5 3.5v9c0 1.2 2.5 2 5.5 2s5.5-.8 5.5-2v-9z"/><ellipse cx="8" cy="3.5" rx="5.5" ry="2" fill="#d4b763"/><path fill="none" stroke="#d4b763" stroke-width=".8" d="M2.5 8c0 1.2 2.5 2 5.5 2s5.5-.8 5.5-2"/>`,
  disk: `<rect x="1.5" y="4" width="13" height="8" rx="1.5" fill="#8a929c"/><path fill="#cfd5db" d="M3.5 9h3v1h-3z"/><circle cx="12" cy="9.5" r=".8" fill="#9fe08a"/>`,
  text: PAGE + LINES,
};

const svg = (body: string) => `<svg viewBox="0 0 16 16">${body}</svg>`;

export const FOLDER_SVG = svg(
  `<path fill="var(--folder-back)" d="M1 3h5l1.5 1.5H15V14H1z"/><path fill="var(--folder-front)" d="M1 6h14v8H1z"/>`,
);

const cache = new Map<string, string>();

/** Complete `<svg>` markup for a file's icon (cached per extension). */
export function fileIconSvg(name: string): string {
  const dot = name.lastIndexOf(".");
  const ext = (dot >= 0 ? name.slice(dot + 1) : "").toLowerCase();
  let out = cache.get(ext);
  if (out) return out;
  const kind = EXT[ext] ?? "file";
  const b = BADGE[kind];
  if (b) {
    const label = b[1] ?? ext.slice(0, 3).toUpperCase().replace(/[^0-9A-Z]/g, "");
    out = svg(PAGE + badge(b[0], label));
  } else {
    out = svg(SHAPES[kind] ?? PAGE);
  }
  cache.set(ext, out);
  return out;
}
