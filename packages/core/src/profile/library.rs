use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::Deserialize;

const DEFAULT_LIBRARY_URL: &str = "https://libraries.minecraft.net/";

/// Maven coordinates `group:artifact:version[:classifier][@extension]`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct GradleSpecifier {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: String,
}

impl FromStr for GradleSpecifier {
    type Err = String;

    fn from_str(spec: &str) -> Result<Self, String> {
        let (coordinates, extension) = spec.split_once('@').unwrap_or((spec, "jar"));
        let parts: Vec<&str> = coordinates.split(':').collect();
        let unsafe_part = |part: &&str| part.is_empty() || part.contains(['/', '\\']) || *part == "..";
        if !(3..=4).contains(&parts.len()) || parts.iter().any(unsafe_part) || extension.contains(['/', '\\']) {
            return Err(format!("invalid gradle specifier `{spec}`"));
        }
        Ok(Self {
            group: parts[0].to_owned(),
            artifact: parts[1].to_owned(),
            version: parts[2].to_owned(),
            classifier: parts.get(3).map(|part| (*part).to_owned()),
            extension: extension.to_owned(),
        })
    }
}

impl TryFrom<String> for GradleSpecifier {
    type Error = String;

    fn try_from(spec: String) -> Result<Self, String> {
        spec.parse()
    }
}

impl fmt::Display for GradleSpecifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.group, self.artifact, self.version)?;
        if let Some(classifier) = &self.classifier {
            write!(f, ":{classifier}")?;
        }
        if self.extension != "jar" {
            write!(f, "@{}", self.extension)?;
        }
        Ok(())
    }
}

impl GradleSpecifier {
    /// Path relative to the libraries directory, always with `/` separators.
    pub fn path(&self) -> String {
        let mut file = format!("{}-{}", self.artifact, self.version);
        if let Some(classifier) = &self.classifier {
            file.push('-');
            file.push_str(classifier);
        }
        format!("{}/{}/{}/{file}.{}", self.group.replace('.', "/"), self.artifact, self.version, self.extension)
    }

    fn with_classifier(&self, classifier: &str) -> Self {
        Self { classifier: Some(classifier.to_owned()), ..self.clone() }
    }

    /// Identity used to dedupe libraries across components: everything but the version.
    pub(crate) fn dedupe_key(&self) -> (&str, &str, Option<&str>) {
        (&self.group, &self.artifact, self.classifier.as_deref())
    }
}

/// OS and architecture that library rules and natives are matched against.
/// The architecture is the one of the Java binary, not of the OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContext {
    /// `windows`, `linux`, `osx`, `freebsd` or `openbsd`.
    pub system: String,
    /// Normalised Java architecture: `x86_64`, `x86`, `arm64`, `arm32` or the raw `os.arch` value.
    pub arch: String,
}

impl RuntimeContext {
    /// `java_os_arch` is the `os.arch` system property of the Java binary (`amd64`, `aarch64`, ...).
    pub fn new(system: &str, java_os_arch: &str) -> Self {
        let arch = match java_os_arch {
            "amd64" => "x86_64",
            "i386" | "i686" => "x86",
            "aarch64" => "arm64",
            "arm" | "armhf" => "arm32",
            other => other,
        };
        Self { system: system.to_owned(), arch: arch.to_owned() }
    }

