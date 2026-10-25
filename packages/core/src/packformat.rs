//! Resource pack formats: which `pack_format` a Minecraft release uses and which range a pack declares.
//! The table mirrors the one in the packconv plugin (it does not export it); a release that is not in
//! the table (a snapshot, say) has no known format, so nothing is claimed about it.

use std::io::{Read, Seek};

use serde_json::Value;

use crate::error::Result;

/// `pack_format` as (major, minor); the minor part exists since 1.21.9.
pub type PackFormat = (u32, u32);
type Version = (u32, u32, u32);

/// Releases and their formats, oldest first (same as packconv's `RELEASES`).
const RELEASES: &[(Version, PackFormat)] = &[
    ((1, 7, 2), (1, 0)),
    ((1, 7, 10), (1, 0)),
    ((1, 8, 0), (1, 0)),
    ((1, 8, 9), (1, 0)),
    ((1, 9, 0), (2, 0)),
    ((1, 9, 4), (2, 0)),
    ((1, 10, 0), (2, 0)),
    ((1, 10, 2), (2, 0)),
    ((1, 11, 0), (3, 0)),
    ((1, 11, 2), (3, 0)),
    ((1, 12, 0), (3, 0)),
    ((1, 12, 2), (3, 0)),
    ((1, 13, 0), (4, 0)),
    ((1, 13, 2), (4, 0)),
    ((1, 14, 0), (4, 0)),
    ((1, 14, 4), (4, 0)),
    ((1, 15, 0), (5, 0)),
    ((1, 15, 2), (5, 0)),
    ((1, 16, 0), (5, 0)),
    ((1, 16, 1), (5, 0)),
    ((1, 16, 2), (6, 0)),
    ((1, 16, 5), (6, 0)),
    ((1, 17, 0), (7, 0)),
    ((1, 17, 1), (7, 0)),
    ((1, 18, 0), (8, 0)),
    ((1, 18, 2), (8, 0)),
    ((1, 19, 0), (9, 0)),
    ((1, 19, 2), (9, 0)),
    ((1, 19, 3), (12, 0)),
    ((1, 19, 4), (13, 0)),
    ((1, 20, 0), (15, 0)),
    ((1, 20, 1), (15, 0)),
    ((1, 20, 2), (18, 0)),
    ((1, 20, 3), (22, 0)),
    ((1, 20, 4), (22, 0)),
    ((1, 20, 5), (32, 0)),
    ((1, 20, 6), (32, 0)),
    ((1, 21, 0), (34, 0)),
    ((1, 21, 1), (34, 0)),
    ((1, 21, 2), (42, 0)),
    ((1, 21, 3), (42, 0)),
    ((1, 21, 4), (46, 0)),
    ((1, 21, 5), (55, 0)),
    ((1, 21, 6), (63, 0)),
    ((1, 21, 7), (64, 0)),
    ((1, 21, 8), (64, 0)),
    ((1, 21, 9), (69, 0)),
    ((1, 21, 10), (69, 0)),
    ((1, 21, 11), (75, 0)),
    ((26, 1, 0), (84, 0)),
    ((26, 1, 1), (84, 0)),
    ((26, 1, 2), (84, 0)),
    ((26, 2, 0), (88, 0)),
    ((26, 3, 0), (97, 1)),
];

fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.split('.').map(|part| part.parse().ok());
    let version = (parts.next()??, parts.next()??, parts.next().unwrap_or(Some(0))?);
    parts.next().is_none().then_some(version)
}

fn show(version: Version) -> String {
    if version.2 == 0 { format!("{}.{}", version.0, version.1) } else { format!("{}.{}.{}", version.0, version.1, version.2) }
}

/// The format of a release name like `1.20.4`; `None` for names outside the table (snapshots, very old or new).
pub fn format_of_version(name: &str) -> Option<PackFormat> {
    let version = parse_version(name)?;
    if version < RELEASES[0].0 || version > RELEASES[RELEASES.len() - 1].0 {
        return None;
    }
    RELEASES.iter().rev().find(|(release, _)| *release <= version).map(|(_, format)| *format)
}

/// A readable name of the releases a declared format range covers, like `1.20-1.20.1`.
pub fn releases_label(range: (PackFormat, PackFormat)) -> String {
    let first = RELEASES.iter().find(|(_, format)| *format >= range.0).map(|(release, _)| *release);
    let last = RELEASES.iter().rev().find(|(_, format)| *format <= range.1).map(|(release, _)| *release);
    match (first, last) {
        (Some(first), Some(last)) if first < last => format!("{}-{}", show(first), show(last)),
        (Some(first), Some(_)) => show(first),
        _ => format!("pack_format {}", range.0.0),
    }
}

