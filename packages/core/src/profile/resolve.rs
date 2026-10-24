use std::cmp::Ordering;

use crate::component::{ComponentSpec, MINECRAFT_UID};
use crate::error::{Error, Result};
use crate::meta::{MetaClient, VersionFile};

use super::version::compare_versions;

const INTERMEDIARY_UIDS: [&str; 2] = ["net.fabricmc.intermediary", "org.quiltmc.hashed"];

#[derive(Debug, Clone)]
pub struct ResolvedComponent {
    pub spec: ComponentSpec,
    pub file: VersionFile,
}

/// A requirement merged over every component that asks for the same uid.
struct Needed {
    uid: String,
    equals: Option<String>,
    suggests: Option<String>,
    /// First component that requires it; new components are inserted before it.
    dependee: String,
}

/// Main class of versions Prism starts the regular way; the 1.6 pre-releases carry `legacyLaunch` too.
const MODERN_MAIN_CLASS: &str = "net.minecraft.client.main.Main";

async fn load(meta: &MetaClient, spec: ComponentSpec) -> Result<ResolvedComponent> {
    let mut file = meta.version_file(&spec.uid, &spec.version).await?;
    // Prism starts pre-1.6 versions through a launcher jar it ships itself. Mojang's own JSON
    // starts them through `launchwrapper` and needs nothing else.
    if spec.uid == MINECRAFT_UID
        && file.traits.iter().any(|name| name == "legacyLaunch")
        && file.main_class.as_deref() != Some(MODERN_MAIN_CLASS)
    {
        let launch = meta.mojang_launch(&spec.version).await?;
        file.main_class = Some(launch.main_class);
        file.minecraft_arguments = Some(launch.minecraft_arguments);
        file.libraries.extend(launch.libraries);
    }
    Ok(ResolvedComponent { spec, file })
}

fn gather(components: &[ResolvedComponent]) -> Result<Vec<Needed>> {
    let mut needed: Vec<Needed> = Vec::new();
    for component in components {
        for requirement in &component.file.requires {
            let Some(entry) = needed.iter_mut().find(|entry| entry.uid == requirement.uid) else {
                needed.push(Needed {
                    uid: requirement.uid.clone(),
                    equals: requirement.equals.clone(),
                    suggests: requirement.suggests.clone(),
                    dependee: component.spec.uid.clone(),
                });
                continue;
            };
            match (&entry.equals, &requirement.equals) {
                (Some(a), Some(b)) if a != b => {
                    return Err(Error::Other(format!("conflicting requirements for {}: {a} and {b}", requirement.uid)));
                }
                (None, Some(b)) => entry.equals = Some(b.clone()),
                _ => {}
            }
            if let Some(suggested) = &requirement.suggests
                && entry
                    .suggests
                    .as_deref()
                    .is_none_or(|current| compare_versions(suggested, current) == Ordering::Greater)
            {
                entry.suggests = Some(suggested.clone());
            }
        }
    }
    Ok(needed)
}

/// Version for a component that has to be added without the user choosing one.
fn default_version(components: &[ResolvedComponent], needed: &Needed) -> Result<String> {
    if let Some(version) = needed.equals.as_ref().or(needed.suggests.as_ref()) {
        return Ok(version.clone());
    }
    match needed.uid.as_str() {
        "org.lwjgl" => Ok("2.9.1".to_owned()),
        "org.lwjgl3" => Ok("3.1.2".to_owned()),
        uid if INTERMEDIARY_UIDS.contains(&uid) => components
            .iter()
            .find(|component| component.spec.uid == MINECRAFT_UID)
            .map(|component| component.spec.version.clone())
            .ok_or_else(|| Error::Other(format!("{uid} requires a Minecraft component"))),
        uid => Err(Error::Other(format!("no version of {uid} can be chosen automatically"))),
    }
}

