//! Recognises the links a user may paste to install a modpack.

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackUrl {
    /// `curseforge://install?addonId=..&fileId=..`, the link CurseForge's own site opens.
    CurseForgeInstall { project_id: String, file_id: String },
    /// `modrinth.com/modpack/<slug or id>[/version/<id or version number>]`.
    Modrinth { project: String, version: Option<String> },
    /// `curseforge.com/minecraft/modpacks/<slug>[/files/<id>]`.
    CurseForgePage { slug: String, file_id: Option<String> },
    /// Any other http(s) link: a pack file to download.
    File(String),
}

pub fn parse_pack_url(input: &str) -> Result<PackUrl> {
    let input = input.trim();
    let url = reqwest::Url::parse(input).map_err(|error| Error::Other(format!("{input:?} is not a valid link: {error}")))?;
    if url.scheme() == "curseforge" {
        let param = |name: &str| url.query_pairs().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.into_owned());
        return match (param("addonId"), param("fileId")) {
            (Some(project_id), Some(file_id)) => Ok(PackUrl::CurseForgeInstall { project_id, file_id }),
            _ => Err(Error::Other("the curseforge:// link has no addonId and fileId".into())),
        };
    }
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::Other(format!("unsupported link scheme {:?}; use an http(s) or curseforge:// link", url.scheme())));
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let segments: Vec<&str> = url.path_segments().map_or_else(Vec::new, |segments| segments.filter(|segment| !segment.is_empty()).collect());
    let host = host.strip_prefix("www.").unwrap_or(&host);
    match (host, segments.as_slice()) {
        ("modrinth.com", ["modpack", project]) => Ok(PackUrl::Modrinth { project: (*project).to_owned(), version: None }),
        ("modrinth.com", ["modpack", project, "version", version, ..]) => {
            Ok(PackUrl::Modrinth { project: (*project).to_owned(), version: Some((*version).to_owned()) })
        }
        ("modrinth.com", ["modpack", project, ..]) => Ok(PackUrl::Modrinth { project: (*project).to_owned(), version: None }),
        ("curseforge.com", ["minecraft", "modpacks", slug]) => Ok(PackUrl::CurseForgePage { slug: (*slug).to_owned(), file_id: None }),
        ("curseforge.com", ["minecraft", "modpacks", slug, "files" | "download", file_id, ..]) => {
            Ok(PackUrl::CurseForgePage { slug: (*slug).to_owned(), file_id: Some((*file_id).to_owned()) })
        }
        ("curseforge.com", ["minecraft", "modpacks", slug, ..]) => Ok(PackUrl::CurseForgePage { slug: (*slug).to_owned(), file_id: None }),
        _ => Ok(PackUrl::File(url.into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modrinth(project: &str, version: Option<&str>) -> PackUrl {
        PackUrl::Modrinth { project: project.into(), version: version.map(Into::into) }
    }

    #[test]
    fn recognises_platform_links() {
        assert_eq!(
            parse_pack_url("curseforge://install?addonId=123&fileId=456").unwrap(),
            PackUrl::CurseForgeInstall { project_id: "123".into(), file_id: "456".into() }
        );
        assert_eq!(parse_pack_url("https://modrinth.com/modpack/fabulously-optimized").unwrap(), modrinth("fabulously-optimized", None));
        assert_eq!(parse_pack_url("https://modrinth.com/modpack/fabulously-optimized/versions").unwrap(), modrinth("fabulously-optimized", None));
        assert_eq!(
            parse_pack_url(" https://modrinth.com/modpack/abc/version/1.2.3?x=1 ").unwrap(),
            modrinth("abc", Some("1.2.3"))
        );
        assert_eq!(
            parse_pack_url("https://www.curseforge.com/minecraft/modpacks/all-the-mods-10").unwrap(),
            PackUrl::CurseForgePage { slug: "all-the-mods-10".into(), file_id: None }
        );
        assert_eq!(
            parse_pack_url("https://www.curseforge.com/minecraft/modpacks/atm/files/789").unwrap(),
            PackUrl::CurseForgePage { slug: "atm".into(), file_id: Some("789".into()) }
        );
    }

    #[test]
    fn other_http_links_are_files_and_other_schemes_fail() {
        assert_eq!(parse_pack_url("https://example.com/p.zip").unwrap(), PackUrl::File("https://example.com/p.zip".into()));
        assert!(parse_pack_url("curseforge://install?addonId=1").is_err());
        assert!(parse_pack_url("ftp://example.com/p.zip").is_err());
        assert!(parse_pack_url("not a url").is_err());
    }
}