fn parse_format(value: &Value) -> Option<PackFormat> {
    match value {
        Value::Number(number) => Some((u32::try_from(number.as_u64()?).ok()?, 0)),
        Value::Array(parts) => Some((u32::try_from(parts.first()?.as_u64()?).ok()?, parts.get(1).and_then(Value::as_u64).map_or(0, |minor| minor as u32))),
        _ => None,
    }
}

fn parse_range(value: &Value) -> Option<(PackFormat, PackFormat)> {
    match value {
        Value::Number(_) => parse_format(value).map(|format| (format, format)),
        Value::Array(parts) if parts.len() == 2 => Some((parse_format(&parts[0])?, parse_format(&parts[1])?)),
        Value::Object(object) => Some((parse_format(object.get("min_inclusive")?)?, parse_format(object.get("max_inclusive")?)?)),
        _ => None,
    }
}

/// The inclusive format range `pack.mcmeta` declares (`pack_format`, `supported_formats`, `min_format`/`max_format`).
pub fn declared_range(mcmeta: &[u8]) -> Option<(PackFormat, PackFormat)> {
    let text = std::str::from_utf8(mcmeta).ok()?.trim_start_matches('\u{feff}');
    let meta: Value = serde_json::from_str(text).ok()?;
    let section = meta.get("pack")?;
    let pack_format = section.get("pack_format").and_then(parse_format);
    let supported = section.get("supported_formats").and_then(parse_range);
    let min = section.get("min_format").and_then(parse_format).or(supported.map(|range| range.0)).or(pack_format)?;
    let max = section.get("max_format").and_then(parse_format).or(supported.map(|range| range.1)).or(pack_format)?;
    Some(if min <= max { (min, max) } else { (max, min) })
}

const MAX_MCMETA_BYTES: u64 = 1024 * 1024;

/// The declared range of a zip resource pack; `None` when it has no readable `pack.mcmeta`.
pub fn zip_declared_range(reader: impl Read + Seek) -> Result<Option<(PackFormat, PackFormat)>> {
    let mut archive = zip::ZipArchive::new(reader)?;
    let Ok(file) = archive.by_name("pack.mcmeta") else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.take(MAX_MCMETA_BYTES).read_to_end(&mut bytes).map_err(|error| crate::Error::Other(format!("cannot read pack.mcmeta: {error}")))?;
    Ok(declared_range(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_map_to_formats() {
        assert_eq!(format_of_version("1.20.4"), Some((22, 0)));
        assert_eq!(format_of_version("1.16.4"), Some((6, 0)));
        assert_eq!(format_of_version("1.21.4"), Some((46, 0)));
        assert_eq!(format_of_version("26.3"), Some((97, 1)));
        assert_eq!(format_of_version("1.6"), None);
        assert_eq!(format_of_version("24w10a"), None);
        assert_eq!(format_of_version("99.0"), None);
    }

    #[test]
    fn declared_ranges() {
        let range = |text: &str| declared_range(text.as_bytes());
        assert_eq!(range(r#"{"pack":{"pack_format":15}}"#), Some(((15, 0), (15, 0))));
        assert_eq!(range(r#"{"pack":{"pack_format":15,"supported_formats":[15,18]}}"#), Some(((15, 0), (18, 0))));
        assert_eq!(range(r#"{"pack":{"supported_formats":{"min_inclusive":15,"max_inclusive":22}}}"#), Some(((15, 0), (22, 0))));
        assert_eq!(range(r#"{"pack":{"min_format":[69,0],"max_format":1000}}"#), Some(((69, 0), (1000, 0))));
        assert_eq!(range("\u{feff}{\"pack\":{\"pack_format\":1}}"), Some(((1, 0), (1, 0))));
        assert_eq!(range("{}"), None);
        assert_eq!(range("nope"), None);
    }

    #[test]
    fn labels_cover_the_declared_releases() {
        assert_eq!(releases_label(((15, 0), (15, 0))), "1.20-1.20.1");
        assert_eq!(releases_label(((46, 0), (46, 0))), "1.21.4");
        assert_eq!(releases_label(((5, 0), (6, 0))), "1.15-1.16.5");
    }
}
