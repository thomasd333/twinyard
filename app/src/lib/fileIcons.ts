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

/* Filled icons in the style of Windows/Total Commander: a grey page with a full-width coloured
   type badge for documents and code, solid pictograms for the rest. Badge labels use a 3×5
   pixel font so they stay legible at 16 px. Colours of the page and the folder come from CSS
   tokens (--icon-*, --folder-*), so they follow the theme.
   Previews of the design: docs/design/file-list-preview.html, docs/design/file-icons-preview.html (variant D) */

const PAGE = `<path fill="var(--icon-page)" d="M3 1h6.5L13 4.5V15H3z"/><path fill="var(--icon-fold)" d="M9.5 1v3.5H13z"/>`;
const LINES = `<path fill="none" stroke="var(--icon-lines)" stroke-width="1" d="M5 6.5h6M5 8.5h6M5 10.5h6M5 12.5h4"/>`;

/** 3×5 pixel glyphs, rows top to bottom, "1" = lit. */
const PX: Record<string, string> = {
  A: "010101111101101", B: "110101110101110", C: "011100100100011", D: "110101101101110",
  E: "111100110100111", F: "111100110100100", G: "011100101101011", H: "101101111101101",
  I: "111010010010111", J: "001001001101010", K: "101101110101101", L: "100100100100111",
  M: "101111111101101", N: "110101101101101", O: "010101101101010", P: "110101110100100",
  Q: "010101101110011", R: "110101110101101", S: "011100010001110", T: "111010010010010",
  U: "101101101101111", V: "101101101101010", W: "101101111111101", X: "101101010101101",
  Y: "101101010010010", Z: "111001010100111", "0": "111101101101111", "1": "010110010010111",
  "2": "110001010100111", "3": "110001010001110", "4": "101101111001001", "5": "111100110001110",
  "6": "011100111101111", "7": "111001010010010", "8": "111101111101111", "9": "111101111001110",
};

function pixelText(text: string, fill: string): string {
  let x = Math.floor((16 - (text.length * 4 - 1)) / 2);
  let d = "";
  for (const ch of text) {
    const g = PX[ch];
    for (let i = 0; i < 15; i++) if (g[i] === "1") d += `M${x + (i % 3)} ${10 + Math.floor(i / 3)}h1v1h-1z`;
    x += 4;
  }
  return `<path fill="${fill}" shape-rendering="crispEdges" d="${d}"/>`;
}

function badge(color: string, fg: string, text: string): string {
  const bg = `<rect y="8" width="16" height="8" rx="1" fill="${color}"/>`;
  // single letters (W/X/P) and "Aa" read better as real text than as 3×5 pixels
  if (text.length === 1 || !/^[0-9A-Z]+$/.test(text))
    return (
      bg +
      `<text x="8" y="12.2" fill="${fg}" font-family="Segoe UI,Arial" font-weight="700" font-size="8" text-anchor="middle" dominant-baseline="central">${text}</text>`
    );
  return bg + pixelText(text, fg);
}

/** Badge colour + fixed label; a null label means "show the extension" (max 3 letters). */
const BADGE: Partial<Record<IconKind, [string, string | null]>> = {
  pdf: ["#c8291c", "PDF"],
  doc: ["#2b5fbf", "W"],
  sheet: ["#1e7d43", "X"],
  slides: ["#cf4a17", "P"],
  font: ["#6f5f52", "Aa"],
  code: ["#3d6fb0", null],
  web: ["#d9531e", null],
  style: ["#2a7bd6", null],
  config: ["#8b6b2e", null],
  script: ["#4f7a1c", null],
};

/** Per-extension badge colour (overrides the kind's), optional text colour. */
const EXT_COLOR: Record<string, [string, string?]> = {};
const color = (c: string, list: string, fg?: string) => list.split(" ").forEach((e) => (EXT_COLOR[e] = [c, fg]));
color("#5b5fc7", "php phtml inc");
color("#f0d33a", "js mjs cjs jsx", "#2a2a2a");
color("#2f74c0", "ts tsx");
color("#2f6aa8", "py");
color("#c2185b", "rb");
color("#b56a13", "java kt");
color("#6a2a8f", "cs");
color("#56687a", "c h");
color("#c8336a", "cpp hpp cc");
color("#b7410e", "rs");
color("#0a8fb3", "go");
color("#e8430d", "svelte");
color("#2f9a6b", "vue");
color("#c26b0a", "sql");
color("#d9531e", "html htm xhtml");
color("#2a7bd6", "css");
color("#c2457f", "scss sass");
color("#a47a00", "json");
color("#0f8a80", "xml");
color("#8b6b2e", "yaml yml toml ini cfg conf env lock");
color("#2c5aa0", "ps1 psm1");
color("#4f7a1c", "bat cmd sh");

/** Whole-icon pictograms chosen by extension (IDE project files). */
const VS = `<path fill="#865fc5" d="M11 1 15 3v10l-4 2-6.5-5.5L2 11.5 1 11V5l1-.5 2.5 2z"/><path fill="#5c2d91" d="M11 1v14l-6.5-5.5L11 4zM2 6.3 4 8l-2 1.7z"/>`;
const GLYPH: Record<string, string> = {
  "code-workspace": `<path fill="none" stroke="#1f9cf0" stroke-width="2.6" stroke-linecap="round" d="M12.4 2.6 3.2 10.3M12.4 13.4 3.2 5.7"/><rect x="11.2" y="1" width="3.8" height="14" rx="1" fill="#0065a9"/>`,
  sln: VS,
  csproj: VS,
  vcxproj: VS,
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
  if (GLYPH[ext]) {
    out = svg(GLYPH[ext]);
  } else if (b) {
    const label = b[1] ?? ext.slice(0, 3).toUpperCase().replace(/[^0-9A-Z]/g, "");
    const [c, fg] = EXT_COLOR[ext] ?? [b[0]];
    out = svg(PAGE + badge(c, fg ?? "#fff", label));
  } else {
    out = svg(SHAPES[kind] ?? PAGE);
  }
  cache.set(ext, out);
  return out;
}
