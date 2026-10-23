use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::JavaVersion;
use crate::error::{Error, Result};

const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// Variables that change which Java runs or how it behaves; dropped for probing and launching.
pub const POISONED_ENV: [&str; 8] = [
    "JAVA_ARGS",
    "CLASSPATH",
    "CONFIGPATH",
    "JAVA_HOME",
    "JRE_HOME",
    "_JAVA_OPTIONS",
    "JAVA_OPTIONS",
    "JAVA_TOOL_OPTIONS",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaInstall {
    /// Executable used to start the game (`javaw.exe` on Windows).
    pub path: PathBuf,
    pub version: JavaVersion,
    pub major: u32,
    /// Raw `os.arch`, e.g. `amd64`.
    pub arch: String,
    pub vendor: String,
    pub is_64bit: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct ProbeOutput {
    version: String,
    vendor: String,
    arch: String,
}

/// Run `java -XshowSettings:properties -version` and describe the installation.
/// On Windows `javaw.exe` is probed through the `java.exe` next to it, because
/// `javaw` has no console to print to.
pub async fn probe(java: &Path) -> Result<JavaInstall> {
    let runner = console_executable(java);
    let mut command = Command::new(&runner);
    command
        .args(["-XshowSettings:properties", "-version"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for name in POISONED_ENV {
        command.env_remove(name);
    }
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

    let failed = |reason: String| Error::Other(format!("Java probe of {} failed: {reason}", java.display()));

    let output = tokio::time::timeout(PROBE_TIMEOUT, command.output())
        .await
        .map_err(|_| failed(format!("no answer within {} s", PROBE_TIMEOUT.as_secs())))?
        .map_err(|e| failed(e.to_string()))?;
    if !output.status.success() {
        return Err(failed(format!("exited with {}", output.status)));
    }

    // The JVM prints its settings to stderr; stdout is included in case a wrapper redirects it.
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let parsed = parse_properties(&text).ok_or_else(|| failed("output has no java.version/os.arch".to_owned()))?;
    let version: JavaVersion = parsed.version.parse()?;

    Ok(JavaInstall {
        path: java.to_owned(),
        major: version.major,
        version,
        is_64bit: is_64bit_arch(&parsed.arch),
        arch: parsed.arch,
        vendor: parsed.vendor,
    })
}

fn console_executable(java: &Path) -> PathBuf {
    if cfg!(windows) && java.file_name().is_some_and(|n| n.eq_ignore_ascii_case("javaw.exe")) {
        let console = java.with_file_name("java.exe");
        if console.is_file() {
            return console;
        }
    }
    java.to_owned()
}

fn is_64bit_arch(arch: &str) -> bool {
    matches!(
        arch,
        "x86_64" | "amd64" | "aarch64" | "arm64" | "riscv64" | "ppc64le" | "ppc64"
    )
}

/// Pick the three properties out of the `Property settings:` block (`    key = value` lines).
/// A missing vendor is tolerated; version and arch are required.
fn parse_properties(text: &str) -> Option<ProbeOutput> {
    let (mut version, mut vendor, mut arch) = (None, None, None);
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once(" = ") else {
            continue;
        };
        match key {
            "java.version" => version = Some(value.trim().to_owned()),
            "java.vendor" => vendor = Some(value.trim().to_owned()),
            "os.arch" => arch = Some(value.trim().to_owned()),
            _ => {}
        }
    }
    Some(ProbeOutput {
        version: version?,
        vendor: vendor.unwrap_or_default(),
        arch: arch?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Property settings:
    file.encoding = UTF-8
    java.class.path =
    java.specification.version = 21
    java.vendor = Microsoft
    java.vendor.url = https://www.microsoft.com
    java.version = 21.0.7
    java.version.date = 2025-04-15
    os.arch = amd64
    os.name = Windows 11

openjdk version \"21.0.7\" 2025-04-15 LTS
OpenJDK Runtime Environment Microsoft-11369603 (build 21.0.7+6-LTS)
";

    #[test]
    fn parses_sample_output() {
        assert_eq!(
            parse_properties(SAMPLE),
            Some(ProbeOutput {
                version: "21.0.7".to_owned(),
                vendor: "Microsoft".to_owned(),
                arch: "amd64".to_owned(),
            })
        );
    }

    #[test]
    fn missing_required_keys_yield_none() {
        assert_eq!(parse_properties("    java.vendor = X\n    os.arch = amd64"), None);
        assert_eq!(parse_properties("garbage"), None);
    }

    #[test]
    fn architecture_classification() {
        assert!(is_64bit_arch("amd64") && is_64bit_arch("aarch64"));
        assert!(!is_64bit_arch("x86") && !is_64bit_arch("i386") && !is_64bit_arch("arm"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn probes_a_fake_java() {
        let dir = tempfile::tempdir().unwrap();
        let java = dir.path().join("bin/java");
        crate::java::test_support::write_fake_java(&java, "1.8.0_392");
        let install = probe(&java).await.unwrap();
        assert_eq!(install.major, 8);
        assert_eq!(install.vendor, "Fake");
        assert!(install.is_64bit);
        assert_eq!(install.path, java);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn failing_java_is_an_error() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let java = dir.path().join("java");
        std::fs::write(&java, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&java, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(probe(&java).await.is_err());
        assert!(probe(&dir.path().join("missing")).await.is_err());
    }
}
