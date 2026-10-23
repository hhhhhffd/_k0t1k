//! CurseForge API v1 client. The API key travels only in the `x-api-key` header.

use std::collections::HashMap;

use reqwest::header::HeaderValue;
use serde::Deserialize;
use serde_json::json;

use super::{
    Category, Dependency, DependencyKind, Hashes, Loader, Platform, Project, ProjectVersion,
    ReleaseType, ResourceKind, SearchHit, SearchPage, SearchQuery, SearchSort, VersionFile,
    send_json,
};
use crate::error::{Error, Result};

const BASE_URL: &str = "https://api.curseforge.com/v1";
const MINECRAFT_GAME_ID: u32 = 432;
/// CurseForge caps `pageSize` of file listings; Prism asks for this much to get everything at once.
const ALL_FILES_PAGE_SIZE: u32 = 10_000;
const MAX_SEARCH_PAGE_SIZE: u32 = 50;

#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    /// Marked sensitive, so `Debug` and reqwest's logging never print it.
    api_key: HeaderValue,
}

impl Client {
    /// `http` should come from `download::http_client()` so requests carry our User-Agent.
    pub fn new(http: reqwest::Client, api_key: &str) -> Result<Self> {
        let mut api_key = HeaderValue::from_str(api_key).map_err(|_| {
            Error::Other("the CurseForge API key contains invalid characters".into())
        })?;
        api_key.set_sensitive(true);
        Ok(Self { http, api_key })
    }

    fn get(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .get(format!("{BASE_URL}{path}"))
            .header("x-api-key", self.api_key.clone())
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        self.http
            .post(format!("{BASE_URL}{path}"))
            .header("x-api-key", self.api_key.clone())
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<SearchPage> {
        let sort_field = match query.sort {
            SearchSort::Relevance => 1,
            SearchSort::Downloads => 6,
            SearchSort::Follows => 2,
            // CurseForge has no creation-date sort; "last updated" is the closest.
            SearchSort::Newest | SearchSort::Updated => 3,
        };
        let mut params = vec![
            ("gameId", MINECRAFT_GAME_ID.to_string()),
            ("classId", query.kind.curseforge_class_id().to_string()),
            ("searchFilter", query.query.clone()),
            ("sortField", sort_field.to_string()),
            ("sortOrder", "desc".to_owned()),
            ("index", query.offset.to_string()),
            (
                "pageSize",
                query.limit.min(MAX_SEARCH_PAGE_SIZE).to_string(),
            ),
        ];
        if let Some(version) = &query.game_version {
            params.push(("gameVersion", version.clone()));
        }
        if !query.loaders.is_empty() {
            let ids: Vec<_> = query
                .loaders
                .iter()
                .map(|loader| loader.curseforge_id())
                .collect();
            params.push(("modLoaderTypes", json!(ids).to_string()));
        }
        if !query.categories.is_empty() {
            let ids = query
                .categories
                .iter()
                .map(|id| parse_id(id))
                .collect::<Result<Vec<_>>>()?;
            params.push(("categoryIds", json!(ids).to_string()));
        }
        let page: Page<RawMod> = send_json(self.get("/mods/search").query(&params)).await?;
        Ok(SearchPage {
            hits: page
                .data
                .into_iter()
                .map(|raw| SearchHit {
                    platform: Platform::CurseForge,
                    project_id: raw.id.to_string(),
                    slug: raw.slug,
                    title: raw.name,
                    description: raw.summary,
                    author: raw
                        .authors
                        .first()
                        .map(|author| author.name.clone())
                        .unwrap_or_default(),
                    icon_url: raw.logo.map(|logo| logo.thumbnail_url),
                    downloads: raw.download_count as u64,
                    categories: raw
                        .categories
                        .into_iter()
                        .map(|category| category.name)
                        .collect(),
                })
                .collect(),
            offset: page.pagination.index,
            total_hits: page.pagination.total_count,
        })
    }

    pub async fn project(&self, id: &str) -> Result<Project> {
        let data: Data<RawMod> = send_json(self.get(&format!("/mods/{}", parse_id(id)?))).await?;
        Ok(data.data.into())
    }

    pub async fn projects(&self, ids: &[String]) -> Result<Vec<Project>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = ids
            .iter()
            .map(|id| parse_id(id))
            .collect::<Result<Vec<_>>>()?;
        let data: Data<Vec<RawMod>> =
            send_json(self.post("/mods").json(&json!({ "modIds": ids }))).await?;
        Ok(data.data.into_iter().map(Project::from).collect())
    }

