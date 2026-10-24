//! Builds the Java command line for a [`LaunchProfile`] and runs the game.

mod account;
mod process;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::error::{Error, IoContext, Result};
use crate::install::AssetIndex;
use crate::paths::Paths;
use crate::profile::LaunchProfile;

pub use account::{Account, offline_uuid};
pub use process::{GameHandle, LogFn, LogLine, LogStream, spawn_game};

/// Variables that would inject JVM options or pick another Java behind the launcher's back.
const DROPPED_ENV: [&str; 6] =
    ["JAVA_TOOL_OPTIONS", "_JAVA_OPTIONS", "JAVA_OPTIONS", "CLASSPATH", "JAVA_HOME", "JRE_HOME"];

/// Windows rejects command lines longer than 32767 characters; `@argfile` support starts with Java 9.
const ARGFILE_MIN_JAVA_MAJOR: u32 = 9;

const CLASSPATH_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone)]
pub struct LaunchOptions {
    pub java_path: PathBuf,
    pub java_major: u32,
    /// Working directory of the game and `${game_directory}`.
    pub game_dir: PathBuf,
    /// Where [`crate::install::extract_natives`] extracted to; `-Djava.library.path`.
    pub natives_dir: PathBuf,
    /// `${profile_name}`, the instance name.
    pub profile_name: String,
    pub min_memory_mb: u32,
    pub max_memory_mb: u32,
    /// `-javaagent:...` arguments (authlib-injector); placed right after `-Duser.language=en`, before
    /// everything else.
    pub java_agent_args: Vec<String>,
    /// Placed after the Java agents and before the profile's own JVM arguments.
    pub extra_jvm_args: Vec<String>,
    /// Passed as `--width`/`--height`; `None` leaves the game's default window.
    pub window: Option<WindowSize>,
    pub account: Account,
    /// Extra environment variables for the game process.
    pub env: BTreeMap<String, String>,
    /// Wrapper command (program and arguments) that is started instead of Java and gets the Java
    /// command line appended. Empty for none.
    pub wrapper: Vec<String>,
}

#[derive(Clone)]
pub struct LaunchCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}

impl LaunchCommand {
    /// The command line for logs, with the access token masked.
    pub fn display_redacted(&self) -> String {
        let mut parts = vec![self.program.display().to_string()];
        let mut mask_next = false;
        for arg in &self.args {
            parts.push(if std::mem::take(&mut mask_next) { "***".to_owned() } else { arg.clone() });
            mask_next = arg == "--accessToken";
        }
        parts.join(" ")
    }
}

/// Java arguments split the way they are passed: everything up to and including the main class
/// (which may go into an argfile) and the game arguments after it.
#[derive(Debug, PartialEq, Eq)]
struct LaunchArgs {
    jvm: Vec<String>,
    game: Vec<String>,
}

/// Replaces `${key}` with its value; an unknown key becomes an empty string.
fn substitute(arg: &str, tokens: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(arg.len());
    let mut rest = arg;
    while let Some(start) = rest.find("${") {
        let Some(length) = rest[start..].find('}') else { break };
        out.push_str(&rest[..start]);
        let key = &rest[start + 2..start + length];
        if let Some(value) = tokens.get(key) {
            out.push_str(value);
        }
        rest = &rest[start + length + 1..];
    }
    out.push_str(rest);
    out
}

