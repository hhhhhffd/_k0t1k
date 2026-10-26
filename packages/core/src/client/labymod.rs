//! LabyMod 4 provider (`https://releases.r2.labymod.net`).
//!
//! The release channel's manifest names the current build (`commitReference`) and the Minecraft versions
//! it supports; each version has a full Mojang-format version JSON whose main class, arguments and Java
//! are used as they are. LabyMod's own libraries, its jar and per-OS natives are added to the classpath
//! the way LabyMod's launcher does it (nothing is filtered out except natives of other systems), and its
//! asset jars go to `<game dir>/labymod-neo/assets`. LabyMod loads mods from
//! `labymod-neo/fabric/<version>/mods`, which the launcher links to the instance's `mods`.
//! Everything LabyMod-specific stays in this file.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::error::{Error, Result};
use crate::meta::{MetaClient, VersionFile};
use crate::profile::library::{Artifact, GradleSpecifier, Library, LibraryDownloads, Rule, RuleAction, RuleOs};

use super::{ClientFiles, GameFile, InstalledMark};

/// Component uid; its version is the release channel.
pub const UID: &str = "net.labymod.labymod4";
pub const DEFAULT_CHANNEL: &str = "production";
const CHANNELS: [&str; 2] = ["production", "snapshot"];

const TERMS_URL: &str = "https://dl.labymod.net/latest/documents/tos.txt";

const API_URL: &str = "https://releases.r2.labymod.net/api/v1";
const JAR_URL: &str = "https://laby-releases.s3.de.io.cloud.ovh.net/api/v1/download/labymod4";
const ASSET_URL: &str = "https://releases.r2.labymod.net/api/v1/download/assets/labymod4";
/// How long a fetched manifest is trusted before the next launch asks for a newer build.
const MANIFEST_MAX_AGE: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    laby_mod_version: String,
    commit_reference: String,
    sha1: String,
    #[serde(default)]
    assets: BTreeMap<String, String>,
    minecraft_versions: Vec<ManifestVersion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestVersion {
    tag: String,
    version: String,
    custom_manifest_url: String,
}

#[derive(Debug, Deserialize)]
struct LibraryList {
    libraries: Vec<LabyLibrary>,
    #[serde(default)]
    isolated_libraries: Vec<IsolatedLibrary>,
}

#[derive(Debug, Deserialize)]
struct LabyLibrary {
    name: String,
    url: String,
    sha1: String,
    size: u64,
}

/// A library LabyMod loads on its own for the listed Minecraft versions, so the vanilla copy is dropped.
#[derive(Debug, Deserialize)]
struct IsolatedLibrary {
    name: String,
    versions: Vec<String>,
}

/// The parts of LabyMod's per-version JSON that are used.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VersionPatch {
    main_class: String,
    #[serde(default)]
    arguments: Arguments,
    #[serde(default)]
    libraries: Vec<Library>,
    java_version: Option<JavaVersion>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JavaVersion {
    component: String,
    major_version: u32,
}

#[derive(Debug, Default, Deserialize)]
struct Arguments {
    #[serde(default)]
    game: Vec<Argument>,
    #[serde(default)]
    jvm: Vec<Argument>,
}

/// Plain arguments are used; ones with rules (OS, launcher features) are handled by the launcher itself.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Argument {
    Plain(String),
    Ruled(IgnoredAny),
}

fn plain(arguments: Vec<Argument>) -> impl Iterator<Item = String> {
    arguments.into_iter().filter_map(|argument| match argument {
        Argument::Plain(value) => Some(value),
        Argument::Ruled(_) => None,
    })
}

/// LabyMod's terms of service, shown before its first use.
pub async fn terms(http: &reqwest::Client) -> Result<String> {
    let error = |source| Error::Http { url: TERMS_URL.to_owned(), source };
    let response = http.get(TERMS_URL).timeout(Duration::from_secs(15)).send().await.map_err(error)?;
    response.error_for_status().map_err(error)?.text().await.map_err(error)
}

fn check_name(what: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.contains(['/', '\\']) || value == ".." || value == "." {
        return Err(Error::Other(format!("invalid LabyMod {what} `{value}`")));
    }
    Ok(())
}

fn check_channel(channel: &str) -> Result<()> {
    if CHANNELS.contains(&channel) {
        Ok(())
    } else {
        Err(Error::Other(format!("unknown LabyMod channel `{channel}`")))
    }
}

fn installed_mark_path(meta: &MetaClient, channel: &str, minecraft_version: &str) -> PathBuf {
    meta.dir().join("labymod").join(channel).join(format!("installed-{minecraft_version}.txt"))
}