    /// Files of a project for `game_version`. CurseForge filters one loader server-side; the full
    /// `loaders` list (any of them, all when empty) is applied to the result.
    pub async fn versions(
        &self,
        project_id: &str,
        game_version: Option<&str>,
        loaders: &[Loader],
    ) -> Result<Vec<ProjectVersion>> {
        let mut params = vec![("pageSize", ALL_FILES_PAGE_SIZE.to_string())];
        if let Some(version) = game_version {
            params.push(("gameVersion", version.to_owned()));
        }
        if let [loader] = loaders {
            params.push(("modLoaderType", loader.curseforge_id().to_string()));
        }
        let data: Data<Vec<RawFile>> = send_json(
            self.get(&format!("/mods/{}/files", parse_id(project_id)?))
                .query(&params),
        )
        .await?;
        let mut versions = data
            .data
            .into_iter()
            .map(ProjectVersion::try_from)
            .collect::<Result<Vec<_>>>()?;
        if !loaders.is_empty() {
            versions.retain(|version| {
                version
                    .loaders
                    .iter()
                    .any(|loader| loaders.contains(loader))
            });
        }
        self.attach_download_pages(&mut versions).await?;
        Ok(versions)
    }

    pub async fn version(&self, project_id: &str, file_id: &str) -> Result<ProjectVersion> {
        let path = format!(
            "/mods/{}/files/{}",
            parse_id(project_id)?,
            parse_id(file_id)?
        );
        let data: Data<RawFile> = send_json(self.get(&path)).await?;
        let mut versions = vec![ProjectVersion::try_from(data.data)?];
        self.attach_download_pages(&mut versions).await?;
        Ok(versions.remove(0))
    }

    /// Resolves many files at once; ids that do not exist are absent from the result.
    pub async fn versions_by_ids(&self, file_ids: &[String]) -> Result<Vec<ProjectVersion>> {
        if file_ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids = file_ids
            .iter()
            .map(|id| parse_id(id))
            .collect::<Result<Vec<_>>>()?;
        let data: Data<Vec<RawFile>> =
            send_json(self.post("/mods/files").json(&json!({ "fileIds": ids }))).await?;
        let mut versions = data
            .data
            .into_iter()
            .map(ProjectVersion::try_from)
            .collect::<Result<Vec<_>>>()?;
        self.attach_download_pages(&mut versions).await?;
        Ok(versions)
    }

    /// Looks up files by their murmur2 fingerprint. The map is keyed by the fingerprint.
    pub async fn versions_by_fingerprints(
        &self,
        fingerprints: &[u32],
    ) -> Result<HashMap<u32, ProjectVersion>> {
        if fingerprints.is_empty() {
            return Ok(HashMap::new());
        }
        let body =
            json!({ "fingerprints": fingerprints.iter().map(u32::to_string).collect::<Vec<_>>() });
        let data: Data<RawFingerprintMatches> =
            send_json(self.post("/fingerprints").json(&body)).await?;
        let mut keyed = Vec::new();
        for matched in data.data.exact_matches {
            keyed.push((
                matched.file.file_fingerprint,
                ProjectVersion::try_from(matched.file)?,
            ));
        }
        let (fingerprints, mut versions): (Vec<_>, Vec<_>) = keyed.into_iter().unzip();
        self.attach_download_pages(&mut versions).await?;
        Ok(fingerprints.into_iter().zip(versions).collect())
    }

    /// Categories usable as `SearchQuery::categories` for projects of `kind`.
    pub async fn categories(&self, kind: ResourceKind) -> Result<Vec<Category>> {
        let params = [
            ("gameId", MINECRAFT_GAME_ID.to_string()),
            ("classId", kind.curseforge_class_id().to_string()),
        ];
        let data: Data<Vec<RawCategory>> =
            send_json(self.get("/categories").query(&params)).await?;
        Ok(data
            .data
            .into_iter()
            .map(|category| Category {
                id: category.id.to_string(),
                name: category.name,
            })
            .collect())
    }

