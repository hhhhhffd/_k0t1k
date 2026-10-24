//! Java management: probing, detection, managed runtimes and choosing a Java for a launch.

mod detect;
mod probe;
mod runtime;
mod version;

pub use detect::detect_java;
pub use probe::{JavaInstall, POISONED_ENV, probe};
pub use runtime::{component_for_major, install_runtime, managed_java_path, mojang_platform, remove_leftover_staging};
pub use version::JavaVersion;

use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::progress::ProgressFn;
use crate::settings::JavaSettings;

/// Java the game is started with when nothing else is known (Minecraft without `javaVersion` info).
const FALLBACK_MAJOR: u32 = 8;

/// File name of the game executable: `javaw.exe` has no console window, unlike `java.exe`.
pub const fn game_executable() -> &'static str {
    if cfg!(windows) { "javaw.exe" } else { "java" }
}

/// Mojang runtime component a launch with these requirements uses when Java is picked automatically.
/// Without requirement info Java 8 is assumed.
pub fn required_component<'a>(required_majors: &[u32], component: Option<&'a str>) -> Option<&'a str> {
    let majors = if required_majors.is_empty() { &[FALLBACK_MAJOR][..] } else { required_majors };
    component.or_else(|| majors.iter().find_map(|&major| component_for_major(major)))
}

/// Choose the Java for a launch, in this order:
/// 1. the explicit path from the settings (it must work; no silent fallback),
/// 2. the managed runtime for `component`,
/// 3. a fresh download of the runtime when `auto_download` is on (and a component is known),
/// 4. otherwise a detected install whose major is in `required_majors`.
///
/// A system Java is deliberately not preferred over the download, so the launcher stays self-contained.
///
/// Without requirement info Java 8 is assumed. The result is never written back into settings.
pub async fn ensure_java(
    paths: &Paths,
    client: &reqwest::Client,
    required_majors: &[u32],
    component: Option<&str>,
    settings: &JavaSettings,
    progress: &ProgressFn,
) -> Result<JavaInstall> {
    if let Some(path) = &settings.path {
        return probe(path)
            .await
            .map_err(|e| Error::Other(format!("configured Java is unusable: {e}")));
    }
    if !settings.auto_select {
        return Err(Error::Other(
            "no Java path is configured and automatic Java selection is disabled".to_owned(),
        ));
    }

    let majors = if required_majors.is_empty() {
        &[FALLBACK_MAJOR][..]
    } else {
        required_majors
    };
    let component = required_component(required_majors, component);

    if let Some(component) = component
        && let Some(path) = managed_java_path(paths, component)
    {
        match probe(&path).await {
            Ok(install) => return Ok(install),
            Err(error) => tracing::warn!("managed Java {component} is broken, looking elsewhere: {error}"),
        }
    }

    if settings.auto_download
        && let Some(component) = component
    {
        let path = install_runtime(paths, client, component, progress).await?;
        return probe(&path).await;
    }

    detect_java(paths)
        .await
        .into_iter()
        .find(|install| majors.contains(&install.major))
        .ok_or_else(|| {
            Error::Other(format!(
                "no installed Java matches the required major version(s) {majors:?} and automatic download is unavailable"
            ))
        })
}

#[cfg(all(test, unix))]
pub(crate) mod test_support {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    /// An executable shell script that prints the property block a real `java` would.
    pub fn write_fake_java(path: &Path, version: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let script = format!(
            "#!/bin/sh\necho '    java.version = {version}' >&2\necho '    java.vendor = Fake' >&2\necho '    os.arch = amd64' >&2\n"
        );
        std::fs::write(path, script).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::test_support::write_fake_java;
    use super::*;
    use crate::download::http_client;
    use crate::progress::no_progress;

    async fn ensure(paths: &Paths, majors: &[u32], component: Option<&str>, settings: &JavaSettings) -> Result<JavaInstall> {
        ensure_java(paths, &http_client(), majors, component, settings, &no_progress()).await
    }

    fn offline() -> JavaSettings {
        JavaSettings {
            auto_download: false,
            ..JavaSettings::default()
        }
    }

    #[tokio::test]
    async fn explicit_path_wins_and_must_work() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let java = dir.path().join("mine/bin/java");
        write_fake_java(&java, "11.0.2");
        write_fake_java(&paths.java_dir().join("java-runtime-delta/bin/java"), "21.0.7");

        let mut settings = offline();
        settings.path = Some(java.clone());
        let chosen = ensure(&paths, &[21], Some("java-runtime-delta"), &settings).await.unwrap();
        assert_eq!(chosen.path, java);

        settings.path = Some(dir.path().join("missing"));
        assert!(ensure(&paths, &[21], None, &settings).await.is_err());
    }

    #[tokio::test]
    async fn prefers_managed_component() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        write_fake_java(&paths.java_dir().join("java-runtime-delta/bin/java"), "21.0.7");

        let chosen = ensure(&paths, &[21], Some("java-runtime-delta"), &offline()).await.unwrap();
        assert_eq!(chosen.major, 21);
        assert!(chosen.path.starts_with(paths.java_dir()));
    }

    #[tokio::test]
    async fn uses_compatible_detected_java_when_download_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        // Not the requested component, but a managed Java 17 satisfies [17].
        write_fake_java(&paths.java_dir().join("other/bin/java"), "17.0.15");

        let chosen = ensure(&paths, &[17], Some("java-runtime-gamma"), &offline()).await.unwrap();
        assert_eq!(chosen.major, 17);
    }

    #[tokio::test]
    async fn no_requirements_means_java_8() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        write_fake_java(&paths.java_dir().join("jre-legacy/bin/java"), "1.8.0_51");
        write_fake_java(&paths.java_dir().join("java-runtime-delta/bin/java"), "21.0.7");

        let chosen = ensure(&paths, &[], None, &offline()).await.unwrap();
        assert_eq!(chosen.major, 8);
    }

    #[tokio::test]
    async fn reports_error_when_nothing_matches_and_download_is_off() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        // Major 99 has no Mojang component, and no installed Java has it.
        assert!(ensure(&paths, &[99], None, &JavaSettings::default()).await.is_err());
    }

    #[tokio::test]
    async fn disabled_auto_select_without_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let settings = JavaSettings {
            auto_select: false,
            ..JavaSettings::default()
        };
        assert!(ensure(&Paths::new(dir.path()), &[], None, &settings).await.is_err());
    }

    /// Hits the network: installs the real runtime for this platform and runs it.
    #[tokio::test]
    #[ignore = "downloads java-runtime-delta from Mojang"]
    async fn installs_and_probes_java_runtime_delta() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let path = install_runtime(&paths, &http_client(), "java-runtime-delta", &no_progress())
            .await
            .unwrap();
        let install = probe(&path).await.unwrap();
        assert_eq!(install.major, 21);
        assert_eq!(managed_java_path(&paths, "java-runtime-delta"), Some(path));
        assert!(!paths.java_dir().join(".tmp").read_dir().is_ok_and(|mut d| d.next().is_some()));
    }
}