/// The newest build of the channel. A copy is kept per commit so an installed build can be resolved offline.
async fn latest(meta: &MetaClient, channel: &str) -> Result<Manifest> {
    let url = format!("{API_URL}/manifest/{channel}/latest.json");
    let bytes = meta.fetch_url(&url, &format!("labymod/{channel}/latest.json"), MANIFEST_MAX_AGE).await?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    check_name("commit", &manifest.commit_reference)?;
    let copy = meta.dir().join("labymod").join(channel).join(format!("manifest-{}.json", manifest.commit_reference));
    if !meta.is_offline() && !copy.exists() {
        tokio::fs::write(&copy, &bytes).await.map_err(|source| Error::Io { path: copy, source })?;
    }
    Ok(manifest)
}

/// Minecraft versions (`version`, as in Mojang's list) the channel's latest build supports.
pub async fn supported_versions(meta: &MetaClient, channel: &str) -> Result<Vec<String>> {
    check_channel(channel)?;
    let manifest = latest(meta, channel).await?;
    Ok(manifest.minecraft_versions.into_iter().map(|entry| entry.version).collect())
}

/// Resolves LabyMod for `minecraft_version`: the newest build when online, otherwise the last build that was
/// fully installed for this version.
pub async fn resolve(meta: &MetaClient, channel: &str, minecraft_version: &str) -> Result<VersionFile> {
    check_channel(channel)?;
    check_name("Minecraft version", minecraft_version)?;
    let mark_path = installed_mark_path(meta, channel, minecraft_version);

    let manifest = if meta.is_offline() {
        let commit = tokio::fs::read_to_string(&mark_path).await.map_err(|_| {
            Error::Other(format!("LabyMod for Minecraft {minecraft_version} is not installed and offline mode is on"))
        })?;
        check_name("commit", commit.trim())?;
        let path = meta.dir().join("labymod").join(channel).join(format!("manifest-{}.json", commit.trim()));
        let bytes = tokio::fs::read(&path).await.map_err(|source| Error::Io { path, source })?;
        serde_json::from_slice(&bytes)?
    } else {
        latest(meta, channel).await?
    };
    let commit = &manifest.commit_reference;
    let entry = manifest
        .minecraft_versions
        .iter()
        .find(|entry| entry.version == minecraft_version)
        .ok_or_else(|| Error::Other(format!("LabyMod 4 does not support Minecraft {minecraft_version}")))?;
    check_name("version tag", &entry.tag)?;
    if !entry.custom_manifest_url.starts_with("https://releases.r2.labymod.net/") {
        return Err(Error::Other(format!("unexpected LabyMod version URL {}", entry.custom_manifest_url)));
    }

    // Both files are immutable for a commit (the library list is saved per commit as LabyMod's launcher does).
    let patch = meta
        .fetch_url(&entry.custom_manifest_url, &format!("labymod/{channel}/{}-{commit}.json", entry.tag), Duration::MAX)
        .await?;
    let libraries = meta
        .fetch_url(
            &format!("{API_URL}/libraries/{channel}.json"),
            &format!("labymod/{channel}/libraries-{commit}.json"),
            Duration::MAX,
        )
        .await?;
    build(
        &manifest,
        &serde_json::from_slice(&libraries)?,
        serde_json::from_slice(&patch)?,
        channel,
        minecraft_version,
        mark_path,
    )
}

/// Native libraries carry the system in their name (`...-natives-linux`, classifier `natives-windows`);
/// the rule keeps the other systems' ones out of the classpath.
fn native_rules(name: &GradleSpecifier) -> Vec<Rule> {
    let full = match &name.classifier {
        Some(classifier) => format!("{}-{classifier}", name.artifact),
        None => name.artifact.clone(),
    };
    let Some((_, system)) = full.split_once("natives-") else { return Vec::new() };
    let os = match system {
        "linux" | "windows" => system,
        "macos" => "osx",
        "macos-arm64" => "osx-arm64",
        _ => return Vec::new(),
    };
    vec![Rule { action: RuleAction::Allow, os: Some(RuleOs { name: Some(os.to_owned()) }) }]
}

fn download_library(name: GradleSpecifier, url: String, sha1: String, size: Option<u64>) -> Library {
    let rules = native_rules(&name);
    Library {
        name,
        downloads: Some(LibraryDownloads {
            artifact: Some(Artifact { url, sha1: Some(sha1), size }),
            classifiers: Default::default(),
        }),
        natives: Default::default(),
        rules,
        url: None,
    }
}