    /// Files without a `downloadUrl` need a website page for the user; build it from the project's URL.
    async fn attach_download_pages(&self, versions: &mut [ProjectVersion]) -> Result<()> {
        let mut project_ids: Vec<String> = versions
            .iter()
            .filter(|version| version.files.iter().any(|file| file.url.is_none()))
            .map(|version| version.project_id.clone())
            .collect();
        project_ids.sort();
        project_ids.dedup();
        let websites: HashMap<String, String> = self
            .projects(&project_ids)
            .await?
            .into_iter()
            .filter_map(|project| Some((project.id, project.website_url?)))
            .collect();

        for version in versions {
            let Some(website) = websites.get(&version.project_id) else {
                continue;
            };
            for file in version.files.iter_mut().filter(|file| file.url.is_none()) {
                file.download_page = Some(format!("{website}/download/{}", version.id));
            }
        }
        Ok(())
    }
}

fn parse_id(id: &str) -> Result<u32> {
    id.parse()
        .map_err(|_| Error::Other(format!("{id:?} is not a CurseForge id")))
}

/// Splits CurseForge's mixed `gameVersions` list: entries containing a dot are Minecraft versions,
/// the rest are loader names or side tags (ignored here).
pub(super) fn parse_game_versions(entries: &[String]) -> (Vec<String>, Vec<Loader>) {
    let versions = entries
        .iter()
        .filter(|entry| entry.contains('.'))
        .cloned()
        .collect();
    let loaders = entries
        .iter()
        .filter_map(|entry| Loader::from_modrinth_id(entry))
        .collect();
    (versions, loaders)
}

#[derive(Deserialize)]
struct Data<T> {
    data: T,
}

