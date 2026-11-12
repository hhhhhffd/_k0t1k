//! Strict extraction of a signed macOS update ZIP. Never executes bundle contents.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, IoContext, Result};

pub const BUNDLE_NAME: &str = "_k0t1k.app";
pub const EXECUTABLE_RELATIVE: &str = "Contents/MacOS/k0t1k-app";
const INFO_RELATIVE: &str = "Contents/Info.plist";
const MAX_ENTRIES: usize = 10_000;
const MAX_UNPACKED_BYTES: u64 = 512 * 1024 * 1024;

fn rejected(reason: impl std::fmt::Display) -> Error {
    Error::Other(format!("invalid macOS update: {reason}"))
}

/// Accepts only `*.app/Contents/MacOS/k0t1k-app`, including a renamed app bundle.
/// This is a layout check, not filesystem or code-signature validation.
pub fn bundle_from_executable(executable: &Path) -> Result<PathBuf> {
    let invalid = || rejected("the executable is not inside an app bundle");
    if executable
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(invalid());
    }
    let macos = executable.parent().ok_or_else(invalid)?;
    let contents = macos.parent().ok_or_else(invalid)?;
    let bundle = contents.parent().ok_or_else(invalid)?;
    if executable
        .file_name()
        .is_none_or(|name| name != "k0t1k-app")
        || macos.file_name().is_none_or(|name| name != "MacOS")
        || contents.file_name().is_none_or(|name| name != "Contents")
        || bundle
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| !name.ends_with(".app") || name == ".app")
    {
        return Err(invalid());
    }
    Ok(bundle.to_owned())
}

/// Extracts into an existing, empty, private directory owned by the caller.
/// The caller must keep it private and remove it on failure (partial extraction is possible).
/// All names/types are checked before creating anything; existing paths are never replaced.
pub fn extract_bundle(archive: &Path, destination: &Path) -> Result<PathBuf> {
    extract_with_limits(archive, destination, MAX_ENTRIES, MAX_UNPACKED_BYTES)
}

