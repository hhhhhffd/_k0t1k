//! Component resolution and the merged launch profile (Prism's `PackProfile` / `LaunchProfile`).

pub mod library;
mod resolve;
mod version;

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashSet};

use crate::component::MINECRAFT_UID;
use crate::error::{Error, Result};
use crate::meta::{AssetIndexRef, LoggingConfig};

use library::{Library, LibraryFile, RuntimeContext};
pub use resolve::{ResolvedComponent, resolve_components};
use version::compare_versions;

/// Used by Prism when no component provides an asset index.
const LEGACY_ASSET_INDEX_URL: &str =
    "https://piston-meta.mojang.com/mc/assets/legacy/c0fd82e8ce9fbc93119e40d96d5a4e62cfa3f729/legacy.json";

/// Everything needed to install and launch a game, merged from the resolved components in order.
#[derive(Debug, Clone)]
pub struct LaunchProfile {
    pub minecraft_version: String,
    /// `release`, `snapshot`, `old_beta` or `old_alpha`.
    pub version_type: String,
    pub main_class: String,
    pub main_jar: Option<Library>,
    /// Active non-native libraries, deduped by `group:artifact[:classifier]`.
    pub libraries: Vec<Library>,
    /// Active libraries with legacy classifier natives; their jars are extracted, not put on the classpath.
    pub native_libraries: Vec<Library>,
    /// Downloaded into `libraries/` but never on the classpath (e.g. the Forge installer).
    pub maven_files: Vec<Library>,
    /// Flat, space-separated game arguments with `${token}` placeholders.
    pub minecraft_arguments: String,
    pub jvm_args: Vec<String>,
    pub tweakers: Vec<String>,
    pub traits: BTreeSet<String>,
    pub asset_index: AssetIndexRef,
    pub compatible_java_majors: Vec<u32>,
    pub compatible_java_name: Option<String>,
    pub logging: Option<LoggingConfig>,
    pub runtime: RuntimeContext,
}

fn apply_library(list: &mut Vec<Library>, library: &Library) {
    let existing = list.iter().position(|other| other.name.dedupe_key() == library.name.dedupe_key());
    match existing {
        None => list.push(library.clone()),
        Some(index) if compare_versions(&library.name.version, &list[index].name.version) == Ordering::Greater => {
            list[index] = library.clone();
        }
        Some(_) => {}
    }
}

impl LaunchProfile {
    /// Applies the components in list order; later components override earlier ones.
    pub fn build(components: &[ResolvedComponent], runtime: RuntimeContext) -> Result<Self> {
        let mut profile = Self {
            minecraft_version: String::new(),
            version_type: "release".to_owned(),
            main_class: String::new(),
            main_jar: None,
            libraries: Vec::new(),
            native_libraries: Vec::new(),
            maven_files: Vec::new(),
            minecraft_arguments: String::new(),
            jvm_args: Vec::new(),
            tweakers: Vec::new(),
            traits: BTreeSet::new(),
            asset_index: AssetIndexRef {
                id: "legacy".to_owned(),
                sha1: None,
                size: None,
                url: LEGACY_ASSET_INDEX_URL.to_owned(),
            },
            compatible_java_majors: Vec::new(),
            compatible_java_name: None,
            logging: None,
            runtime,
        };
        let mut has_minecraft = false;

        for ResolvedComponent { spec, file } in components {
            if spec.uid == MINECRAFT_UID {
                has_minecraft = true;
                profile.minecraft_version = file.version.clone();
                if let Some(kind) = &file.kind {
                    profile.version_type = kind.clone();
                }
                if let Some(asset_index) = &file.asset_index {
                    profile.asset_index = asset_index.clone();
                }
            }
            if let Some(main_class) = file.main_class.as_ref().filter(|class| !class.is_empty()) {
                profile.main_class = main_class.clone();
            }
            if file.main_jar.is_some() {
                profile.main_jar = file.main_jar.clone();
            }
            if let Some(arguments) = file.minecraft_arguments.as_ref().filter(|args| !args.is_empty()) {
                profile.minecraft_arguments = arguments.clone();
            }
            profile.jvm_args.extend(file.jvm_args.iter().cloned());
            for tweaker in &file.tweakers {
                profile.tweakers.retain(|existing| existing != tweaker);
                profile.tweakers.push(tweaker.clone());
            }
            profile.traits.extend(file.traits.iter().cloned());
            profile.compatible_java_majors.extend(&file.compatible_java_majors);
            if let Some(name) = file.compatible_java_name.as_ref().filter(|name| !name.is_empty()) {
                profile.compatible_java_name = Some(name.clone());
            }
            if file.logging.is_some() {
                profile.logging = file.logging.clone();
            }

            for library in file.libraries.iter().filter(|library| library.is_active(&profile.runtime)) {
                let list = if library.is_native() { &mut profile.native_libraries } else { &mut profile.libraries };
                apply_library(list, library);
            }
            let runtime = &profile.runtime;
            let maven_files =
                file.maven_files.iter().filter(|library| library.is_active(runtime) && !library.is_native());
            profile.maven_files.extend(maven_files.cloned());
        }

        if !has_minecraft {
            return Err(Error::Other("the component list has no net.minecraft component".to_owned()));
        }
        if profile.main_class.is_empty() {
            return Err(Error::Other("no component provides a main class".to_owned()));
        }
        Ok(profile)
    }

