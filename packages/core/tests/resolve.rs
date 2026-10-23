//! Component resolution and profile merging against real Prism meta files saved in `tests/fixtures/meta`.

use std::path::Path;

use k0t1k_core::component::ComponentSpec;
use k0t1k_core::meta::MetaClient;
use k0t1k_core::paths::Paths;
use k0t1k_core::profile::library::RuntimeContext;
use k0t1k_core::profile::{LaunchProfile, ResolvedComponent, resolve_components};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// An offline meta client whose cache holds the fixtures.
fn fixture_client() -> (tempfile::TempDir, MetaClient) {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path());
    copy_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/meta"), &paths.meta_dir());
    (root, MetaClient::new(&paths, true))
}

fn spec(uid: &str, version: &str) -> ComponentSpec {
    ComponentSpec { uid: uid.to_owned(), version: version.to_owned(), dependency_only: false }
}

fn summary(components: &[ResolvedComponent]) -> Vec<(&str, &str, bool)> {
    components.iter().map(|c| (c.spec.uid.as_str(), c.spec.version.as_str(), c.spec.dependency_only)).collect()
}

fn linux() -> RuntimeContext {
    RuntimeContext::new("linux", "amd64")
}

#[tokio::test]
async fn vanilla_gets_the_suggested_lwjgl_before_minecraft() {
    let (_root, meta) = fixture_client();
    let resolved = resolve_components(&meta, vec![spec("net.minecraft", "1.21.4")]).await.unwrap();
    assert_eq!(summary(&resolved), [("org.lwjgl3", "3.3.3", true), ("net.minecraft", "1.21.4", false)]);
}

#[tokio::test]
async fn fabric_gets_intermediary_for_the_minecraft_version() {
    let (_root, meta) = fixture_client();
    let specs = vec![spec("net.minecraft", "1.21.4"), spec("net.fabricmc.fabric-loader", "0.16.9")];
    let resolved = resolve_components(&meta, specs).await.unwrap();
    assert_eq!(
        summary(&resolved),
        [
            ("org.lwjgl3", "3.3.3", true),
            ("net.minecraft", "1.21.4", false),
            ("net.fabricmc.intermediary", "1.21.4", true),
            ("net.fabricmc.fabric-loader", "0.16.9", false),
        ]
    );

    let profile = LaunchProfile::build(&resolved, linux()).unwrap();
    assert_eq!(profile.main_class, "net.fabricmc.loader.impl.launch.knot.KnotClient");
    assert_eq!(profile.minecraft_version, "1.21.4");
    assert_eq!(profile.asset_index.id, "19");
    assert_eq!(profile.compatible_java_majors, [21]);
    assert_eq!(profile.compatible_java_name.as_deref(), Some("java-runtime-delta"));
    let classpath: Vec<_> = profile.classpath_files().into_iter().map(|file| file.path).collect();
    assert!(classpath.contains(&"net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar".to_owned()));
    assert!(classpath.contains(&"net/fabricmc/intermediary/1.21.4/intermediary-1.21.4.jar".to_owned()));
    assert_eq!(classpath.last().unwrap(), "com/mojang/minecraft/1.21.4/minecraft-1.21.4-client.jar");
    assert!(profile.library_files().iter().all(|file| file.url.starts_with("https://")));
}

#[tokio::test]
async fn resolution_is_stable_when_run_again_on_its_own_output() {
    let (_root, meta) = fixture_client();
    let first =
        resolve_components(&meta, vec![spec("net.minecraft", "1.21.4"), spec("net.fabricmc.fabric-loader", "0.16.9")])
            .await
            .unwrap();
    let specs = first.iter().map(|component| component.spec.clone()).collect();
    let second = resolve_components(&meta, specs).await.unwrap();
    assert_eq!(summary(&first), summary(&second));
}

#[tokio::test]
async fn unused_volatile_dependency_is_dropped() {
    let (_root, meta) = fixture_client();
    let stale = ComponentSpec { dependency_only: true, ..spec("net.fabricmc.intermediary", "1.21.4") };
    let resolved = resolve_components(&meta, vec![spec("net.minecraft", "1.21.4"), stale]).await.unwrap();
    assert_eq!(summary(&resolved), [("org.lwjgl3", "3.3.3", true), ("net.minecraft", "1.21.4", false)]);
}

#[tokio::test]
async fn forge_resolves_with_forgewrapper_and_installer_maven_file() {
    let (_root, meta) = fixture_client();
    let specs = vec![spec("net.minecraft", "1.20.1"), spec("net.minecraftforge", "47.4.28")];
    let resolved = resolve_components(&meta, specs).await.unwrap();
    assert_eq!(
        summary(&resolved),
        [("org.lwjgl3", "3.3.1", true), ("net.minecraft", "1.20.1", false), ("net.minecraftforge", "47.4.28", false),]
    );

    let profile = LaunchProfile::build(&resolved, linux()).unwrap();
    assert_eq!(profile.main_class, "io.github.zekerzhayard.forgewrapper.installer.Main");
    assert!(profile.minecraft_arguments.contains("--launchTarget forgeclient"));
    assert!(profile.compatible_java_majors.contains(&17));

    let libraries: Vec<_> = profile.library_files().into_iter().map(|file| file.path).collect();
    let installer = "net/minecraftforge/forge/1.20.1-47.4.28/forge-1.20.1-47.4.28-installer.jar";
    assert!(libraries.contains(&installer.to_owned()));
    assert!(
        !profile.classpath_files().iter().any(|file| file.path == installer),
        "maven files must not be on the classpath"
    );
    assert_eq!(profile.maven_files[0].name.to_string(), "net.minecraftforge:forge:1.20.1-47.4.28:installer");
    let wrapper = "io/github/zekerzhayard/ForgeWrapper/prism-2026-08-01/ForgeWrapper-prism-2026-08-01.jar";
    assert!(profile.classpath_files().iter().any(|file| file.path == wrapper));
}

#[tokio::test]
async fn forge_for_another_minecraft_version_is_a_conflict() {
    let (_root, meta) = fixture_client();
    let specs = vec![spec("net.minecraft", "1.21.4"), spec("net.minecraftforge", "47.4.28")];
    let error = resolve_components(&meta, specs).await.unwrap_err();
    assert!(error.to_string().contains("conflicting dependencies"), "{error}");
}

#[tokio::test]
async fn natives_for_other_systems_are_not_in_the_linux_profile() {
    let (_root, meta) = fixture_client();
    let resolved = resolve_components(&meta, vec![spec("net.minecraft", "1.21.4")]).await.unwrap();

    let windows = LaunchProfile::build(&resolved, RuntimeContext::new("windows", "amd64")).unwrap();
    let linux = LaunchProfile::build(&resolved, linux()).unwrap();
    let urls = |profile: &LaunchProfile| -> Vec<String> {
        profile.classpath_files().into_iter().map(|file| file.url).collect()
    };
    assert!(urls(&windows).iter().any(|url| url.ends_with("lwjgl-glfw-3.3.3-natives-windows.jar")));
    assert!(!urls(&windows).iter().any(|url| url.ends_with("natives-linux.jar")));
    assert!(urls(&linux).iter().any(|url| url.ends_with("lwjgl-glfw-3.3.3-natives-linux.jar")));
    assert!(!urls(&linux).iter().any(|url| url.ends_with("natives-windows.jar")));
}