#[derive(Deserialize)]
struct Page<T> {
    data: Vec<T>,
    pagination: RawPagination,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawPagination {
    index: u32,
    total_count: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMod {
    id: u32,
    name: String,
    slug: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    download_count: f64,
    class_id: Option<u32>,
    #[serde(default)]
    categories: Vec<RawCategory>,
    #[serde(default)]
    authors: Vec<RawAuthor>,
    logo: Option<RawLogo>,
    links: Option<RawLinks>,
}

#[derive(Deserialize)]
struct RawCategory {
    id: u32,
    name: String,
}

#[derive(Deserialize)]
struct RawAuthor {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLogo {
    thumbnail_url: String,
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawLinks {
    website_url: Option<String>,
    issues_url: Option<String>,
    source_url: Option<String>,
    wiki_url: Option<String>,
}

impl From<RawMod> for Project {
    fn from(raw: RawMod) -> Self {
        let links = raw.links;
        let link = |pick: fn(&RawLinks) -> &Option<String>| {
            links
                .as_ref()
                .and_then(|links| pick(links).clone())
                .filter(|url| !url.is_empty())
        };
        Project {
            platform: Platform::CurseForge,
            id: raw.id.to_string(),
            slug: raw.slug,
            title: raw.name,
            description: raw.summary,
            body: None,
            icon_url: raw.logo.map(|logo| logo.url),
            kind: raw
                .class_id
                .and_then(ResourceKind::from_curseforge_class_id),
            side: None,
            authors: raw.authors.into_iter().map(|author| author.name).collect(),
            categories: raw
                .categories
                .into_iter()
                .map(|category| category.name)
                .collect(),
            website_url: link(|links| &links.website_url),
            issues_url: link(|links| &links.issues_url),
            source_url: link(|links| &links.source_url),
            wiki_url: link(|links| &links.wiki_url),
        }
    }
}

#[derive(Deserialize)]
struct RawFingerprintMatches {
    #[serde(rename = "exactMatches")]
    exact_matches: Vec<RawFingerprintMatch>,
}

#[derive(Deserialize)]
struct RawFingerprintMatch {
    file: RawFile,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawFile {
    id: u32,
    mod_id: u32,
    display_name: String,
    file_name: String,
    file_date: String,
    file_length: u64,
    download_url: Option<String>,
    release_type: u8,
    game_versions: Vec<String>,
    hashes: Vec<RawHash>,
    dependencies: Vec<RawDependency>,
    #[serde(default)]
    file_fingerprint: u32,
}

#[derive(Deserialize)]
struct RawHash {
    algo: u8,
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawDependency {
    mod_id: u32,
    relation_type: u8,
}

impl TryFrom<RawFile> for ProjectVersion {
    type Error = Error;

    fn try_from(raw: RawFile) -> Result<Self> {
        let release_type = match raw.release_type {
            1 => ReleaseType::Release,
            2 => ReleaseType::Beta,
            3 => ReleaseType::Alpha,
            other => {
                return Err(Error::Other(format!(
                    "unknown CurseForge releaseType {other}"
                )));
            }
        };
        let (game_versions, loaders) = parse_game_versions(&raw.game_versions);
        let sha1 = raw
            .hashes
            .into_iter()
            .find(|hash| hash.algo == 1)
            .map(|hash| hash.value.to_ascii_lowercase());
        let dependencies = raw
            .dependencies
            .into_iter()
            .map(|dependency| {
                let kind = match dependency.relation_type {
                    3 => DependencyKind::Required,
                    1 | 6 => DependencyKind::Embedded,
                    5 => DependencyKind::Incompatible,
                    // Optional (2) and tool (4) dependencies are never installed automatically.
                    _ => DependencyKind::Optional,
                };
                Dependency {
                    project_id: Some(dependency.mod_id.to_string()),
                    version_id: None,
                    kind,
                }
            })
            .collect();
        Ok(ProjectVersion {
            platform: Platform::CurseForge,
            id: raw.id.to_string(),
            project_id: raw.mod_id.to_string(),
            name: raw.display_name.clone(),
            version_number: raw.display_name,
            release_type,
            game_versions,
            loaders,
            files: vec![VersionFile {
                url: raw.download_url.filter(|url| !url.is_empty()),
                download_page: None,
                filename: raw.file_name,
                size: raw.file_length,
                hashes: Hashes { sha1, sha512: None },
                primary: true,
            }],
            dependencies,
            published: raw.file_date,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn splits_game_versions_loaders_and_side_tags() {
        let (versions, loaders) = parse_game_versions(&strings(&[
            "1.21.4", "Fabric", "Client", "1.21.3", "NeoForge", "Server",
        ]));
        assert_eq!(versions, ["1.21.4", "1.21.3"]);
        assert_eq!(loaders, [Loader::Fabric, Loader::NeoForge]);
    }

    #[test]
    fn maps_a_file_without_download_url() {
        let raw: RawFile = serde_json::from_str(
            r#"{"id":5,"modId":9,"displayName":"Pack 1.0","fileName":"pack-1.0.jar","fileDate":"2024-01-02T03:04:05Z",
            "fileLength":1234,"downloadUrl":null,"releaseType":2,"gameVersions":["1.20.1","Forge"],
            "hashes":[{"algo":2,"value":"md5"},{"algo":1,"value":"ABCDEF"}],
            "dependencies":[{"modId":10,"relationType":3},{"modId":11,"relationType":2}],"fileFingerprint":42}"#,
        )
        .unwrap();
        let version = ProjectVersion::try_from(raw).unwrap();
        assert_eq!(version.release_type, ReleaseType::Beta);
        assert_eq!(version.loaders, [Loader::Forge]);
        let file = version.install_file().unwrap();
        assert_eq!(file.url, None);
        assert_eq!(file.hashes.sha1.as_deref(), Some("abcdef"));
        assert_eq!(version.dependencies[0].kind, DependencyKind::Required);
        assert_eq!(version.dependencies[1].kind, DependencyKind::Optional);
    }

    #[test]
    fn api_key_is_redacted_in_debug_output() {
        let client = Client::new(reqwest::Client::new(), "super-secret-key").unwrap();
        assert!(!format!("{client:?}").contains("super-secret-key"));
    }

    #[test]
    fn rejects_non_numeric_ids() {
        assert!(parse_id("AANobbMI").is_err());
        assert_eq!(parse_id("306612").unwrap(), 306612);
    }
}
