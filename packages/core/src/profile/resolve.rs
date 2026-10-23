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

async fn load(meta: &MetaClient, spec: ComponentSpec) -> Result<ResolvedComponent> {
    let file = meta.version_file(&spec.uid, &spec.version).await?;
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
