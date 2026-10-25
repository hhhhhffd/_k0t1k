//! Own test binary: the language is process-global, so it must not run beside tests that expect English.

use k0t1k_core::i18n::{Language, set_language, tr};
use k0t1k_core::tr_fmt;

#[test]
fn follows_the_current_language() {
    assert_eq!(tr("Cancel", "Отмена"), "Cancel");
    set_language(Language::Ru);
    assert_eq!(tr("Cancel", "Отмена"), "Отмена");
    assert_eq!(tr_fmt!("{} files", "Файлов: {}", 3), "Файлов: 3");
    set_language(Language::En);
    assert_eq!(tr_fmt!("{} files", "Файлов: {}", 3), "3 files");
}