    pub fn has_trait(&self, name: &str) -> bool {
        self.traits.contains(name)
    }

    /// An empty list of compatible majors accepts any Java.
    pub fn is_java_compatible(&self, java_major: u32) -> bool {
        self.compatible_java_majors.is_empty() || self.compatible_java_majors.contains(&java_major)
    }

    /// Classpath entries in order: libraries, then the main jar.
    pub fn classpath_files(&self) -> Vec<LibraryFile> {
        self.libraries.iter().chain(&self.main_jar).filter_map(|library| library.file(&self.runtime)).collect()
    }

    /// Native jars to extract into the natives directory.
    pub fn native_files(&self) -> Vec<LibraryFile> {
        self.native_libraries.iter().filter_map(|library| library.file(&self.runtime)).collect()
    }

    /// Every file to download into `libraries/`: classpath, natives and maven files.
    pub fn library_files(&self) -> Vec<LibraryFile> {
        let maven = self.maven_files.iter().filter_map(|library| library.file(&self.runtime));
        let mut seen = HashSet::new();
        self.classpath_files()
            .into_iter()
            .chain(self.native_files())
            .chain(maven)
            .filter(|file| seen.insert(file.path.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::component::ComponentSpec;
    use crate::meta::VersionFile;

    use super::*;

    fn component(uid: &str, version: &str, json_body: &str) -> ResolvedComponent {
        let file: VersionFile =
            serde_json::from_str(&format!(r#"{{"uid":"{uid}","version":"{version}",{json_body}}}"#)).unwrap();
        ResolvedComponent {
            spec: ComponentSpec { uid: uid.to_owned(), version: version.to_owned(), dependency_only: false },
            file,
        }
    }

    fn linux() -> RuntimeContext {
        RuntimeContext::new("linux", "amd64")
    }

    fn lib_names(libraries: &[Library]) -> Vec<String> {
        libraries.iter().map(|library| library.name.to_string()).collect()
    }

    #[test]
    fn later_components_override_scalars_and_append_lists() {
        let minecraft = component(
            "net.minecraft",
            "1.0",
            r#""type":"release","mainClass":"A","minecraftArguments":"--a","+jvmArgs":["-Da"],
               "+traits":["t1"],"+tweakers":["x","y"],"compatibleJavaMajors":[8],"compatibleJavaName":"jre-a",
               "assetIndex":{"id":"5","url":"https://x/5.json"}"#,
        );
        let loader = component(
            "loader",
            "2",
            r#""mainClass":"B","minecraftArguments":"","+jvmArgs":["-Db"],"+traits":["t2","t1"],
               "+tweakers":["x"],"compatibleJavaMajors":[17],"compatibleJavaName":"""#,
        );
        let profile = LaunchProfile::build(&[minecraft, loader], linux()).unwrap();

        assert_eq!(profile.main_class, "B");
        assert_eq!(profile.minecraft_arguments, "--a");
        assert_eq!(profile.jvm_args, ["-Da", "-Db"]);
        assert_eq!(profile.traits.iter().collect::<Vec<_>>(), ["t1", "t2"]);
        assert_eq!(profile.tweakers, ["y", "x"]);
        assert_eq!(profile.compatible_java_majors, [8, 17]);
        assert_eq!(profile.compatible_java_name.as_deref(), Some("jre-a"));
        assert_eq!(profile.asset_index.id, "5");
        assert_eq!(profile.minecraft_version, "1.0");
        assert!(profile.is_java_compatible(17));
        assert!(!profile.is_java_compatible(21));
    }

    #[test]
    fn libraries_are_deduped_keeping_position_and_highest_version() {
        let minecraft = component(
            "net.minecraft",
            "1.0",
            r#""mainClass":"A","libraries":[{"name":"g:a:1.0"},{"name":"g:b:1.0"},{"name":"g:a:1.0:natives"}]"#,
        );
        let loader = component(
            "loader",
            "1",
            r#""libraries":[{"name":"g:a:2.0"},{"name":"g:b:0.5"},{"name":"g:a:1.0:natives"}]"#,
        );
        let profile = LaunchProfile::build(&[minecraft, loader], linux()).unwrap();
        assert_eq!(lib_names(&profile.libraries), ["g:a:2.0", "g:b:1.0", "g:a:1.0:natives"]);
    }

    #[test]
    fn inactive_libraries_are_dropped_and_natives_separated() {
        let minecraft = component(
            "net.minecraft",
            "1.0",
            r#""mainClass":"A","libraries":[
                {"name":"g:mac:1","rules":[{"action":"allow","os":{"name":"osx"}}]},
                {"name":"g:nat:1","natives":{"linux":"natives-linux"},"downloads":{"classifiers":{"natives-linux":{"url":"u"}}}},
                {"name":"g:winonly:1","natives":{"windows":"natives-windows"}},
                {"name":"g:plain:1"}],
               "mavenFiles":[{"name":"g:installer:1:installer"},{"name":"g:mac:1","rules":[{"action":"allow","os":{"name":"osx"}}]}]"#,
        );
        let profile = LaunchProfile::build(&[minecraft], linux()).unwrap();
        assert_eq!(lib_names(&profile.libraries), ["g:plain:1"]);
        assert_eq!(lib_names(&profile.native_libraries), ["g:nat:1"]);
        assert_eq!(lib_names(&profile.maven_files), ["g:installer:1:installer"]);
        assert_eq!(profile.native_files()[0].path, "g/nat/1/nat-1-natives-linux.jar");
    }

    #[test]
    fn main_jar_follows_libraries_on_the_classpath() {
        let minecraft = component(
            "net.minecraft",
            "1.21.4",
            r#""mainClass":"A","libraries":[{"name":"g:a:1"}],
               "mainJar":{"name":"com.mojang:minecraft:1.21.4:client","downloads":{"artifact":{"url":"https://x/c.jar","sha1":"aa"}}}"#,
        );
        let loader = component("loader", "1", r#""libraries":[{"name":"g:b:1"}]"#);
        let profile = LaunchProfile::build(&[minecraft, loader], linux()).unwrap();
        let paths: Vec<_> = profile.classpath_files().into_iter().map(|file| file.path).collect();
        assert_eq!(
            paths,
            ["g/a/1/a-1.jar", "g/b/1/b-1.jar", "com/mojang/minecraft/1.21.4/minecraft-1.21.4-client.jar"]
        );
    }

    #[test]
    fn missing_minecraft_component_is_an_error() {
        let loader = component("loader", "1", r#""mainClass":"B""#);
        assert!(LaunchProfile::build(&[loader], linux()).is_err());
    }

    #[test]
    fn asset_index_defaults_to_legacy() {
        let minecraft = component("net.minecraft", "1.0", r#""mainClass":"A""#);
        let profile = LaunchProfile::build(&[minecraft], linux()).unwrap();
        assert_eq!(profile.asset_index.id, "legacy");
    }
}
