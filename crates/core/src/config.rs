//! User configuration: `<data dir>\config.toml` (portable, see `data_dir`).

use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub tools: Tools,
    pub search: SearchCfg,
    pub colors: Colors,
}

/// Interface colour overrides: CSS token name (without `--`) -> CSS colour.
/// Defaults live in the app's CSS; only what is set here replaces them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Colors {
    pub light: std::collections::BTreeMap<String, String>,
    pub dark: std::collections::BTreeMap<String, String>,
}

/// Appends the (commented) colour template to config.toml when the file has
/// no colour section yet. Append-only: the user's text is never rewritten.
pub fn ensure_colors_section(template: &str) -> io::Result<()> {
    use std::io::Write;
    let path = config_path();
    let text = std::fs::read_to_string(&path)?;
    if text.contains("[colors") {
        return Ok(());
    }
    let mut f = std::fs::OpenOptions::new().append(true).open(&path)?;
    let sep = if text.ends_with('\n') { "\n" } else { "\n\n" };
    f.write_all(format!("{sep}{template}").as_bytes())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchCfg {
    /// Extensions whose content Alt+F7 searches; other files: names only.
    pub text_ext: Vec<String>,
}

impl Default for SearchCfg {
    fn default() -> Self {
        SearchCfg { text_ext: crate::search::DEFAULT_TEXT_EXT.iter().map(|s| s.to_string()).collect() }
    }
}

/// Command templates. `%f` = file, `%l` / `%r` = left / right file.
/// Quote paths containing spaces: `'"C:\Program Files\X\x.exe" "%f"'`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Tools {
    pub editor: String,
    pub viewer: String,
    pub diff: String,
}

/// Portable: all data (config, favorites, connections, passwords, logs,
/// WebView state) lives in `data\` next to twinyard.exe. A dev build
/// (`…\target\debug\twinyard.exe`) uses `<repo>\data\` so `cargo clean`
/// can't wipe it. Falls back to `%APPDATA%\filecmd` (the pre-portable location) only when the exe's
/// folder is read-only. `TWINYARD_DATA` overrides (tests).
pub fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("TWINYARD_DATA") {
        return PathBuf::from(d);
    }
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let legacy = legacy_dir();
        let Some(mut base) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) else {
            return legacy;
        };
        let lower = base.to_string_lossy().to_lowercase();
        if lower.ends_with(r"\target\debug") || lower.ends_with(r"\target\release") {
            base.pop();
            base.pop();
        }
        let dir = base.join("data");
        let fresh = !dir.exists();
        if std::fs::create_dir_all(&dir).is_err() || !writable(&dir) {
            return legacy;
        }
        if fresh {
            migrate(&legacy, &dir);
        }
        dir
    })
    .clone()
}

/// Where versions before the portable layout kept their data.
pub fn legacy_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("filecmd")
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".write-test");
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// One-time copy of data from the old `%APPDATA%\filecmd` location (the app's former name).
fn migrate(from: &Path, to: &Path) {
    for name in ["config.toml", "favorites.json", "connections.json"] {
        let (a, b) = (from.join(name), to.join(name));
        if a.exists() && !b.exists() {
            let _ = std::fs::copy(&a, &b);
        }
    }
}

pub fn config_path() -> PathBuf {
    data_dir().join("config.toml")
}

fn first_existing(candidates: &[(&str, &str)]) -> Option<String> {
    candidates.iter().find(|(exe, _)| Path::new(&expand_env(exe)).exists()).map(|(exe, args)| format!("\"{}\" {args}", expand_env(exe)))
}

fn expand_env(s: &str) -> String {
    let mut out = s.to_string();
    for var in ["LOCALAPPDATA", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(v) = std::env::var(var) {
            out = out.replace(&format!("%{var}%"), &v);
        }
    }
    out
}