fn extract_with_limits(
    archive: &Path,
    destination: &Path,
    max_entries: usize,
    max_bytes: u64,
) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(destination).at(destination)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(rejected("extraction destination must be a real directory"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(rejected(
                "extraction destination must be private (mode 0700)",
            ));
        }
    }
    if fs::read_dir(destination)
        .at(destination)?
        .next()
        .transpose()
        .at(destination)?
        .is_some()
    {
        return Err(rejected("extraction destination must be empty"));
    }

    let mut zip = zip::ZipArchive::new(File::open(archive).at(archive)?)?;
    if zip.len() > max_entries {
        return Err(rejected("too many ZIP entries"));
    }
    // ZipArchive indexes by name and otherwise hides exact duplicates. Also reject divergent
    // local headers and overlapping entries before trusting the central directory metadata.
    zip.validate_header_consistency()?;
    let mut explicit = HashSet::new();
    let mut paths = HashMap::<PathBuf, bool>::new(); // true = directory, including implied parents
    let mut announced = 0u64;
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index)?;
        let name = entry.name()?;
        let directory = entry.is_dir();
        let relative = name.strip_suffix('/').unwrap_or(&name);
        if entry.encrypted() {
            return Err(rejected("encrypted ZIP entries are not supported"));
        }
        if relative.is_empty()
            || relative.contains(['\\', ':', '\0'])
            || relative
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == ".." || part == "__MACOSX")
            || relative.split('/').next() != Some(BUNDLE_NAME)
        {
            return Err(rejected(format!("unsafe ZIP path {name:?}")));
        }
        let path = PathBuf::from(relative);
        if !explicit.insert(path.clone()) {
            return Err(rejected(format!("duplicate ZIP path {name:?}")));
        }
        let mode = entry.unix_mode().unwrap_or(0);
        let kind = mode & 0o170000;
        if (directory && !matches!(kind, 0 | 0o040000))
            || (!directory && !matches!(kind, 0 | 0o100000))
        {
            return Err(rejected(format!("symlink or special ZIP entry {name:?}")));
        }
        if directory && entry.size() != 0 {
            return Err(rejected(format!("ZIP directory has data: {name:?}")));
        }
        if paths
            .insert(path.clone(), directory)
            .is_some_and(|previous| previous != directory)
        {
            return Err(rejected(format!("file/directory collision at {name:?}")));
        }
        for parent in path
            .ancestors()
            .skip(1)
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            if paths.insert(parent.to_owned(), true) == Some(false) {
                return Err(rejected(format!("file/directory collision at {name:?}")));
            }
        }
        if relative == format!("{BUNDLE_NAME}/{EXECUTABLE_RELATIVE}") && mode & 0o111 == 0 {
            return Err(rejected("the bundle binary has no executable bits"));
        }
        announced = announced
            .checked_add(entry.size())
            .filter(|total| *total <= max_bytes)
            .ok_or_else(|| rejected("unpacked ZIP size exceeds the limit"))?;
    }
    for required in [INFO_RELATIVE, EXECUTABLE_RELATIVE] {
        if paths.get(&Path::new(BUNDLE_NAME).join(required)) != Some(&false) {
            return Err(rejected(format!("missing regular bundle file {required}")));
        }
    }

    let mut directories: Vec<_> = paths
        .iter()
        .filter(|(_, directory)| **directory)
        .map(|(path, _)| path)
        .collect();
    directories.sort_by_key(|path| path.components().count());
    for relative in directories {
        let path = destination.join(relative);
        // Unlike create_dir_all, this also rejects case/normalization aliases on macOS.
        fs::create_dir(&path).at(&path)?;
        set_mode(&path, 0o755)?;
    }
    let mut unpacked = 0u64;
    let mut buffer = [0u8; 32 * 1024];
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let path = destination.join(entry.name()?.as_ref());
        let mut file = File::create_new(&path).at(&path)?;
        let mut written = 0u64;
        loop {
            // Bound actual decompressed output, even if an entry underreports its size.
            let remaining = (max_bytes - unpacked).min(entry.size() - written);
            let length = buffer.len().min(remaining.saturating_add(1) as usize);
            let read = entry.read(&mut buffer[..length]).at(archive)?;
            if read == 0 {
                break;
            }
            if read as u64 > remaining {
                return Err(rejected("actual unpacked ZIP size exceeds its budget"));
            }
            file.write_all(&buffer[..read]).at(&path)?;
            written += read as u64;
            unpacked += read as u64;
        }
        if written != entry.size() {
            return Err(rejected("ZIP entry ended before its announced size"));
        }
        let mode = if entry.unix_mode().unwrap_or(0) & 0o111 != 0 {
            0o755
        } else {
            0o644
        };
        // Only normalized access/executable bits survive; never propagate suid/sgid/sticky bits.
        set_mode(&path, mode)?;
    }
    Ok(destination.join(BUNDLE_NAME))
}

fn set_mode(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).at(path)?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