fn build(
    manifest: &Manifest,
    list: &LibraryList,
    patch: VersionPatch,
    channel: &str,
    minecraft_version: &str,
    mark_path: PathBuf,
) -> Result<VersionFile> {
    let commit = &manifest.commit_reference;
    let isolated = |library: &Library| {
        library.name.group.starts_with("org.lwjgl")
            && list.isolated_libraries.iter().any(|entry| {
                entry.name == library.name.artifact && entry.versions.iter().any(|version| version == minecraft_version)
            })
    };
    let mut libraries: Vec<Library> = patch.libraries.into_iter().filter(|library| !isolated(library)).collect();
    for library in &list.libraries {
        let name = library.name.parse().map_err(Error::Other)?;
        libraries.push(download_library(name, library.url.clone(), library.sha1.clone(), Some(library.size)));
    }
    // The build number can stay the same between commits, so the commit is part of the version: a jar that
    // is already on disk is trusted by its path.
    let jar = GradleSpecifier {
        group: "net.labymod".to_owned(),
        artifact: "LabyMod".to_owned(),
        version: format!("{}-{commit}", manifest.laby_mod_version),
        classifier: None,
        extension: "jar".to_owned(),
    };
    libraries.push(download_library(
        jar,
        format!("{JAR_URL}/{channel}/{commit}.jar"),
        manifest.sha1.clone(),
        // The manifest's size is not exact; the hash is the check.
        None,
    ));

    let mut files = Vec::with_capacity(manifest.assets.len());
    for (name, sha1) in &manifest.assets {
        check_name("asset", name)?;
        files.push(GameFile {
            path: format!("labymod-neo/assets/{name}.jar"),
            url: format!("{ASSET_URL}/{channel}/{commit}/{name}/{sha1}.jar"),
            sha1: sha1.clone(),
        });
    }

    // Arguments handled by the launcher itself (library path, classpath) are not repeated.
    let jvm_args = plain(patch.arguments.jvm)
        .filter(|arg| {
            !(arg == "-cp" || arg == "${classpath}" || arg.starts_with("-Djava.library.path="))
        })
        .collect();
    let (compatible_java_majors, compatible_java_name) = match patch.java_version {
        Some(java) => (vec![java.major_version], Some(java.component)),
        None => (Vec::new(), None),
    };

    Ok(VersionFile {
        uid: UID.to_owned(),
        version: channel.to_owned(),
        kind: None,
        volatile: false,
        requires: Vec::new(),
        libraries,
        maven_files: Vec::new(),
        main_class: Some(patch.main_class),
        main_jar: None,
        minecraft_arguments: Some(plain(patch.arguments.game).collect::<Vec<_>>().join(" ")),
        jvm_args,
        tweakers: Vec::new(),
        traits: Vec::new(),
        asset_index: None,
        compatible_java_majors,
        compatible_java_name,
        logging: None,
        client_files: ClientFiles {
            files,
            mark: Some(InstalledMark { path: mark_path, contents: commit.clone() }),
            mods_dir: Some(format!("labymod-neo/fabric/{minecraft_version}/mods")),
        },
    })
}

#[cfg(test)]
mod tests {
    use crate::client;
    use crate::component::{ComponentSpec, MINECRAFT_UID};
    use crate::profile::library::RuntimeContext;
    use crate::profile::{LaunchProfile, ResolvedComponent};

    use super::*;

    const MANIFEST: &str = include_str!("../../tests/fixtures/labymod/latest.json");
    const LIBRARIES: &str = include_str!("../../tests/fixtures/labymod/libraries.json");
    const PATCH_1_21_11: &str = include_str!("../../tests/fixtures/labymod/1.21.11.json");
    const PATCH_1_8_9: &str = include_str!("../../tests/fixtures/labymod/1.8.9.json");

    fn version_file(patch: &str, minecraft_version: &str) -> VersionFile {
        build(
            &serde_json::from_str(MANIFEST).unwrap(),
            &serde_json::from_str(LIBRARIES).unwrap(),
            serde_json::from_str(patch).unwrap(),
            "production",
            minecraft_version,
            PathBuf::from("mark.txt"),
        )
        .unwrap()
    }