/// Completes a component list with the dependencies the components require (LWJGL, Fabric's
/// intermediary), repeating until nothing changes. The result keeps the apply order; added
/// components are marked `dependency_only` and placed right before their first dependee.
pub async fn resolve_components(meta: &MetaClient, specs: Vec<ComponentSpec>) -> Result<Vec<ResolvedComponent>> {
    let mut components = Vec::with_capacity(specs.len());
    for spec in specs {
        components.push(load(meta, spec).await?);
    }

    loop {
        let needed = gather(&components)?;

        let before = components.len();
        components.retain(|component| {
            !(component.spec.dependency_only
                && component.file.volatile
                && !needed.iter().any(|entry| entry.uid == component.spec.uid))
        });
        if components.len() != before {
            continue;
        }

        let mut changed = false;
        for entry in &needed {
            let existing = components.iter().position(|component| component.spec.uid == entry.uid);
            match (existing, &entry.equals) {
                (None, _) => {
                    let version = default_version(&components, entry)?;
                    let component =
                        load(meta, ComponentSpec { uid: entry.uid.clone(), version, dependency_only: true }).await?;
                    let at = components
                        .iter()
                        .position(|component| component.spec.uid == entry.dependee)
                        .unwrap_or(components.len());
                    components.insert(at, component);
                    changed = true;
                }
                (Some(index), Some(equals)) if components[index].spec.version != *equals => {
                    if !components[index].spec.dependency_only {
                        return Err(Error::Other(format!(
                            "conflicting dependencies: {} {} is selected but {} is required",
                            entry.uid, components[index].spec.version, equals
                        )));
                    }
                    let spec = ComponentSpec { version: equals.clone(), ..components[index].spec.clone() };
                    components[index] = load(meta, spec).await?;
                    changed = true;
                }
                _ => {}
            }
        }
        if !changed {
            return Ok(components);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::paths::Paths;

    use super::*;

    fn write(paths: &Paths, relative: &str, json: &str) {
        let path = paths.meta_dir().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json).unwrap();
    }

    async fn minecraft(paths: &Paths, version: &str) -> VersionFile {
        let spec = ComponentSpec { uid: MINECRAFT_UID.to_owned(), version: version.to_owned(), dependency_only: false };
        load(&MetaClient::new(paths, true), spec).await.unwrap().file
    }

    #[tokio::test]
    async fn legacy_launch_versions_take_the_launch_parts_from_mojang() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths::new(root.path());
        write(
            &paths,
            "net.minecraft/b1.7.3.json",
            r#"{"uid":"net.minecraft","version":"b1.7.3","+traits":["legacyLaunch"],"libraries":[{"name":"g:own:1"}],
                "minecraftArguments":"${auth_player_name} ${auth_session} --gameDir ${game_directory}"}"#,
        );
        write(
            &paths,
            "net.minecraft/1.6.4.json",
            r#"{"uid":"net.minecraft","version":"1.6.4","+traits":["legacyLaunch"],"mainClass":"net.minecraft.client.main.Main"}"#,
        );
        write(
            &paths,
            "mojang/b1.7.3.json",
            r#"{"mainClass":"net.minecraft.launchwrapper.Launch","minecraftArguments":"mojang args",
                "libraries":[{"name":"net.minecraft:launchwrapper:1.5"}]}"#,
        );

        let legacy = minecraft(&paths, "b1.7.3").await;
        assert_eq!(legacy.main_class.as_deref(), Some("net.minecraft.launchwrapper.Launch"));
        assert_eq!(legacy.minecraft_arguments.as_deref(), Some("mojang args"));
        let names: Vec<_> = legacy.libraries.iter().map(|library| library.name.to_string()).collect();
        assert_eq!(names, ["g:own:1", "net.minecraft:launchwrapper:1.5"]);

        // 1.6 pre-releases also carry the trait but start through the regular main class.
        let modern = minecraft(&paths, "1.6.4").await;
        assert_eq!(modern.main_class.as_deref(), Some("net.minecraft.client.main.Main"));
        assert!(modern.libraries.is_empty());
    }
}
