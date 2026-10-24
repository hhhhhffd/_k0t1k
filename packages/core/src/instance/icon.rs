//! Custom instance icons: an image picked by the user, stored as `instances/<id>/icon-<millis>.<ext>`.
//! `instance.json` marks it with `icon: "custom:<file name>"`; every other value is a built-in icon key.
//! The file name changes with each icon, so the UI never shows a cached older picture.

use std::path::Path;

use super::Instance;
use super::manager::InstanceManager;
use crate::error::{Error, IoContext, Result};

const CUSTOM_PREFIX: &str = "custom:";
/// Biggest image accepted; an icon is shown at 64 px.
pub const MAX_ICON_BYTES: u64 = 5 * 1024 * 1024;
/// Icon value of an instance without a custom picture (the UI maps unknown keys to its default).
pub const DEFAULT_ICON: &str = "default";

/// The file name inside the instance folder when `icon` is a custom icon. Only plain names are accepted,
/// since the value comes from a file the user may have imported.
pub fn custom_file_name(icon: &str) -> Option<&str> {
    let name = icon.strip_prefix(CUSTOM_PREFIX)?;
    let plain = !name.is_empty() && !name.contains(['/', '\\', ':']) && name != "." && name != "..";
    plain.then_some(name)
}

/// File extension for the image format recognised by its magic bytes.
fn extension_of(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

fn too_big(size: u64) -> Error {
    Error::Other(format!("the image is {size} bytes; icons can be at most {MAX_ICON_BYTES}"))
}

/// Reads an image file for use as an icon, rejecting big files before reading them.
pub fn read_image(path: &Path) -> Result<Vec<u8>> {
    let size = std::fs::metadata(path).at(path)?.len();
    if size > MAX_ICON_BYTES {
        return Err(too_big(size));
    }
    std::fs::read(path).at(path)
}

impl InstanceManager {
    /// Make `image` (PNG, JPEG, GIF or WebP, checked by content rather than by name) the instance's icon.
    /// The previous custom icon file is removed.
    pub fn set_custom_icon(&self, id: &str, image: &[u8]) -> Result<Instance> {
        if image.len() as u64 > MAX_ICON_BYTES {
            return Err(too_big(image.len() as u64));
        }
        let extension = extension_of(image).ok_or_else(|| Error::Other("the file is not a PNG, JPEG, GIF or WebP image".to_owned()))?;
        let file_name = format!("icon-{}.{extension}", chrono::Utc::now().timestamp_millis());
        let path = self.get(id)?.dir(self.paths()).join(&file_name);
        std::fs::write(&path, image).at(&path)?;
        self.update(id, |instance| instance.icon = format!("{CUSTOM_PREFIX}{file_name}")).inspect_err(|_| {
            // The instance still points at its old icon, so the new file is unused; the update error is the one to report.
            let _ = std::fs::remove_file(&path);
        })
    }

    /// Back to the default built-in icon, removing the custom file.
    pub fn reset_icon(&self, id: &str) -> Result<Instance> {
        self.update(id, |instance| instance.icon = DEFAULT_ICON.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance::{CopyOptions, NewInstance};
    use crate::paths::Paths;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest";

    fn manager_with_instance() -> (tempfile::TempDir, InstanceManager, String) {
        let dir = tempfile::tempdir().unwrap();
        let manager = InstanceManager::new(Paths::new(dir.path().join("data")));
        let new = NewInstance { name: "Pack".into(), icon: None, group: None, components: Vec::new(), managed_pack: None };
        let id = manager.create(new).unwrap().id;
        (dir, manager, id)
    }

    fn icon_file(manager: &InstanceManager, instance: &Instance) -> std::path::PathBuf {
        instance.dir(manager.paths()).join(custom_file_name(&instance.icon).unwrap())
    }

    #[test]
    fn magic_bytes_decide_the_format() {
        assert_eq!(extension_of(PNG), Some("png"));
        assert_eq!(extension_of(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(extension_of(b"GIF89a.."), Some("gif"));
        assert_eq!(extension_of(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(extension_of(b"RIFF\0\0\0\0WAVEfmt "), None);
        assert_eq!(extension_of(b"<svg></svg>"), None);
        assert_eq!(extension_of(b""), None);
    }

    #[test]
    fn custom_names_must_be_plain() {
        assert_eq!(custom_file_name("custom:icon-1.png"), Some("icon-1.png"));
        for bad in ["grass", "custom:", "custom:../x.png", "custom:a/b.png", "custom:a\\b.png", "custom:..", "custom:C:x.png"] {
            assert_eq!(custom_file_name(bad), None, "{bad}");
        }
    }

    #[test]
    fn set_replace_and_reset() {
        let (_dir, manager, id) = manager_with_instance();
        let first = manager.set_custom_icon(&id, PNG).unwrap();
        let first_file = icon_file(&manager, &first);
        assert_eq!(std::fs::read(&first_file).unwrap(), PNG);

        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = manager.set_custom_icon(&id, &[0xFF, 0xD8, 0xFF, 0, 1]).unwrap();
        assert_ne!(first.icon, second.icon);
        assert!(second.icon.ends_with(".jpg"));
        assert!(!first_file.exists());
        assert!(icon_file(&manager, &second).is_file());
        assert_eq!(manager.get(&id).unwrap().icon, second.icon);

        let reset = manager.reset_icon(&id).unwrap();
        assert_eq!(reset.icon, DEFAULT_ICON);
        assert!(!icon_file(&manager, &second).exists());
    }

    #[test]
    fn rejects_non_images_and_leaves_the_instance_alone() {
        let (_dir, manager, id) = manager_with_instance();
        assert!(manager.set_custom_icon(&id, b"not an image").is_err());
        let big = vec![0u8; MAX_ICON_BYTES as usize + 1];
        assert!(manager.set_custom_icon(&id, &big).is_err());
        assert_eq!(manager.get(&id).unwrap().icon, "default");
    }

    #[test]
    fn replacing_the_icon_with_a_built_in_one_removes_the_file() {
        let (_dir, manager, id) = manager_with_instance();
        let custom = manager.set_custom_icon(&id, PNG).unwrap();
        manager.update(&id, |instance| instance.icon = "sword".to_owned()).unwrap();
        assert!(!icon_file(&manager, &custom).exists());
    }

    #[test]
    fn copy_keeps_the_icon() {
        let (_dir, manager, id) = manager_with_instance();
        manager.set_custom_icon(&id, PNG).unwrap();
        let copy = manager.copy(&id, "Pack copy", &CopyOptions::default()).unwrap();
        assert!(icon_file(&manager, &copy).is_file());
        manager.reset_icon(&id).unwrap();
        assert!(icon_file(&manager, &copy).is_file());
    }
}
