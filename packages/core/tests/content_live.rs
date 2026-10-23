//! Live tests against api.modrinth.com and api.curseforge.com. Run with
//! `cargo test -p k0t1k-core --test content_live -- --ignored --nocapture`.
//! CurseForge tests need `K0T1K_CURSEFORGE_API_KEY` and are skipped without it.

use std::io::Write;
use std::path::{Path, PathBuf};

use k0t1k_core::content::{
    Clients, InstallContext, Loader, ResourceKind, SearchQuery, SearchSort, check_updates,
    inspect_modpack, install_modpack, install_version, murmur2, packwiz, scan_resources,
};
use k0t1k_core::download::{download_checked, http_client};
use k0t1k_core::progress::no_progress;

const FABRIC_API: &str = "P7dR8mSH";
const MOD_MENU: &str = "mOgUt4GM";
const SODIUM: &str = "AANobbMI";

fn clients() -> Clients {
    let key = std::env::var("K0T1K_CURSEFORGE_API_KEY").ok();
    Clients::new(http_client(), key.as_deref()).unwrap()
}

fn curseforge_clients() -> Option<Clients> {
    let clients = clients();
    if clients.curseforge.is_none() {
        eprintln!("K0T1K_CURSEFORGE_API_KEY is not set, skipping");
        return None;
    }
    Some(clients)
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("k0t1k-live-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn fabric_context(game_dir: &Path) -> InstallContext {
    InstallContext {
        game_dir: game_dir.to_owned(),
        game_version: "1.21.4".into(),
        loaders: vec![Loader::Fabric],
    }
}

fn mod_query(query: &str) -> SearchQuery {
    SearchQuery {
        query: query.into(),
        kind: ResourceKind::Mod,
        game_version: Some("1.21.4".into()),
        loaders: vec![Loader::Fabric],
        categories: Vec::new(),
        sort: SearchSort::Relevance,
        offset: 0,
        limit: 10,
    }
}

#[tokio::test]
#[ignore = "network"]
async fn modrinth_search_finds_sodium() {
    let page = clients()
        .modrinth
        .search(&mod_query("sodium"))
        .await
        .unwrap();
    println!(
        "{} hits, first: {:?}",
        page.total_hits,
        page.hits.first().map(|hit| &hit.slug)
    );
    assert!(
        page.hits
            .iter()
            .any(|hit| hit.slug == "sodium" && hit.project_id == SODIUM)
    );
}

#[tokio::test]
#[ignore = "network"]
async fn modrinth_install_hash_lookup_and_update_check() {
    let clients = clients();
    let game_dir = temp_dir("install");
    let ctx = fabric_context(&game_dir);
    let progress = no_progress();

    // Mod Menu requires Fabric API and Text Placeholder API, so the dependency must be installed too.
    let menu = clients
        .modrinth
        .versions(MOD_MENU, &["1.21.4".into()], &[Loader::Fabric])
        .await
        .unwrap();
    let menu = menu
        .into_iter()
        .max_by(|a, b| a.published.cmp(&b.published))
        .unwrap();
    let report = install_version(&clients, &ctx, ResourceKind::Mod, &menu, &progress)
        .await
        .unwrap();
    println!(
        "installed: {:?}",
        report
            .installed
            .iter()
            .map(|i| &i.file_name)
            .collect::<Vec<_>>()
    );
    assert!(
        report
            .installed
            .iter()
            .any(|item| item.project_id == FABRIC_API && item.is_dependency)
    );
    assert!(report.blocked.is_empty() && report.unresolved_dependencies.is_empty());

    // Install an older Sodium, then look it up by hash and find the newer one as an update.
    let sodium = clients
        .modrinth
        .versions(SODIUM, &["1.21.4".into()], &[Loader::Fabric])
        .await
        .unwrap();
    let mut sodium: Vec<_> = sodium
        .into_iter()
        .filter(|version| version.release_type == k0t1k_core::content::ReleaseType::Release)
        .collect();
    sodium.sort_by(|a, b| b.published.cmp(&a.published));
    assert!(sodium.len() >= 2);
    install_version(&clients, &ctx, ResourceKind::Mod, &sodium[1], &progress)
        .await
        .unwrap();

    let resources = scan_resources(&game_dir, ResourceKind::Mod).unwrap();
    println!(
        "scanned: {:?}",
        resources
            .iter()
            .map(|r| (&r.file_name, r.metadata.as_ref().map(|m| &m.id)))
            .collect::<Vec<_>>()
    );
    let local = resources
        .iter()
        .find(|r| r.metadata.as_ref().is_some_and(|m| m.id == "sodium"))
        .unwrap();
    assert!(local.origin.is_some());

    let hash = local.origin.as_ref().unwrap().download.hash.clone();
    let found = clients
        .modrinth
        .versions_by_hashes(
            std::slice::from_ref(&hash),
            k0t1k_core::content::modrinth::HashAlgorithm::Sha512,
        )
        .await
        .unwrap();
    assert_eq!(found[&hash].id, sodium[1].id);

    let updates = check_updates(&clients, &ctx, ResourceKind::Mod)
        .await
        .unwrap();
    println!(
        "updates: {:?}",
        updates
            .iter()
            .map(|u| (&u.name, &u.current_version, &u.latest.version_number))
            .collect::<Vec<_>>()
    );
    let update = updates
        .iter()
        .find(|update| update.latest.project_id == SODIUM)
        .unwrap();
    assert_eq!(update.latest.id, sodium[0].id);

    // Applying the update replaces the old file.
    install_version(&clients, &ctx, ResourceKind::Mod, &update.latest, &progress)
        .await
        .unwrap();
    let sodium_files: Vec<_> = scan_resources(&game_dir, ResourceKind::Mod)
        .unwrap()
        .into_iter()
        .filter(|r| r.metadata.as_ref().is_some_and(|m| m.id == "sodium"))
        .collect();
    assert_eq!(sodium_files.len(), 1);
    assert!(
        check_updates(&clients, &ctx, ResourceKind::Mod)
            .await
            .unwrap()
            .iter()
            .all(|u| u.latest.project_id != SODIUM)
    );
    assert_eq!(
        packwiz::list(&game_dir.join("mods/.index")).unwrap().len(),
        4
    );
    std::fs::remove_dir_all(&game_dir).unwrap();
}

#[tokio::test]
#[ignore = "network"]
async fn modrinth_mrpack_import() {
    let clients = clients();
    let mut query = mod_query("");
    query.kind = ResourceKind::Modpack;
    query.limit = 30;
    query.sort = SearchSort::Downloads;
    let page = clients.modrinth.search(&query).await.unwrap();

    // Pick the smallest .mrpack among the popular packs to keep the test light.
    let mut best = None;
    for hit in page.hits {
        let versions = clients
            .modrinth
            .versions(&hit.project_id, &["1.21.4".into()], &[Loader::Fabric])
            .await
            .unwrap();
        let Some(version) = versions
            .into_iter()
            .max_by(|a, b| a.published.cmp(&b.published))
        else {
            continue;
        };
        let Some(file) = version.install_file().cloned() else {
            continue;
        };
        if file.filename.ends_with(".mrpack")
            && best.as_ref().is_none_or(|(_, size, _)| file.size < *size)
        {
            best = Some((file.url.clone().unwrap(), file.size, hit.title));
        }
    }
    let (url, size, title) = best.expect("a modpack for 1.21.4 fabric");
    println!("importing {title} ({size} bytes)");

    let dir = temp_dir("mrpack");
    let zip_path = dir.join("pack.mrpack");
    download_checked(&clients.http, &url, &zip_path, None)
        .await
        .unwrap();
    let manifest = inspect_modpack(&zip_path).unwrap();
    println!("components: {:?}", manifest.components);
    let game_dir = dir.join("game");
    let report = install_modpack(&manifest, &zip_path, &game_dir, &clients, &no_progress())
        .await
        .unwrap();
    println!("optional files: {}", report.optional_files.len());

    let mods = scan_resources(&game_dir, ResourceKind::Mod).unwrap();
    let with_origin = mods.iter().filter(|m| m.origin.is_some()).count();
    println!("{} mods, {with_origin} with pw.toml", mods.len());
    assert!(!mods.is_empty() && with_origin > 0);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
#[ignore = "network and API key"]
async fn curseforge_search_files_and_fingerprint() {
    let Some(clients) = curseforge_clients() else {
        return;
    };
    let curseforge = clients.curseforge.as_ref().unwrap();

    let page = curseforge.search(&mod_query("jei")).await.unwrap();
    println!(
        "{} hits, first: {:?}",
        page.total_hits,
        page.hits.first().map(|hit| &hit.slug)
    );
    assert!(!page.hits.is_empty());

    let mut versions = curseforge
        .versions("238222", Some("1.21.1"), &[Loader::NeoForge])
        .await
        .unwrap();
    versions.retain(|version| {
        version
            .install_file()
            .is_some_and(|file| file.url.is_some())
    });
    let version = versions
        .into_iter()
        .max_by(|a, b| a.published.cmp(&b.published))
        .expect("a JEI file");
    let file = version.install_file().unwrap();
    println!("file {} ({})", file.filename, version.id);

    let dir = temp_dir("fingerprint");
    let jar = dir.join(&file.filename);
    download_checked(&clients.http, file.url.as_ref().unwrap(), &jar, None)
        .await
        .unwrap();
    let fingerprint = murmur2::fingerprint_file(&jar).unwrap();
    let matches = curseforge
        .versions_by_fingerprints(&[fingerprint])
        .await
        .unwrap();
    println!(
        "fingerprint {fingerprint} -> {:?}",
        matches.get(&fingerprint).map(|v| &v.id)
    );
    assert_eq!(matches[&fingerprint].id, version.id);
    std::fs::remove_dir_all(&dir).unwrap();

    let categories = curseforge.categories(ResourceKind::Mod).await.unwrap();
    assert!(!categories.is_empty());
}

#[tokio::test]
#[ignore = "network and API key"]
async fn curseforge_modpack_resolution_with_blocked_file() {
    let Some(clients) = curseforge_clients() else {
        return;
    };
    let curseforge = clients.curseforge.as_ref().unwrap();

    let jei = curseforge
        .versions("238222", Some("1.21.1"), &[Loader::NeoForge])
        .await
        .unwrap();
    let jei = jei
        .into_iter()
        .filter(|v| v.install_file().is_some_and(|f| f.url.is_some()))
        .max_by(|a, b| a.published.cmp(&b.published))
        .unwrap();

    // Projects whose files have downloads disabled on CurseForge. Those also on Modrinth are fetched
    // from there by SHA-1; the rest must come back as blocked.
    let mut file_ids = vec![jei.id.clone()];
    for project_id in ["448233", "433760", "521480"] {
        let versions = curseforge.versions(project_id, None, &[]).await.unwrap();
        let blocked = versions
            .into_iter()
            .find(|v| v.install_file().is_some_and(|f| f.url.is_none()))
            .expect("a file with downloads disabled");
        file_ids.push(blocked.id);
    }
    let files: Vec<String> = [238222, 448233, 433760, 521480]
        .iter()
        .zip(&file_ids)
        .map(|(project, file)| {
            format!(r#"{{"projectID":{project},"fileID":{file},"required":true}}"#)
        })
        .collect();

    let dir = temp_dir("cfpack");
    let manifest_json = format!(
        r#"{{"minecraft":{{"version":"1.21.1","modLoaders":[{{"id":"neoforge-21.1.172","primary":true}}]}},
        "manifestType":"minecraftModpack","manifestVersion":1,"name":"Live","version":"1","author":"t","overrides":"overrides",
        "files":[{}]}}"#,
        files.join(",")
    );
    let zip_path = dir.join("pack.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
    writer
        .start_file("manifest.json", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(manifest_json.as_bytes()).unwrap();
    writer
        .start_file(
            "overrides/config/x.txt",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"x").unwrap();
    writer.finish().unwrap();

    let manifest = inspect_modpack(&zip_path).unwrap();
    let game_dir = dir.join("game");
    let report = install_modpack(&manifest, &zip_path, &game_dir, &clients, &no_progress())
        .await
        .unwrap();
    let mods = scan_resources(&game_dir, ResourceKind::Mod).unwrap();
    println!(
        "blocked: {:?}",
        report
            .blocked
            .iter()
            .map(|b| (&b.file_name, &b.download_page))
            .collect::<Vec<_>>()
    );
    println!(
        "installed: {:?}",
        mods.iter().map(|m| &m.file_name).collect::<Vec<_>>()
    );
    assert_eq!(report.blocked.len() + mods.len(), 4);
    assert!(
        report
            .blocked
            .iter()
            .all(|file| file.download_page.is_some())
    );
    assert!(
        mods.iter()
            .all(|m| m.origin.as_ref().is_some_and(|o| o.lock_update))
    );
    assert!(game_dir.join("config/x.txt").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
