//! Real-network end-to-end check: `cargo test -p k0t1k-core --test e2e -- --ignored`.
//! Downloads about 500 MB the first time (set `K0T1K_E2E_DIR` to keep and reuse the data directory).

use std::collections::BTreeMap;
use std::path::PathBuf;

use k0t1k_core::component::ComponentSpec;
use k0t1k_core::install::{check_installed, extract_natives, install_game};
use k0t1k_core::launch::{Account, LaunchOptions, WindowSize, build_command};
use k0t1k_core::meta::{Loader, MetaClient};
use k0t1k_core::paths::Paths;
use k0t1k_core::profile::library::RuntimeContext;
use k0t1k_core::profile::{LaunchProfile, resolve_components};
use k0t1k_core::progress::no_progress;

fn spec(uid: &str, version: &str) -> ComponentSpec {
    ComponentSpec { uid: uid.to_owned(), version: version.to_owned(), dependency_only: false }
}

#[tokio::test]
#[ignore = "downloads Minecraft from the network"]
async fn install_vanilla_and_fabric_and_build_commands() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = std::env::var_os("K0T1K_E2E_DIR").map(PathBuf::from).unwrap_or_else(|| temp.path().to_owned());
    let paths = Paths::new(&data_dir);
    let meta = MetaClient::new(&paths, false);
    let client = k0t1k_core::download::http_client();

    let minecraft = meta.minecraft_versions().await.unwrap();
    let entry = minecraft.iter().find(|entry| entry.version == "1.21.4").expect("1.21.4 is listed");
    assert_eq!(entry.kind.as_deref(), Some("release"));
    let fabric = meta.loader_versions(Loader::Fabric, "1.21.4").await.unwrap();
    let fabric_version = &fabric.first().expect("Fabric versions exist").version;

    let instances = [
        ("vanilla", vec![spec("net.minecraft", "1.21.4")], "net.minecraft.client.main.Main"),
        (
            "fabric",
            vec![spec("net.minecraft", "1.21.4"), spec("net.fabricmc.fabric-loader", fabric_version)],
            "net.fabricmc.loader.impl.launch.knot.KnotClient",
        ),
    ];
    for (name, specs, main_class) in instances {
        let components = resolve_components(&meta, specs).await.unwrap();
        let profile = LaunchProfile::build(&components, RuntimeContext::host("amd64")).unwrap();
        assert_eq!(profile.main_class, main_class);

        let assets = install_game(&client, &paths, &profile, name, &no_progress()).await.unwrap();
        check_installed(&paths, &profile).await.unwrap();

        let instance_dir = paths.instances_dir().join(name);
        let natives_dir = instance_dir.join("natives");
        std::fs::create_dir_all(&instance_dir).unwrap();
        extract_natives(&paths, &profile, &natives_dir).await.unwrap();

        let options = LaunchOptions {
            java_path: PathBuf::from("java"),
            java_major: 21,
            game_dir: instance_dir.join("minecraft"),
            natives_dir,
            profile_name: name.to_owned(),
            min_memory_mb: 512,
            max_memory_mb: 2048,
            extra_jvm_args: Vec::new(),
            window: Some(WindowSize { width: 854, height: 480 }),
            account: Account::offline("Player"),
            env: BTreeMap::new(),
            wrapper: Vec::new(),
            join_server: None,
        };
        let command = build_command(&paths, &profile, &assets, &options).await.unwrap();
        assert!(command.args[0].starts_with('@'));
        let argfile = std::fs::read_to_string(instance_dir.join("launch-args.txt")).unwrap();
        assert!(argfile.contains(main_class));
        for file in profile.classpath_files() {
            assert!(paths.libraries_dir().join(&file.path).is_file(), "{} is missing", file.path);
        }
    }
}