    /// What Prism meta gives for `net.minecraft`: it brings its own libraries and an LWJGL 2 requirement.
    fn prism_minecraft(version: &str) -> ResolvedComponent {
        let file: VersionFile = serde_json::from_str(&format!(
            r#"{{"uid":"net.minecraft","version":"{version}","type":"release","mainClass":"net.minecraft.client.main.Main",
                "minecraftArguments":"--vanilla","+jvmArgs":["-Dvanilla=1"],"compatibleJavaMajors":[8],
                "requires":[{{"uid":"org.lwjgl"}}],"+traits":["feature:is_quick_play_multiplayer"],
                "libraries":[{{"name":"org.lwjgl.lwjgl:lwjgl:2.9.4"}},{{"name":"com.mojang:vanilla:1"}}],
                "mainJar":{{"name":"com.mojang:minecraft:{version}:client","downloads":{{"artifact":{{"url":"https://x/c.jar","sha1":"aa"}}}}}},
                "assetIndex":{{"id":"1.8","url":"https://x/1.8.json"}}}}"#
        ))
        .unwrap();
        ResolvedComponent {
            spec: ComponentSpec { uid: MINECRAFT_UID.to_owned(), version: version.to_owned(), dependency_only: false },
            file,
        }
    }

    fn profile(patch: &str, minecraft_version: &str, runtime: RuntimeContext) -> LaunchProfile {
        let laby = ResolvedComponent {
            spec: ComponentSpec { uid: UID.to_owned(), version: "production".to_owned(), dependency_only: false },
            file: version_file(patch, minecraft_version),
        };
        let mut components = vec![prism_minecraft(minecraft_version), laby];
        client::apply(&mut components);
        LaunchProfile::build(&components, runtime).unwrap()
    }

    fn classpath(profile: &LaunchProfile) -> Vec<String> {
        profile.classpath_files().into_iter().map(|file| file.path).collect()
    }

    #[test]
    fn launches_through_launchwrapper_with_labymods_arguments() {
        let profile = profile(PATCH_1_21_11, "1.21.11", RuntimeContext::new("linux", "amd64"));

        assert_eq!(profile.main_class, "net.minecraft.launchwrapper.Launch");
        assert!(profile.jvm_args.contains(&"-Dnet.labymod.running-version=1.21.11".to_owned()));
        assert!(profile.jvm_args.contains(&"--add-opens=java.base/java.net=ALL-UNNAMED".to_owned()));
        assert!(profile.jvm_args.contains(&"-Djna.tmpdir=${natives_directory}".to_owned()));
        assert!(!profile.jvm_args.iter().any(|arg| arg == "-cp" || arg == "${classpath}" || arg == "-Dvanilla=1"));
        assert!(!profile.jvm_args.iter().any(|arg| arg.starts_with("-Djava.library.path")));
        assert!(profile.minecraft_arguments.ends_with("--dummyMinecraftClass net.minecraft.client.main.Main"));
        assert!(!profile.minecraft_arguments.contains("--vanilla"));
        assert!(!profile.minecraft_arguments.contains("--demo"));
        assert_eq!(profile.compatible_java_majors, [21]);
        assert_eq!(profile.compatible_java_name.as_deref(), Some("java-runtime-delta"));
    }

    #[test]
    fn keeps_the_minecraft_jar_assets_and_traits_but_not_vanilla_libraries() {
        let profile = profile(PATCH_1_8_9, "1.8.9", RuntimeContext::new("linux", "amd64"));
        let classpath = classpath(&profile);

        assert!(classpath.iter().any(|path| path.starts_with("net/labymod/LabyMod/48-a80b82de/")), "{classpath:?}");
        assert_eq!(classpath.last().unwrap(), "com/mojang/minecraft/1.8.9/minecraft-1.8.9-client.jar");
        assert!(classpath.iter().any(|path| path.contains("launchwrapper")));
        assert!(!classpath.iter().any(|path| path.contains("lwjgl/lwjgl/2.9.4") || path.contains("vanilla")));
        assert_eq!(profile.asset_index.id, "1.8");
        assert!(profile.has_trait("feature:is_quick_play_multiplayer"));
        assert_eq!(profile.minecraft_version, "1.8.9");
        // Java 21 from LabyMod, not the Java 8 of vanilla 1.8.9.
        assert_eq!(profile.compatible_java_majors, [21]);
    }

    #[test]
    fn natives_of_other_systems_are_left_out() {
        let linux = classpath(&profile(PATCH_1_21_11, "1.21.11", RuntimeContext::new("linux", "amd64")));
        let windows = classpath(&profile(PATCH_1_21_11, "1.21.11", RuntimeContext::new("windows", "amd64")));

        assert!(linux.iter().any(|path| path.ends_with("imgui-java-natives-linux-1.0.2.jar")));
        assert!(!linux.iter().any(|path| path.contains("natives-windows") || path.contains("natives-macos")));
        assert!(linux.iter().any(|path| path.ends_with("lwjgl-vma-3.4.3-natives-linux.jar")));
        assert!(windows.iter().any(|path| path.ends_with("imgui-java-natives-windows-1.0.2.jar")));
        assert!(!windows.iter().any(|path| path.contains("natives-linux") || path.contains("natives-macos")));
        // Plain libraries are on both.
        assert!(linux.iter().any(|path| path.contains("sponge-mixin")) && windows.iter().any(|path| path.contains("sponge-mixin")));
    }

    #[test]
    fn jar_and_assets_are_verified_by_hash() {
        let profile = profile(PATCH_1_21_11, "1.21.11", RuntimeContext::new("linux", "amd64"));
        let jar = profile
            .library_files()
            .into_iter()
            .find(|file| file.path.starts_with("net/labymod/LabyMod/"))
            .unwrap();
        assert_eq!(jar.sha1.as_deref(), Some("517d2c2b40e64be49ef43ae6a70804ba275b4e5b"));
        assert_eq!(jar.size, None);
        assert_eq!(
            jar.url,
            "https://laby-releases.s3.de.io.cloud.ovh.net/api/v1/download/labymod4/production/a80b82de.jar"
        );

        let assets = &profile.client_files.files;
        assert_eq!(assets.len(), 6);
        let shader = assets.iter().find(|file| file.path == "labymod-neo/assets/shader.jar").unwrap();
        assert_eq!(shader.sha1, "1fea40a4953fead005a921a8fb5561e72c010427");
        assert_eq!(
            shader.url,
            "https://releases.r2.labymod.net/api/v1/download/assets/labymod4/production/a80b82de/shader/1fea40a4953fead005a921a8fb5561e72c010427.jar"
        );
        assert_eq!(profile.client_files.mark.as_ref().unwrap().contents, "a80b82de");
    }

    #[test]
    fn isolated_lwjgl_libraries_are_dropped_from_the_version_json() {
        let mut patch: serde_json::Value = serde_json::from_str(PATCH_1_21_11).unwrap();
        patch["libraries"].as_array_mut().unwrap().extend([
            serde_json::json!({"name": "org.lwjgl:lwjgl-glfw:3.3.3"}),
            serde_json::json!({"name": "org.lwjgl:lwjgl-nanovg:3.3.3"}),
        ]);
        let profile = profile(&patch.to_string(), "1.21.11", RuntimeContext::new("linux", "amd64"));
        let names: Vec<String> = profile.libraries.iter().map(|library| library.name.artifact.clone()).collect();
        assert!(!names.contains(&"lwjgl-glfw".to_owned()));
        assert!(names.contains(&"lwjgl-nanovg".to_owned()));
    }

    #[tokio::test]
    async fn offline_resolving_uses_the_installed_build_and_only_that() {
        let root = tempfile::tempdir().unwrap();
        let paths = crate::paths::Paths::new(root.path());
        let dir = paths.meta_dir().join("labymod/production");
        std::fs::create_dir_all(&dir).unwrap();
        let offline = MetaClient::new(&paths, true);
        let error = resolve(&offline, "production", "1.21.11").await.unwrap_err();
        assert!(error.to_string().contains("not installed"), "{error}");

        std::fs::write(dir.join("installed-1.21.11.txt"), "a80b82de").unwrap();
        std::fs::write(dir.join("manifest-a80b82de.json"), MANIFEST).unwrap();
        std::fs::write(dir.join("1.21.11-a80b82de.json"), PATCH_1_21_11).unwrap();
        std::fs::write(dir.join("libraries-a80b82de.json"), LIBRARIES).unwrap();
        let file = resolve(&offline, "production", "1.21.11").await.unwrap();
        assert_eq!(file.main_class.as_deref(), Some("net.minecraft.launchwrapper.Launch"));
        assert!(resolve(&offline, "production", "1.7.10").await.is_err());
        assert!(resolve(&offline, "nightly", "1.21.11").await.is_err());
    }

    #[test]
    fn only_manifest_versions_are_supported_by_version_not_tag() {
        let manifest: Manifest = serde_json::from_str(MANIFEST).unwrap();
        assert!(manifest.minecraft_versions.iter().any(|entry| entry.version == "1.21.11"));
        assert!(manifest.minecraft_versions.iter().any(|entry| entry.tag == "1.8" && entry.version == "1.8.9"));
    }
}