/// Sensible defaults based on what is installed.
pub fn detect_tools() -> Tools {
    let editor = first_existing(&[
        (r"%ProgramFiles%\Notepad++\notepad++.exe", "\"%f\""),
        (r"%ProgramFiles(x86)%\Notepad++\notepad++.exe", "\"%f\""),
        (r"%ProgramFiles%\Microsoft VS Code\Code.exe", "\"%f\""),
        (r"%LOCALAPPDATA%\Programs\Microsoft VS Code\Code.exe", "\"%f\""),
    ])
    .unwrap_or_else(|| "notepad.exe \"%f\"".into());
    let diff = first_existing(&[
        (r"%ProgramFiles%\WinMerge\WinMergeU.exe", "/e /u \"%l\" \"%r\""),
        (r"%ProgramFiles(x86)%\WinMerge\WinMergeU.exe", "/e /u \"%l\" \"%r\""),
        (r"%ProgramFiles%\Beyond Compare 5\BCompare.exe", "\"%l\" \"%r\""),
        (r"%ProgramFiles%\Beyond Compare 4\BCompare.exe", "\"%l\" \"%r\""),
        (r"%ProgramFiles%\Meld\Meld.exe", "\"%l\" \"%r\""),
    ])
    .unwrap_or_else(|| BUILTIN.into());
    Tools { viewer: BUILTIN.into(), editor, diff }
}

/// Tool value meaning "use twinyard's own viewer / diff".
pub const BUILTIN: &str = "builtin";

fn render(cfg: &Config) -> String {
    let q = |s: &str| format!("'{s}'"); // TOML literal string: backslashes stay as-is
    if crate::i18n::english() {
        return format!(
            "# twinyard — configuration. Changes apply at once (the file is read on every use).\n\
             # Command templates: %f = file, %l / %r = left / right file to compare.\n\
             # Paths with spaces in double quotes, the whole value in apostrophes, e.g.:\n\
             #   editor = '\"C:\\Program Files\\Notepad++\\notepad++.exe\" \"%f\"'\n\
             #   diff   = '\"C:\\Program Files\\WinMerge\\WinMergeU.exe\" /e /u \"%l\" \"%r\"'\n\
             \n\
             [tools]\n\
             # F4\n\
             editor = {}\n\
             # F3 — 'builtin' (built-in viewer) or a program with \"%f\"\n\
             viewer = {}\n\
             # Compare files (Ctrl+F2) — 'builtin' (built-in diff), WinMerge, Beyond Compare…\n\
             # VS Code opens the comparison in an existing window; new window: add --new-window before --diff\n\
             diff = {}\n\
             \n\
             [search]\n\
             # Text search (Alt+F7) only in files with these extensions (without the dot);\n\
             # the others (videos, archives, databases…) are searched by name only.\n\
             text_ext = [{}]\n",
            q(&cfg.tools.editor),
            q(&cfg.tools.viewer),
            q(&cfg.tools.diff),
            cfg.search.text_ext.iter().map(|e| format!("\"{e}\"")).collect::<Vec<_>>().join(", "),
        );
    }
    format!(
        "# twinyard — konfiguracja. Zmiany działają od razu (plik czytany przy każdym użyciu).\n\
         # Szablony poleceń: %f = plik, %l / %r = lewy / prawy plik do porównania.\n\
         # Ścieżki ze spacjami w cudzysłowie, całość w apostrofach, np.:\n\
         #   editor = '\"C:\\Program Files\\Notepad++\\notepad++.exe\" \"%f\"'\n\
         #   diff   = '\"C:\\Program Files\\WinMerge\\WinMergeU.exe\" /e /u \"%l\" \"%r\"'\n\
         \n\
         [tools]\n\
         # F4\n\
         editor = {}\n\
         # F3 — 'builtin' (wbudowany podgląd) albo program z \"%f\"\n\
         viewer = {}\n\
         # Porównaj pliki (Ctrl+F2) — 'builtin' (wbudowana porównywarka), WinMerge, Beyond Compare…\n\
         # VS Code otwiera porównanie w istniejącym oknie; nowe okno: dodaj --new-window przed --diff\n\
         diff = {}\n\
         \n\
         [search]\n\
         # Szukanie tekstu (Alt+F7) tylko w plikach o tych rozszerzeniach (bez kropki);\n\
         # pozostałe (filmy, archiwa, bazy…) są przeszukiwane tylko po nazwie.\n\
         text_ext = [{}]\n",
        q(&cfg.tools.editor),
        q(&cfg.tools.viewer),
        q(&cfg.tools.diff),
        cfg.search.text_ext.iter().map(|e| format!("\"{e}\"")).collect::<Vec<_>>().join(", "),
    )
}

