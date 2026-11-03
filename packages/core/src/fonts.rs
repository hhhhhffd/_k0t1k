//! Font families installed on this computer, for the interface font picker. The webview cannot list local fonts itself.

use std::collections::BTreeMap;

/// Sorted family names without duplicates (compared case-insensitively). Names starting with `.` are hidden system fonts.
pub fn list_system_families() -> Vec<String> {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut by_key = BTreeMap::new();
    for face in db.faces() {
        // the first family is the primary (English when the font has one) name
        let Some((name, _)) = face.families.first() else { continue };
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        by_key.entry(name.to_lowercase()).or_insert_with(|| name.clone());
    }
    by_key.into_values().collect()
}
