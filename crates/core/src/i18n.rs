//! Interface language of backend messages (errors, job notes): Polish or
//! English. The UI owns the setting (`data\state.json`, key `lang`) and tells
//! the backend through the `set_lang` command at start-up and on change.
//!
//! Messages are written in place, both versions side by side:
//! `tr("Pusta ścieżka", "Empty path")`, `trf!("Już istnieje: {p}", "Already exists: {p}")`.
//! Texts of the OS / the FTP server are passed through as they come.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(true);

/// "pl" = Polish, anything else = English (the default).
pub fn set_lang(code: &str) {
    ENGLISH.store(!code.eq_ignore_ascii_case("pl"), Ordering::Relaxed);
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// Picks the text in the current language.
pub fn tr(pl: &'static str, en: &'static str) -> &'static str {
    if english() { en } else { pl }
}

/// `format!` in the current language: `trf!("pl {x}", "en {x}")`, optional
/// positional arguments after the two templates.
#[macro_export]
macro_rules! trf {
    ($pl:literal, $en:literal $(, $($arg:tt)*)?) => {
        if $crate::i18n::english() { format!($en $(, $($arg)*)?) } else { format!($pl $(, $($arg)*)?) }
    };
}