fn path_string(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

fn launch_args(paths: &Paths, profile: &LaunchProfile, assets: &AssetIndex, options: &LaunchOptions) -> LaunchArgs {
    let account = &options.account;
    let tokens: HashMap<&str, String> = HashMap::from([
        ("auth_player_name", account.name.clone()),
        ("auth_uuid", account.uuid.clone()),
        ("auth_access_token", account.access_token.clone()),
        ("auth_session", account.session.clone()),
        ("user_type", account.user_type.clone()),
        ("user_properties", "{}".to_owned()),
        ("profile_name", options.profile_name.clone()),
        ("version_name", profile.minecraft_version.clone()),
        ("version_type", profile.version_type.clone()),
        ("game_directory", path_string(&options.game_dir)),
        ("game_assets", path_string(&assets.game_assets_dir(paths, &options.game_dir))),
        ("assets_root", path_string(&paths.assets_dir())),
        ("assets_index_name", profile.asset_index.id.clone()),
        ("library_directory", path_string(&paths.libraries_dir())),
    ]);

    let mut jvm = vec!["-Duser.language=en".to_owned()];
    jvm.extend(options.java_agent_args.iter().cloned());
    jvm.extend(options.extra_jvm_args.iter().cloned());
    jvm.extend(profile.jvm_args.iter().map(|arg| substitute(arg, &tokens)));
    if profile.runtime.system == "windows" {
        // Makes Intel drivers pick the discrete GPU profile for Minecraft.
        jvm.push("-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump".to_owned());
    }
    if profile.runtime.system == "osx" && profile.has_trait("FirstThreadOnMacOS") {
        jvm.push("-XstartOnFirstThread".to_owned());
    }
    let (min, max) = if options.min_memory_mb >= options.max_memory_mb {
        (options.max_memory_mb, options.min_memory_mb)
    } else {
        (options.min_memory_mb, options.max_memory_mb)
    };
    jvm.push(format!("-Xms{min}m"));
    jvm.push(format!("-Xmx{max}m"));
    jvm.push(format!("-Djava.library.path={}", path_string(&options.natives_dir)));

    let libraries_dir = paths.libraries_dir();
    let classpath: Vec<String> =
        profile.classpath_files().iter().map(|file| path_string(&libraries_dir.join(&file.path))).collect();
    jvm.push("-cp".to_owned());
    jvm.push(classpath.join(&CLASSPATH_SEPARATOR.to_string()));
    jvm.push(profile.main_class.clone());

    let mut game: Vec<String> = profile.minecraft_arguments.split_whitespace().map(str::to_owned).collect();
    for tweaker in &profile.tweakers {
        game.push("--tweakClass".to_owned());
        game.push(tweaker.clone());
    }
    let mut game: Vec<String> = game.iter().map(|arg| substitute(arg, &tokens)).collect();
    if let Some(window) = options.window {
        game.extend(["--width".to_owned(), window.width.to_string(), "--height".to_owned(), window.height.to_string()]);
    }

    LaunchArgs { jvm, game }
}

/// Quotes an argument for a Java `@argfile`: double quotes with `\` and `"` escaped.
fn argfile_line(arg: &str) -> String {
    format!("\"{}\"", arg.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Builds the command that starts the game. For Java 9+ the JVM arguments and classpath are
/// written to `launch-args.txt` next to the natives directory and passed as `@file`, which keeps
/// the command line far below the Windows limit. Java 8 gets everything directly.
pub async fn build_command(
    paths: &Paths,
    profile: &LaunchProfile,
    assets: &AssetIndex,
    options: &LaunchOptions,
) -> Result<LaunchCommand> {
    if options.java_path.as_os_str().is_empty() {
        return Err(Error::Other("no Java executable is configured".to_owned()));
    }
    let LaunchArgs { jvm, game } = launch_args(paths, profile, assets, options);

    let mut java_args = if options.java_major >= ARGFILE_MIN_JAVA_MAJOR {
        let argfile = options.natives_dir.with_file_name("launch-args.txt");
        let contents = jvm.iter().map(|arg| argfile_line(arg)).collect::<Vec<_>>().join("\n");
        tokio::fs::write(&argfile, contents).await.at(&argfile)?;
        vec![format!("@{}", argfile.display())]
    } else {
        jvm
    };
    java_args.extend(game);

    let (program, mut args) = match options.wrapper.split_first() {
        Some((wrapper, wrapper_args)) => {
            let mut args = wrapper_args.to_vec();
            args.push(path_string(&options.java_path));
            (PathBuf::from(wrapper), args)
        }
        None => (options.java_path.clone(), Vec::new()),
    };
    args.extend(java_args);

    Ok(LaunchCommand { program, args, cwd: options.game_dir.clone(), env: options.env.clone() })
}

#[cfg(test)]
mod tests;
