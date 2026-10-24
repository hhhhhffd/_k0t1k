#[cfg(unix)]
use std::collections::BTreeSet;
#[cfg(unix)]
use std::sync::Arc;

use crate::component::ComponentSpec;
use crate::meta::VersionFile;
use crate::profile::ResolvedComponent;
use crate::profile::library::RuntimeContext;

use super::*;

fn profile(system: &str, extra: &str) -> LaunchProfile {
    let json = format!(
        r#"{{"uid":"net.minecraft","version":"1.21.4","type":"release","mainClass":"net.minecraft.client.main.Main",
            "minecraftArguments":"--username ${{auth_player_name}} --version ${{version_name}} --gameDir ${{game_directory}} --assetsDir ${{assets_root}} --assetIndex ${{assets_index_name}} --uuid ${{auth_uuid}} --accessToken ${{auth_access_token}} --clientId ${{clientid}} --versionType ${{version_type}}",
            "+jvmArgs":["-Dprofile=${{profile_name}}"],
            "assetIndex":{{"id":"19","url":"https://x/19.json"}},
            "libraries":[{{"name":"g:a:1","downloads":{{"artifact":{{"url":"u"}}}}}},{{"name":"g:b:2","downloads":{{"artifact":{{"url":"u"}}}}}}],
            "mainJar":{{"name":"com.mojang:minecraft:1.21.4:client","downloads":{{"artifact":{{"url":"u"}}}}}}{extra}}}"#
    );
    let file: VersionFile = serde_json::from_str(&json).unwrap();
    let component = ResolvedComponent {
        spec: ComponentSpec { uid: "net.minecraft".to_owned(), version: "1.21.4".to_owned(), dependency_only: false },
        file,
    };
    LaunchProfile::build(&[component], RuntimeContext::new(system, "amd64")).unwrap()
}

fn assets() -> AssetIndex {
    AssetIndex { id: "19".to_owned(), objects: BTreeMap::new(), is_virtual: false, map_to_resources: false }
}

fn options(root: &std::path::Path) -> LaunchOptions {
    LaunchOptions {
        java_path: PathBuf::from("/jre/bin/java"),
        java_major: 21,
        game_dir: root.join("game"),
        natives_dir: root.join("inst/natives"),
        profile_name: "My Instance".to_owned(),
        min_memory_mb: 512,
        max_memory_mb: 4096,
        java_agent_args: Vec::new(),
        extra_jvm_args: vec!["-XX:+UseG1GC".to_owned()],
        window: Some(WindowSize { width: 854, height: 480 }),
        account: Account::offline("Notch"),
        env: BTreeMap::new(),
        wrapper: Vec::new(),
        join_server: None,
    }
}

#[test]
fn unknown_tokens_become_empty_and_text_is_kept() {
    let tokens = HashMap::from([("a", "1".to_owned())]);
    assert_eq!(substitute("x${a}y${a}", &tokens), "x1y1");
    assert_eq!(substitute("--id=${missing}", &tokens), "--id=");
    assert_eq!(substitute("${missing}", &tokens), "");
    assert_eq!(substitute("no ${ close", &tokens), "no ${ close");
    assert_eq!(substitute("plain", &tokens), "plain");
}

#[test]
fn jvm_arguments_follow_the_documented_order() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path().join("data"));
    let args = launch_args(&paths, &profile("windows", ""), &assets(), &options(root.path()));
    let natives = format!("-Djava.library.path={}", root.path().join("inst/natives").display());
    let expected_prefix = [
        "-Duser.language=en",
        "-XX:+UseG1GC",
        "-Dprofile=My Instance",
        "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump",
        "-Xms512m",
        "-Xmx4096m",
        natives.as_str(),
        "-cp",
    ];
    assert_eq!(&args.jvm[..expected_prefix.len()], expected_prefix);

    let classpath = &args.jvm[expected_prefix.len()];
    let libraries = paths.libraries_dir();
    let entries: Vec<_> = classpath.split(CLASSPATH_SEPARATOR).collect();
    assert_eq!(entries.len(), 3);
    assert!(entries[0].ends_with("g/a/1/a-1.jar") && entries[0].starts_with(&*libraries.to_string_lossy()));
    assert!(entries[2].ends_with("com/mojang/minecraft/1.21.4/minecraft-1.21.4-client.jar"));
    assert_eq!(args.jvm.last().unwrap(), "net.minecraft.client.main.Main");
}

#[test]
fn heap_dump_hack_is_windows_only() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path());
    let args = launch_args(&paths, &profile("linux", ""), &assets(), &options(root.path()));
    assert!(!args.jvm.iter().any(|arg| arg.starts_with("-XX:HeapDumpPath")));
}

