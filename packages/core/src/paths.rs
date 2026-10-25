use std::path::{Path, PathBuf};

/// On-disk layout of the launcher data directory.
///
/// Game metadata, libraries, assets and Java runtimes are shared between all instances;
/// each instance only owns its own game directory.
#[derive(Debug, Clone)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// `%APPDATA%/_k0t1k` on Windows, the XDG data dir elsewhere.
    pub fn default_root() -> Option<PathBuf> {
        dirs::data_dir().map(|dir| dir.join("_k0t1k"))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Cached JSON metadata (version manifests, loader profiles).
    pub fn meta_dir(&self) -> PathBuf {
        self.root.join("meta")
    }

    pub fn versions_dir(&self) -> PathBuf {
        self.root.join("meta").join("versions")
    }

    pub fn libraries_dir(&self) -> PathBuf {
        self.root.join("libraries")
    }

    pub fn assets_dir(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn natives_dir(&self) -> PathBuf {
        self.root.join("natives")
    }

    pub fn java_dir(&self) -> PathBuf {
        self.root.join("java")
    }

    pub fn instances_dir(&self) -> PathBuf {
        self.root.join("instances")
    }

    /// Content shared by every instance (resource packs, etc.).
    pub fn shared_dir(&self) -> PathBuf {
        self.root.join("shared")
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// Pictures the user can pick as instance icons.
    pub fn icons_dir(&self) -> PathBuf {
        self.root.join("icons")
    }

    pub fn catpacks_dir(&self) -> PathBuf {
        self.root.join("catpacks")
    }

    /// Custom theme files (`*.json`).
    pub fn themes_dir(&self) -> PathBuf {
        self.root.join("themes")
    }

    /// Icon sets: one subfolder per set.
    pub fn iconsets_dir(&self) -> PathBuf {
        self.root.join("iconsets")
    }

    /// Installed wasm plugins: `<id>/<version>/` folders and an `active` pointer per plugin.
    pub fn plugins_dir(&self) -> PathBuf {
        self.root.join("plugins")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    /// Account tokens; kept apart from `settings.json`, which is sent to the UI as a whole.
    pub fn accounts_file(&self) -> PathBuf {
        self.root.join("accounts.json")
    }
}