/// Checks a regular, non-symlink executable with a native little-endian Mach-O64 header.
/// Fat/universal binaries are deliberately unsupported; this does not validate code signatures.
pub fn validate_bundle_binary(bundle: &Path, arch: &str) -> Result<()> {
    let expected: u32 = match arch {
        "aarch64" | "arm64" => 0x0100_000c,
        "x86_64" => 0x0100_0007,
        _ => return Err(rejected(format!("unsupported architecture {arch:?}"))),
    };
    for directory in [
        bundle.to_owned(),
        bundle.join("Contents"),
        bundle.join("Contents/MacOS"),
    ] {
        let metadata = fs::symlink_metadata(&directory).at(&directory)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(rejected("bundle directories must not be symlinks"));
        }
    }
    let path = bundle.join(EXECUTABLE_RELATIVE);
    let metadata = fs::symlink_metadata(&path).at(&path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(rejected("bundle binary must be a regular non-symlink file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(rejected("bundle binary is not executable"));
        }
    }
    let mut header = [0u8; 32];
    File::open(&path)
        .at(&path)?
        .read_exact(&mut header)
        .at(&path)?;
    if header[..4] != [0xcf, 0xfa, 0xed, 0xfe] {
        return Err(rejected(
            "expected native Mach-O64; fat/universal binaries are not supported",
        ));
    }
    if u32::from_le_bytes(header[4..8].try_into().expect("four-byte CPU type")) != expected {
        return Err(rejected(format!(
            "bundle binary architecture does not match {arch}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use zip::write::SimpleFileOptions;

    fn macho(cpu: u32) -> Vec<u8> {
        let mut bytes = vec![0; 32];
        bytes[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        bytes[4..8].copy_from_slice(&cpu.to_le_bytes());
        bytes
    }

    fn valid_entries() -> Vec<(&'static str, Vec<u8>, u32)> {
        vec![
            (
                "_k0t1k.app/Contents/Info.plist",
                b"synthetic plist".to_vec(),
                0o644,
            ),
            (
                "_k0t1k.app/Contents/MacOS/k0t1k-app",
                macho(0x0100_000c),
                0o6755,
            ),
        ]
    }

    fn zip_bytes(entries: &[(&str, Vec<u8>, u32)]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes, mode) in entries {
            let options = SimpleFileOptions::default().unix_permissions(*mode);
            if mode & 0o170000 == 0o120000 {
                zip.add_symlink(*name, "target", options).unwrap();
            } else if name.ends_with('/') {
                zip.add_directory(*name, options).unwrap();
            } else {
                zip.start_file(*name, options).unwrap();
                zip.write_all(bytes).unwrap();
            }
        }
        zip.finish().unwrap().into_inner()
    }

    fn fixture(bytes: &[u8]) -> (tempfile::TempDir, PathBuf, tempfile::TempDir) {
        let source = tempfile::tempdir().unwrap();
        let archive = source.path().join("update.zip");
        fs::write(&archive, bytes).unwrap();
        let destination = tempfile::tempdir().unwrap();
        set_mode(destination.path(), 0o700).unwrap();
        (source, archive, destination)
    }

    #[test]
    fn extracts_bundle_and_preserves_only_safe_executable_bits() {
        let mut entries = valid_entries();
        entries.push(("_k0t1k.app/", vec![], 0o777));
        let (_source, archive, destination) = fixture(&zip_bytes(&entries));
        let bundle = extract_bundle(&archive, destination.path()).unwrap();
        assert_eq!(bundle, destination.path().join(BUNDLE_NAME));
        assert_eq!(
            fs::read(bundle.join(INFO_RELATIVE)).unwrap(),
            b"synthetic plist"
        );
        validate_bundle_binary(&bundle, "aarch64").unwrap();
        validate_bundle_binary(&bundle, "arm64").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for (path, mode) in [
                (bundle.clone(), 0o755),
                (bundle.join(INFO_RELATIVE), 0o644),
                (bundle.join(EXECUTABLE_RELATIVE), 0o755),
            ] {
                assert_eq!(
                    fs::metadata(path).unwrap().permissions().mode() & 0o7777,
                    mode
                );
            }
        }
    }

    #[test]
    fn binary_architecture_magic_and_permissions_are_checked() {
        let (_source, archive, destination) = fixture(&zip_bytes(&valid_entries()));
        let bundle = extract_bundle(&archive, destination.path()).unwrap();
        assert!(validate_bundle_binary(&bundle, "x86_64").is_err());
        assert!(validate_bundle_binary(&bundle, "riscv64").is_err());
        let path = bundle.join(EXECUTABLE_RELATIVE);
        fs::write(&path, macho(0x0100_0007)).unwrap();
        validate_bundle_binary(&bundle, "x86_64").unwrap();
        assert!(validate_bundle_binary(&bundle, "aarch64").is_err());
        for bad in [
            vec![0xca, 0xfe, 0xba, 0xbe]
                .into_iter()
                .chain([0; 28])
                .collect(),
            vec![0; 32],
            vec![0xcf, 0xfa, 0xed, 0xfe],
        ] {
            fs::write(&path, bad).unwrap();
            assert!(validate_bundle_binary(&bundle, "x86_64").is_err());
        }
        fs::write(&path, macho(0x0100_0007)).unwrap();
        #[cfg(unix)]
        {
            set_mode(&path, 0o644).unwrap();
            assert!(validate_bundle_binary(&bundle, "x86_64").is_err());
            fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(bundle.join(INFO_RELATIVE), &path).unwrap();
            assert!(validate_bundle_binary(&bundle, "x86_64").is_err());
        }
    }

    #[test]
    fn executable_layout_allows_renamed_bundles_but_not_standalone_binaries() {
        for name in [BUNDLE_NAME, "Renamed launcher.app"] {
            let bundle = Path::new("/Applications").join(name);
            assert_eq!(
                bundle_from_executable(&bundle.join(EXECUTABLE_RELATIVE)).unwrap(),
                bundle
            );
        }
        for path in [
            "/tmp/k0t1k-app",
            "/tmp/Contents/MacOS/k0t1k-app",
            "/A.app/k0t1k-app",
            "/A.app/Contents/MacOS/other",
            "/A.app/../Contents/MacOS/k0t1k-app",
        ] {
            assert!(bundle_from_executable(Path::new(path)).is_err(), "{path}");
        }
    }

    #[test]
    fn unsafe_names_and_symlinks_are_rejected_before_extraction() {
        for (name, mode) in [
            ("../escape", 0o644),
            ("/absolute", 0o644),
            ("_k0t1k.app/../escape", 0o644),
            ("_k0t1k.app/./file", 0o644),
            ("_k0t1k.app//file", 0o644),
            ("_k0t1k.app/Contents\\escape", 0o644),
            ("_k0t1k.app/C:/escape", 0o644),
            ("other.app/file", 0o644),
            ("__MACOSX/._k0t1k.app", 0o644),
            ("_k0t1k.app/__MACOSX/resource", 0o644),
            ("_k0t1k.app/link", 0o120777),
        ] {
            let mut entries = valid_entries();
            entries.push((name, vec![], mode));
            let (_source, archive, destination) = fixture(&zip_bytes(&entries));
            assert!(
                extract_bundle(&archive, destination.path()).is_err(),
                "{name}"
            );
            assert_eq!(
                fs::read_dir(destination.path()).unwrap().count(),
                0,
                "{name}"
            );
        }
    }

    #[test]
    fn required_files_and_executable_bits_are_mandatory() {
        for entries in [
            vec![],
            valid_entries()[..1].to_vec(),
            valid_entries()[1..].to_vec(),
            {
                let mut entries = valid_entries();
                entries[1].2 = 0o644;
                entries
            },
        ] {
            let (_source, archive, destination) = fixture(&zip_bytes(&entries));
            assert!(extract_bundle(&archive, destination.path()).is_err());
            assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn file_directory_collisions_are_rejected_in_either_order() {
        for reverse in [false, true] {
            let mut entries = valid_entries();
            entries.extend([
                ("_k0t1k.app/collision", vec![], 0o644),
                ("_k0t1k.app/collision/child", vec![], 0o644),
            ]);
            if reverse {
                entries.reverse();
            }
            let (_source, archive, destination) = fixture(&zip_bytes(&entries));
            assert!(extract_bundle(&archive, destination.path()).is_err());
            assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
        }
    }

    // Patch both local and central headers while keeping each header consistent. This creates
    // malformed metadata that ZipWriter correctly refuses to generate itself.
    fn patch_headers(bytes: &mut [u8], mut patch: impl FnMut(&mut [u8], bool)) {
        let offsets: Vec<_> = bytes
            .windows(4)
            .enumerate()
            .filter_map(|(offset, magic)| {
                (magic == b"PK\x03\x04" || magic == b"PK\x01\x02")
                    .then_some((offset, magic == b"PK\x01\x02"))
            })
            .collect();
        for (offset, central) in offsets {
            patch(&mut bytes[offset..], central);
        }
    }

    #[test]
    fn duplicate_names_encryption_and_special_entries_are_rejected() {
        for kind in ["duplicate", "encrypted", "special"] {
            let mut entries = valid_entries();
            entries.extend([
                ("_k0t1k.app/one", vec![], 0o644),
                ("_k0t1k.app/two", vec![], 0o644),
            ]);
            let mut bytes = zip_bytes(&entries);
            patch_headers(&mut bytes, |header, central| {
                let name_start = if central { 46 } else { 30 };
                let length_offset = if central { 28 } else { 26 };
                let length = u16::from_le_bytes(
                    header[length_offset..length_offset + 2].try_into().unwrap(),
                ) as usize;
                if kind == "duplicate"
                    && &header[name_start..name_start + length] == b"_k0t1k.app/two"
                {
                    header[name_start..name_start + length].copy_from_slice(b"_k0t1k.app/one");
                } else if kind == "encrypted" {
                    let flags = if central { 8 } else { 6 };
                    header[flags] |= 1;
                } else if kind == "special" && central {
                    header[38..42].copy_from_slice(&(0o010644u32 << 16).to_le_bytes());
                }
            });
            let (_source, archive, destination) = fixture(&bytes);
            assert!(
                extract_bundle(&archive, destination.path()).is_err(),
                "{kind}"
            );
            assert_eq!(
                fs::read_dir(destination.path()).unwrap().count(),
                0,
                "{kind}"
            );
        }
    }

    #[test]
    fn announced_entry_count_and_total_size_are_bounded() {
        let bytes = zip_bytes(&valid_entries());
        let (_source, archive, destination) = fixture(&bytes);
        assert!(extract_with_limits(&archive, destination.path(), 1, MAX_UNPACKED_BYTES).is_err());
        assert!(extract_with_limits(&archive, destination.path(), MAX_ENTRIES, 40).is_err());
        assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
        let mut oversized = bytes;
        patch_headers(&mut oversized, |header, central| {
            let offset = if central { 24 } else { 22 };
            header[offset..offset + 4]
                .copy_from_slice(&((MAX_UNPACKED_BYTES + 1) as u32).to_le_bytes());
        });
        fs::write(&archive, oversized).unwrap();
        assert!(extract_bundle(&archive, destination.path()).is_err());
        assert_eq!(fs::read_dir(destination.path()).unwrap().count(), 0);
    }

    #[test]
    fn actual_entry_bytes_cannot_exceed_announced_budget() {
        let mut entries = valid_entries();
        entries.push(("_k0t1k.app/large", vec![b'x'; 100_000], 0o644));
        let mut bytes = zip_bytes(&entries);
        patch_headers(&mut bytes, |header, central| {
            let name_start = if central { 46 } else { 30 };
            if header[name_start..].starts_with(b"_k0t1k.app/large") {
                let offset = if central { 24 } else { 22 };
                header[offset..offset + 4].copy_from_slice(&1u32.to_le_bytes());
            }
        });
        let (_source, archive, destination) = fixture(&bytes);
        let error = extract_bundle(&archive, destination.path()).unwrap_err();
        // zip also enforces its own size limit before returning decompressed bytes.
        let message = error.to_string();
        assert!(
            message.contains("actual unpacked ZIP size")
                || message.contains("declared uncompressed size"),
            "{error}"
        );
        assert!(
            fs::metadata(destination.path().join(BUNDLE_NAME).join("large"))
                .unwrap()
                .len()
                <= 1
        );
    }

    #[test]
    fn destination_must_exist_and_be_empty_without_replacing_files() {
        let (_source, archive, destination) = fixture(&zip_bytes(&valid_entries()));
        assert!(extract_bundle(&archive, &destination.path().join("absent")).is_err());
        #[cfg(unix)]
        {
            set_mode(destination.path(), 0o755).unwrap();
            let error = extract_bundle(&archive, destination.path()).unwrap_err();
            assert!(error.to_string().contains("must be private"));
            set_mode(destination.path(), 0o700).unwrap();
        }
        let existing = destination.path().join("keep");
        fs::write(&existing, b"untouched").unwrap();
        assert!(extract_bundle(&archive, destination.path()).is_err());
        assert_eq!(fs::read(&existing).unwrap(), b"untouched");
        #[cfg(unix)]
        {
            let link = destination.path().join("link");
            std::os::unix::fs::symlink(destination.path(), &link).unwrap();
            assert!(extract_bundle(&archive, &link).is_err());
        }
    }
}