#[test]
fn first_thread_flag_needs_macos_and_trait() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path());
    let extra = r#","+traits":["FirstThreadOnMacOS"]"#;
    let mac = launch_args(&paths, &profile("osx", extra), &assets(), &options(root.path()));
    assert!(mac.jvm.contains(&"-XstartOnFirstThread".to_owned()));
    let linux = launch_args(&paths, &profile("linux", extra), &assets(), &options(root.path()));
    assert!(!linux.jvm.contains(&"-XstartOnFirstThread".to_owned()));
}

#[test]
fn memory_is_swapped_when_min_is_not_below_max() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path());
    let mut options = options(root.path());
    options.min_memory_mb = 4096;
    options.max_memory_mb = 1024;
    let args = launch_args(&paths, &profile("linux", ""), &assets(), &options);
    assert!(args.jvm.contains(&"-Xms1024m".to_owned()));
    assert!(args.jvm.contains(&"-Xmx4096m".to_owned()));
}

#[test]
fn game_arguments_are_substituted_with_tweakers_and_window_size() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path().join("data"));
    let extra = r#","+tweakers":["net.minecraftforge.fml.common.launcher.FMLTweaker"]"#;
    let args = launch_args(&paths, &profile("linux", extra), &assets(), &options(root.path()));

    let game_dir = root.path().join("game").display().to_string();
    let assets_root = paths.assets_dir().display().to_string();
    let uuid = offline_uuid("Notch");
    let expected: Vec<String> = [
        "--username",
        "Notch",
        "--version",
        "1.21.4",
        "--gameDir",
        &game_dir,
        "--assetsDir",
        &assets_root,
        "--assetIndex",
        "19",
        "--uuid",
        &uuid,
        "--accessToken",
        "0",
        "--clientId",
        "",
        "--versionType",
        "release",
        "--tweakClass",
        "net.minecraftforge.fml.common.launcher.FMLTweaker",
        "--width",
        "854",
        "--height",
        "480",
    ]
    .map(str::to_owned)
    .to_vec();
    assert_eq!(args.game, expected);
}

#[test]
fn window_size_is_optional() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path());
    let mut options = options(root.path());
    options.window = None;
    let args = launch_args(&paths, &profile("linux", ""), &assets(), &options);
    assert!(!args.game.contains(&"--width".to_owned()));
}

#[test]
fn argfile_lines_escape_backslashes_and_quotes() {
    assert_eq!(argfile_line(r#"C:\Users\a b\x"#), r#""C:\\Users\\a b\\x""#);
    assert_eq!(argfile_line(r#"say "hi""#), r#""say \"hi\"""#);
}

#[tokio::test]
async fn java_9_plus_uses_an_argfile_next_to_the_natives_dir() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("inst")).unwrap();
    let paths = Paths::new(root.path().join("data"));
    let options = options(root.path());
    let command = build_command(&paths, &profile("linux", ""), &assets(), &options).await.unwrap();

    let argfile = root.path().join("inst").join("launch-args.txt");
    assert_eq!(command.program, PathBuf::from("/jre/bin/java"));
    assert_eq!(command.args[0], format!("@{}", argfile.display()));
    assert_eq!(command.args[1], "--username");
    assert_eq!(command.cwd, root.path().join("game"));

    let contents = std::fs::read_to_string(argfile).unwrap();
    let lines: Vec<_> = contents.lines().collect();
    assert_eq!(lines[0], "\"-Duser.language=en\"");
    assert_eq!(*lines.last().unwrap(), "\"net.minecraft.client.main.Main\"");
}

#[tokio::test]
async fn java_8_gets_every_argument_directly_and_wrapper_goes_first() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path());
    let mut options = options(root.path());
    options.java_major = 8;
    options.wrapper = vec!["gamemoderun".to_owned(), "--flag".to_owned()];
    let command = build_command(&paths, &profile("linux", ""), &assets(), &options).await.unwrap();

    assert_eq!(command.program, PathBuf::from("gamemoderun"));
    assert_eq!(&command.args[..3], ["--flag", "/jre/bin/java", "-Duser.language=en"]);
    assert!(command.args.contains(&"net.minecraft.client.main.Main".to_owned()));
    assert!(!root.path().join("launch-args.txt").exists());
}

#[tokio::test]
async fn empty_java_path_is_an_error() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path());
    let mut options = options(root.path());
    options.java_path = PathBuf::new();
    assert!(build_command(&paths, &profile("linux", ""), &assets(), &options).await.is_err());
}

#[test]
fn redacted_display_masks_the_access_token() {
    let command = LaunchCommand {
        program: PathBuf::from("java"),
        args: ["--username", "Notch", "--accessToken", "secret", "--uuid", "u"].map(str::to_owned).to_vec(),
        cwd: PathBuf::new(),
        env: BTreeMap::new(),
    };
    let shown = command.display_redacted();
    assert_eq!(shown, "java --username Notch --accessToken *** --uuid u");
}