/// config.toml text -> Config. Notepad may save with a BOM: not an error.
fn parse(text: &str) -> Result<Config, toml::de::Error> {
    toml::from_str(text.trim_start_matches('\u{feff}'))
}

/// Loads the config, creating it with detected defaults on first run.
pub fn load() -> io::Result<Config> {
    let path = config_path();
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text).map_err(|e| io::Error::other(format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let cfg = Config { tools: detect_tools(), ..Default::default() };
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, render(&cfg))?;
            Ok(cfg)
        }
        Err(e) => Err(e),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Favorite {
    pub name: String,
    pub path: String,
}

fn favorites_path() -> PathBuf {
    config_path().with_file_name("favorites.json")
}

pub fn load_favorites() -> io::Result<Vec<Favorite>> {
    match std::fs::read_to_string(favorites_path()) {
        Ok(t) => serde_json::from_str(t.trim_start_matches('\u{feff}')).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e),
    }
}

/// Atomic save: write a temp file, then rename over the old one.
pub fn save_favorites(list: &[Favorite]) -> io::Result<()> {
    let path = favorites_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(list).map_err(io::Error::other)?)?;
    crate::win::move_path(&tmp, &path, true)
}

/// Splits a command line into tokens; `"..."` groups, quotes are removed.
pub fn split_command(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has = false;
    for c in cmd.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

/// Starts a tool from a template; placeholders are substituted per token,
/// so paths with spaces stay single arguments. No shell involved.
pub fn launch(template: &str, vars: &[(&str, &str)]) -> io::Result<()> {
    let mut tokens = split_command(template);
    if tokens.is_empty() {
        return Err(io::Error::other(crate::i18n::tr("Narzędzie nie jest skonfigurowane (config.toml)", "The tool is not configured (config.toml)")));
    }
    let uses_vars = tokens.iter().any(|t| vars.iter().any(|(k, _)| t.contains(k)));
    for t in &mut tokens {
        for (k, v) in vars {
            *t = t.replace(k, v);
        }
    }
    if !uses_vars {
        // Template is just a program: append the files.
        tokens.extend(vars.iter().map(|(_, v)| v.to_string()));
    }
    Command::new(&tokens[0])
        .args(&tokens[1..])
        .spawn()
        .map(|_| ())
        .map_err(|e| io::Error::other(format!("{}: {e}", tokens[0])))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_overrides() {
        let cfg = parse("[tools]\neditor = 'x'\n\n[colors.light]\n# bg = \"#fff\"\n\n[colors.dark]\nlist-bg = \"#202020\"\n").unwrap();
        assert!(cfg.colors.light.is_empty());
        assert_eq!(cfg.colors.dark.get("list-bg").map(String::as_str), Some("#202020"));
        // Older files without the section still load.
        assert!(parse("[tools]\neditor = 'x'\n").unwrap().colors.dark.is_empty());
    }

    #[test]
    fn splits_quoted() {
        assert_eq!(
            split_command(r#""C:\Program Files\X\x.exe" /e "%l" "%r""#),
            [r"C:\Program Files\X\x.exe", "/e", "%l", "%r"]
        );
        assert_eq!(split_command("notepad.exe"), ["notepad.exe"]);
        assert_eq!(split_command(r#"a "" b"#), ["a", "", "b"]);
    }

    #[test]
    fn rendered_default_parses_back() {
        let cfg = Config {
            tools: Tools {
                editor: r#""C:\Program Files\Microsoft VS Code\Code.exe" "%f""#.into(),
                viewer: "notepad.exe \"%f\"".into(),
                diff: String::new(),
            },
            ..Default::default()
        };
        let back: Config = toml::from_str(&render(&cfg)).unwrap();
        assert_eq!(back.search.text_ext, cfg.search.text_ext);
        // A BOM (Notepad) is fine.
        assert_eq!(parse(&format!("\u{feff}{}", render(&cfg))).unwrap().tools.editor, cfg.tools.editor);
        // Old config files without [search] get the defaults.
        let old: Config = toml::from_str("[tools]\neditor = 'x'\n").unwrap();
        assert!(old.search.text_ext.iter().any(|e| e == "php"));
        assert_eq!(back.tools.editor, cfg.tools.editor);
        assert_eq!(back.tools.viewer, cfg.tools.viewer);
        assert_eq!(back.tools.diff, "");
    }
}