    pub fn host(java_os_arch: &str) -> Self {
        let system = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "osx"
        } else if cfg!(target_os = "freebsd") {
            "freebsd"
        } else if cfg!(target_os = "openbsd") {
            "openbsd"
        } else {
            "linux"
        };
        Self::new(system, java_os_arch)
    }

    pub fn classifier(&self) -> String {
        format!("{}-{}", self.system, self.arch)
    }

    /// Mojang only knew x86 and x86_64, so for them a plain OS name also matches.
    fn is_legacy_arch(&self) -> bool {
        self.arch == "x86_64" || self.arch == "x86"
    }

    pub fn classifier_matches(&self, name: &str) -> bool {
        name == self.classifier() || (self.is_legacy_arch() && name == self.system)
    }

    /// Pointer width of the Java binary, substituted for `${arch}` in native classifiers.
    pub fn java_bits(&self) -> &'static str {
        if self.arch == "x86" || self.arch == "arm32" { "32" } else { "64" }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Rule {
    pub action: RuleAction,
    #[serde(default)]
    pub os: Option<RuleOs>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RuleOs {
    #[serde(default)]
    pub name: Option<String>,
}

impl Rule {
    /// `None` (defer) when the rule names an OS that does not match the context.
    fn apply(&self, ctx: &RuntimeContext) -> Option<RuleAction> {
        let os_name = self.os.as_ref().and_then(|os| os.name.as_deref());
        match os_name {
            Some(name) if !ctx.classifier_matches(name) => None,
            _ => Some(self.action),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Artifact {
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: HashMap<String, Artifact>,
}

/// A library, maven file or main jar entry of a Prism meta version file.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Library {
    pub name: GradleSpecifier,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    /// Legacy natives: OS name -> classifier, possibly containing `${arch}`.
    #[serde(default)]
    pub natives: HashMap<String, String>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Maven repository base URL for libraries without `downloads`.
    #[serde(default)]
    pub url: Option<String>,
}

/// One file of a library: where it lives under `libraries/` and where it comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryFile {
    /// Relative to the libraries directory, `/`-separated.
    pub path: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

impl Library {
    pub fn is_native(&self) -> bool {
        !self.natives.is_empty()
    }

    /// Without rules a library is active. Otherwise the last rule that matches the context wins,
    /// starting from "disallow". A native library also needs a classifier for the context.
    pub fn is_active(&self, ctx: &RuntimeContext) -> bool {
        let rules_allow = self.rules.is_empty()
            || self.rules.iter().filter_map(|rule| rule.apply(ctx)).next_back() == Some(RuleAction::Allow);
        rules_allow && (!self.is_native() || self.native_classifier(ctx).is_some())
    }

    /// Classifier of the native jar for the context, with `${arch}` replaced by the Java bitness.
    pub fn native_classifier(&self, ctx: &RuntimeContext) -> Option<String> {
        let template = self
            .natives
            .get(&ctx.classifier())
            .or_else(|| ctx.is_legacy_arch().then(|| self.natives.get(&ctx.system)).flatten())?;
        Some(template.replace("${arch}", ctx.java_bits()))
    }

    /// The file this library contributes in the context; `None` when it has nothing to download
    /// (no artifact, or no native classifier).
    pub fn file(&self, ctx: &RuntimeContext) -> Option<LibraryFile> {
        let (name, native_classifier) = if self.is_native() {
            let classifier = self.native_classifier(ctx)?;
            (self.name.with_classifier(&classifier), Some(classifier))
        } else {
            (self.name.clone(), None)
        };
        let path = name.path();

        match (&self.downloads, native_classifier) {
            (Some(downloads), classifier) => {
                let artifact = match classifier {
                    Some(classifier) => downloads.classifiers.get(&classifier)?,
                    None => downloads.artifact.as_ref()?,
                };
                Some(LibraryFile { path, url: artifact.url.clone(), sha1: artifact.sha1.clone(), size: artifact.size })
            }
            (None, _) => {
                let base = self.url.as_deref().unwrap_or(DEFAULT_LIBRARY_URL);
                let separator = if base.ends_with('/') { "" } else { "/" };
                Some(LibraryFile { url: format!("{base}{separator}{path}"), path, sha1: None, size: None })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(json: &str) -> Library {
        serde_json::from_str(json).unwrap()
    }

    fn linux() -> RuntimeContext {
        RuntimeContext::new("linux", "amd64")
    }

    #[test]
    fn gradle_paths() {
        let spec: GradleSpecifier = "org.ow2.asm:asm:9.7.1".parse().unwrap();
        assert_eq!(spec.path(), "org/ow2/asm/asm/9.7.1/asm-9.7.1.jar");

        let spec: GradleSpecifier = "com.mojang:minecraft:1.21.4:client".parse().unwrap();
        assert_eq!(spec.path(), "com/mojang/minecraft/1.21.4/minecraft-1.21.4-client.jar");

        let spec: GradleSpecifier = "net.minecraftforge:forge:1.20.1-47.4.28:installer".parse().unwrap();
        assert_eq!(spec.path(), "net/minecraftforge/forge/1.20.1-47.4.28/forge-1.20.1-47.4.28-installer.jar");

        let spec: GradleSpecifier = "de.oceanlabs.mcp:mcp_config:1.20.1-20230612.114412@zip".parse().unwrap();
        assert_eq!(
            spec.path(),
            "de/oceanlabs/mcp/mcp_config/1.20.1-20230612.114412/mcp_config-1.20.1-20230612.114412.zip"
        );
        assert_eq!(spec.to_string(), "de.oceanlabs.mcp:mcp_config:1.20.1-20230612.114412@zip");

        assert!("group:artifact".parse::<GradleSpecifier>().is_err());
    }

    #[test]
    fn architecture_mapping() {
        assert_eq!(RuntimeContext::new("windows", "amd64").classifier(), "windows-x86_64");
        assert_eq!(RuntimeContext::new("windows", "i386").arch, "x86");
        assert_eq!(RuntimeContext::new("osx", "aarch64").classifier(), "osx-arm64");
        assert_eq!(RuntimeContext::new("linux", "armhf").arch, "arm32");
        assert_eq!(RuntimeContext::new("linux", "armhf").java_bits(), "32");
        assert_eq!(RuntimeContext::new("linux", "aarch64").java_bits(), "64");
    }

    #[test]
    fn plain_os_rule_matches_only_legacy_arches() {
        let x64 = RuntimeContext::new("windows", "amd64");
        let arm = RuntimeContext::new("windows", "aarch64");
        assert!(x64.classifier_matches("windows"));
        assert!(x64.classifier_matches("windows-x86_64"));
        assert!(!x64.classifier_matches("windows-arm64"));
        assert!(!arm.classifier_matches("windows"));
        assert!(arm.classifier_matches("windows-arm64"));
    }

    #[test]
    fn rules_last_match_wins() {
        let lib = library(r#"{"name":"a:b:1","rules":[{"action":"allow"},{"action":"disallow","os":{"name":"osx"}}]}"#);
        assert!(lib.is_active(&linux()));
        assert!(!lib.is_active(&RuntimeContext::new("osx", "amd64")));
    }

    #[test]
    fn rules_matching_nothing_exclude_the_library() {
        let lib = library(r#"{"name":"a:b:1","rules":[{"action":"allow","os":{"name":"osx"}}]}"#);
        assert!(!lib.is_active(&linux()));
        assert!(library(r#"{"name":"a:b:1"}"#).is_active(&linux()));
    }

    #[test]
    fn lwjgl_windows_natives_on_x64() {
        let lib = library(
            r#"{"name":"org.lwjgl:lwjgl-glfw-natives-windows-x86:3.3.3","rules":[{"action":"allow","os":{"name":"windows"}}],
                "downloads":{"artifact":{"url":"https://x/y.jar","sha1":"aa","size":1}}}"#,
        );
        assert!(lib.is_active(&RuntimeContext::new("windows", "amd64")));
        assert!(!lib.is_active(&RuntimeContext::new("windows", "aarch64")));
    }

    #[test]
    fn legacy_native_classifier_and_file() {
        let lib = library(
            r#"{"name":"org.lwjgl.lwjgl:lwjgl-platform:2.9.4","natives":{"linux":"natives-linux","windows":"natives-windows-${arch}"},
                "downloads":{"artifact":{"url":"https://x/main.jar"},
                "classifiers":{"natives-linux":{"url":"https://x/l.jar","sha1":"bb","size":5},
                               "natives-windows-64":{"url":"https://x/w64.jar"}}}}"#,
        );
        assert!(lib.is_native());
        let file = lib.file(&linux()).unwrap();
        assert_eq!(file.path, "org/lwjgl/lwjgl/lwjgl-platform/2.9.4/lwjgl-platform-2.9.4-natives-linux.jar");
        assert_eq!(file.url, "https://x/l.jar");
        assert_eq!(file.sha1.as_deref(), Some("bb"));

        let windows = RuntimeContext::new("windows", "amd64");
        assert_eq!(lib.native_classifier(&windows).as_deref(), Some("natives-windows-64"));
        assert_eq!(lib.file(&windows).unwrap().url, "https://x/w64.jar");

        let osx = RuntimeContext::new("osx", "amd64");
        assert!(!lib.is_active(&osx));
        assert!(lib.file(&osx).is_none());
    }

    #[test]
    fn maven_url_libraries_have_no_checksum() {
        let lib = library(r#"{"name":"net.fabricmc:intermediary:1.21.4","url":"https://maven.fabricmc.net"}"#);
        let file = lib.file(&linux()).unwrap();
        assert_eq!(file.url, "https://maven.fabricmc.net/net/fabricmc/intermediary/1.21.4/intermediary-1.21.4.jar");
        assert_eq!(file.sha1, None);

        let default_host = library(r#"{"name":"com.google.guava:guava:31.0"}"#).file(&linux()).unwrap();
        assert_eq!(default_host.url, "https://libraries.minecraft.net/com/google/guava/guava/31.0/guava-31.0.jar");
    }

    #[test]
    fn library_with_downloads_but_no_artifact_is_skipped() {
        let lib = library(r#"{"name":"a:b:1","downloads":{}}"#);
        assert!(lib.file(&linux()).is_none());
    }
}
