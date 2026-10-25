//! Language of the messages the user sees. The UI has its own dictionaries; this module only serves text
//! produced by Rust (errors, task titles, tray menu, dialogs). Logs and internal errors stay English.

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Language {
    #[default]
    En,
    Ru,
}

static CURRENT: AtomicU8 = AtomicU8::new(Language::En as u8);

pub fn set_language(language: Language) {
    CURRENT.store(language as u8, Ordering::Relaxed);
}

pub fn language() -> Language {
    match CURRENT.load(Ordering::Relaxed) {
        x if x == Language::Ru as u8 => Language::Ru,
        _ => Language::En,
    }
}

/// The text in the current language.
pub fn tr(en: &'static str, ru: &'static str) -> &'static str {
    match language() {
        Language::En => en,
        Language::Ru => ru,
    }
}

/// `format!` with a separate format string per language; both get the same arguments.
#[macro_export]
macro_rules! tr_fmt {
    ($en:literal, $ru:literal $(, $($args:tt)*)?) => {
        match $crate::i18n::language() {
            $crate::i18n::Language::En => format!($en $(, $($args)*)?),
            $crate::i18n::Language::Ru => format!($ru $(, $($args)*)?),
        }
    };
}