#[cfg(unix)]
#[tokio::test]
async fn spawn_streams_both_outputs_and_reports_the_exit_code() {
    use std::sync::Mutex;

    let command = LaunchCommand {
        program: PathBuf::from("sh"),
        args: ["-c", "echo out; echo err >&2; exit 3"].map(str::to_owned).to_vec(),
        cwd: std::env::temp_dir(),
        env: BTreeMap::new(),
    };
    let lines = Arc::new(Mutex::new(BTreeSet::new()));
    let sink = lines.clone();
    let on_log: LogFn = Arc::new(move |line| {
        sink.lock().unwrap().insert((line.stream == LogStream::Stderr, line.line));
    });
    let handle = spawn_game(&command, on_log).unwrap();
    assert_eq!(handle.wait().await.unwrap(), Some(3));
    let lines = lines.lock().unwrap();
    assert!(lines.contains(&(false, "out".to_owned())));
    assert!(lines.contains(&(true, "err".to_owned())));
}

#[cfg(unix)]
#[tokio::test]
async fn kill_stops_a_running_process() {
    let command = LaunchCommand {
        program: PathBuf::from("sleep"),
        args: vec!["60".to_owned()],
        cwd: std::env::temp_dir(),
        env: BTreeMap::new(),
    };
    let handle = spawn_game(&command, Arc::new(|_| {})).unwrap();
    handle.kill();
    assert_eq!(handle.wait().await.unwrap(), None);
}

#[cfg(unix)]
#[tokio::test]
async fn spawn_drops_java_env_vars_and_applies_custom_ones() {
    use std::sync::Mutex;

    // SAFETY: no other test in this binary reads JAVA_TOOL_OPTIONS.
    unsafe { std::env::set_var("JAVA_TOOL_OPTIONS", "-Dleak=1") };
    let command = LaunchCommand {
        program: PathBuf::from("sh"),
        args: ["-c", "echo \"[$JAVA_TOOL_OPTIONS][$K0T1K_TEST]\""].map(str::to_owned).to_vec(),
        cwd: std::env::temp_dir(),
        env: BTreeMap::from([("K0T1K_TEST".to_owned(), "yes".to_owned())]),
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let on_log: LogFn = Arc::new(move |line| sink.lock().unwrap().push(line.line));
    spawn_game(&command, on_log).unwrap().wait().await.unwrap();
    assert_eq!(*seen.lock().unwrap(), ["[][yes]"]);
}

fn game_args_with_server(extra: &str, server: Option<&str>) -> Vec<String> {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path().join("data"));
    let mut options = options(root.path());
    options.join_server = server.map(str::to_owned);
    launch_args(&paths, &profile("linux", extra), &assets(), &options).game
}

#[test]
fn quick_play_trait_selects_quick_play_multiplayer() {
    let extra = r#","+traits":["feature:is_quick_play_multiplayer"]"#;
    let game = game_args_with_server(extra, Some("play.example.org:25570"));
    assert_eq!(&game[game.len() - 2..], ["--quickPlayMultiplayer", "play.example.org:25570"]);
    let game = game_args_with_server(extra, Some(" play.example.org "));
    assert_eq!(&game[game.len() - 2..], ["--quickPlayMultiplayer", "play.example.org:25565"]);
    let game = game_args_with_server(extra, Some("::1"));
    assert_eq!(&game[game.len() - 2..], ["--quickPlayMultiplayer", "[::1]:25565"]);
}

#[test]
fn old_versions_use_server_and_port() {
    let game = game_args_with_server("", Some("mc.example.org:1234"));
    assert_eq!(&game[game.len() - 4..], ["--server", "mc.example.org", "--port", "1234"]);
    let game = game_args_with_server("", Some("mc.example.org"));
    assert_eq!(&game[game.len() - 4..], ["--server", "mc.example.org", "--port", "25565"]);
}

#[test]
fn blank_or_missing_server_adds_nothing() {
    for server in [None, Some(""), Some("  ")] {
        let game = game_args_with_server("", server);
        assert!(!game.iter().any(|arg| arg == "--server" || arg == "--quickPlayMultiplayer"));
    }
}

#[test]
fn server_address_forms() {
    assert_eq!(split_server_address("a.b:99"), Some(("a.b".to_owned(), 99)));
    assert_eq!(split_server_address("[::1]:99"), Some(("::1".to_owned(), 99)));
    assert_eq!(split_server_address("a.b:notaport"), Some(("a.b:notaport".to_owned(), 25565)));
    assert_eq!(split_server_address(""), None);
}
