//! Modrinth API v2 client (no authentication).

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::json;

use super::{
    Category, Dependency, DependencyKind, Hashes, Loader, Platform, Project, ProjectVersion,
    ReleaseType, ResourceKind, SearchHit, SearchPage, SearchQuery, SearchSort, Side, VersionFile,
    send_json,
};
use crate::error::{Error, Result};

const BASE_URL: &str = "https://api.modrinth.com/v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashAlgorithm {
    Sha1,
    Sha512,
}

impl HashAlgorithm {
    fn as_str(self) -> &'static str {
        match self {
            HashAlgorithm::Sha1 => "sha1",
            HashAlgorithm::Sha512 => "sha512",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
}

impl Client {
    /// `http` should come from `download::http_client()` so requests carry our User-Agent.
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            base_url: BASE_URL.to_owned(),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    pub async fn search(&self, query: &SearchQuery) -> Result<SearchPage> {
        let index = match query.sort {
            SearchSort::Relevance => "relevance",
            SearchSort::Downloads => "downloads",
            SearchSort::Follows => "follows",
            SearchSort::Newest => "newest",
            SearchSort::Updated => "updated",
        };
        let request = self.http.get(self.url("/search")).query(&[
            ("query", query.query.clone()),
            ("index", index.to_owned()),
            ("facets", build_facets(query)?),
            ("offset", query.offset.to_string()),
            ("limit", query.limit.to_string()),
        ]);
        let raw: RawSearch = send_json(request).await?;
        Ok(SearchPage {
            hits: raw
                .hits
                .into_iter()
                .map(|hit| SearchHit {
                    platform: Platform::Modrinth,
                    project_id: hit.project_id,
                    slug: hit.slug,
                    title: hit.title,
                    description: hit.description,
                    author: hit.author,
                    icon_url: hit.icon_url,
                    downloads: hit.downloads,
                    categories: hit.categories,
                })
                .collect(),
            offset: raw.offset,
            total_hits: raw.total_hits,
        })
    }

    /// `id` is a project id or slug.
    pub async fn project(&self, id: &str) -> Result<Project> {
        let raw: RawProject = send_json(self.http.get(self.url(&format!("/project/{id}")))).await?;
        Ok(raw.into())
    }

    pub async fn projects(&self, ids: &[String]) -> Result<Vec<Project>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let request = self
            .http
            .get(self.url("/projects"))
            .query(&[("ids", json!(ids).to_string())]);
        let raw: Vec<RawProject> = send_json(request).await?;
        Ok(raw.into_iter().map(Project::from).collect())
    }

    /// Versions of a project; empty `game_versions` or `loaders` mean no filter on that axis.
    pub async fn versions(
        &self,
        project_id: &str,
        game_versions: &[String],
        loaders: &[Loader],
    ) -> Result<Vec<ProjectVersion>> {
        let mut params = vec![("include_changelog", "false".to_owned())];
        if !game_versions.is_empty() {
            let versions: Vec<_> = game_versions.iter().map(|v| game_version_id(v)).collect();
            params.push(("game_versions", json!(versions).to_string()));
        }
        if !loaders.is_empty() {
            params.push(("loaders", json!(loader_ids(loaders)).to_string()));
        }
        let request = self
            .http
            .get(self.url(&format!("/project/{project_id}/version")))
            .query(&params);
        let raw: Vec<RawVersion> = send_json(request).await?;
        raw.into_iter()
            .filter_map(RawVersion::into_version)
            .collect()
    }

    pub async fn version(&self, version_id: &str) -> Result<ProjectVersion> {
        let raw: RawVersion =
            send_json(self.http.get(self.url(&format!("/version/{version_id}")))).await?;
        raw.into_version().ok_or_else(|| {
            Error::Other(format!(
                "Modrinth version {version_id} has no game versions"
            ))
        })?
    }

    /// Looks up the versions that own the given file hashes. Unknown hashes are absent from the map.
    pub async fn versions_by_hashes(
        &self,
        hashes: &[String],
        algorithm: HashAlgorithm,
    ) -> Result<HashMap<String, ProjectVersion>> {
        if hashes.is_empty() {
            return Ok(HashMap::new());
        }
        let body = json!({ "hashes": hashes, "algorithm": algorithm.as_str() });
        collect_versions(send_json(self.http.post(self.url("/version_files")).json(&body)).await?)
    }

    /// Latest version for each hash among those matching `loaders` and `game_versions` (empty means any).
    /// The map is keyed by the hash that was sent; hashes without a newer match are absent.
    pub async fn latest_versions_by_hashes(
        &self,
        hashes: &[String],
        algorithm: HashAlgorithm,
        loaders: &[Loader],
        game_versions: &[String],
    ) -> Result<HashMap<String, ProjectVersion>> {
        if hashes.is_empty() {
            return Ok(HashMap::new());
        }
        let game_versions: Vec<_> = game_versions.iter().map(|v| game_version_id(v)).collect();
        let mut body = json!({
            "hashes": hashes,
            "algorithm": algorithm.as_str(),
            "version_types": ["release", "beta", "alpha"],
        });
        if !loaders.is_empty() {
            body["loaders"] = json!(loader_ids(loaders));
        }
        if !game_versions.is_empty() {
            body["game_versions"] = json!(game_versions);
        }
        collect_versions(
            send_json(
                self.http
                    .post(self.url("/version_files/update"))
                    .json(&body),
            )
            .await?,
        )
    }

    /// Categories usable as `SearchQuery::categories` for projects of `kind`.
    pub async fn categories(&self, kind: ResourceKind) -> Result<Vec<Category>> {
        let project_type = kind.modrinth_project_type()?;
        let raw: Vec<RawCategory> = send_json(self.http.get(self.url("/tag/category"))).await?;
        Ok(raw
            .into_iter()
            .filter(|category| category.project_type == project_type)
            .map(|category| Category {
                id: category.name.clone(),
                name: category.name,
            })
            .collect())
    }

    /// Loader ids Modrinth knows, as raw strings.
    pub async fn loaders(&self) -> Result<Vec<String>> {
        let raw: Vec<RawLoader> = send_json(self.http.get(self.url("/tag/loader"))).await?;
        Ok(raw.into_iter().map(|loader| loader.name).collect())
    }
}

fn loader_ids(loaders: &[Loader]) -> Vec<&'static str> {
    loaders.iter().map(|loader| loader.modrinth_id()).collect()
}

/// Modrinth spells pre-releases `1.21-pre1` where Mojang's manifest says `1.21 Pre-Release 1`.
fn game_version_id(version: &str) -> String {
    version.replace(" Pre-Release ", "-pre").replace(' ', "-")
}

/// Facets are AND-ed groups of OR-ed terms; the project type group goes last.
pub(super) fn build_facets(query: &SearchQuery) -> Result<String> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    if !query.loaders.is_empty() {
        groups.push(
            query
                .loaders
                .iter()
                .map(|loader| format!("categories:{}", loader.modrinth_id()))
                .collect(),
        );
    }
    if let Some(version) = &query.game_version {
        groups.push(vec![format!("versions:{}", game_version_id(version))]);
    }
    for category in &query.categories {
        groups.push(vec![format!("categories:{category}")]);
    }
    groups.push(vec![format!(
        "project_type:{}",
        query.kind.modrinth_project_type()?
    )]);
    Ok(json!(groups).to_string())
}

fn collect_versions(raw: HashMap<String, RawVersion>) -> Result<HashMap<String, ProjectVersion>> {
    raw.into_iter()
        .filter_map(|(hash, version)| {
            version
                .into_version()
                .map(|version| version.map(|v| (hash, v)))
        })
        .collect()
}

#[derive(Deserialize)]
struct RawSearch {
    hits: Vec<RawHit>,
    offset: u32,
    total_hits: u32,
}

#[derive(Deserialize)]
struct RawHit {
    project_id: String,
    slug: String,
    title: String,
    description: String,
    author: String,
    icon_url: Option<String>,
    downloads: u64,
    categories: Vec<String>,
}

#[derive(Deserialize)]
struct RawProject {
    id: String,
    slug: String,
    title: String,
    description: String,
    body: Option<String>,
    icon_url: Option<String>,
    project_type: String,
    client_side: String,
    server_side: String,
    categories: Vec<String>,
    issues_url: Option<String>,
    source_url: Option<String>,
    wiki_url: Option<String>,
}

impl From<RawProject> for Project {
    fn from(raw: RawProject) -> Self {
        let supported = |side: &str| side == "required" || side == "optional";
        let side = match (supported(&raw.client_side), supported(&raw.server_side)) {
            (true, true) => Some(Side::Both),
            (true, false) => Some(Side::Client),
            (false, true) => Some(Side::Server),
            (false, false) => None,
        };
        let kind = match raw.project_type.as_str() {
            "mod" => Some(ResourceKind::Mod),
            "resourcepack" => Some(ResourceKind::ResourcePack),
            "shader" => Some(ResourceKind::ShaderPack),
            "datapack" => Some(ResourceKind::DataPack),
            "modpack" => Some(ResourceKind::Modpack),
            _ => None,
        };
        Project {
            platform: Platform::Modrinth,
            website_url: Some(format!(
                "https://modrinth.com/{}/{}",
                raw.project_type, raw.slug
            )),
            id: raw.id,
            slug: raw.slug,
            title: raw.title,
            description: raw.description,
            body: raw.body,
            icon_url: raw.icon_url,
            kind,
            side,
            authors: Vec::new(),
            categories: raw.categories,
            issues_url: raw.issues_url,
            source_url: raw.source_url,
            wiki_url: raw.wiki_url,
        }
    }
}

#[derive(Deserialize)]
struct RawVersion {
    id: String,
    project_id: String,
    name: String,
    version_number: String,
    version_type: String,
    game_versions: Vec<String>,
    loaders: Vec<String>,
    files: Vec<RawFile>,
    dependencies: Vec<RawDependency>,
    date_published: String,
}

#[derive(Deserialize)]
struct RawFile {
    url: String,
    filename: String,
    size: u64,
    primary: bool,
    hashes: Hashes,
}

#[derive(Deserialize)]
struct RawDependency {
    project_id: Option<String>,
    version_id: Option<String>,
    dependency_type: String,
}

impl RawVersion {
    /// `None` for versions without game versions, which Prism skips as well.
    fn into_version(self) -> Option<Result<ProjectVersion>> {
        if self.game_versions.is_empty() {
            return None;
        }
        Some(self.convert())
    }

    fn convert(self) -> Result<ProjectVersion> {
        let release_type = ReleaseType::parse(&self.version_type).ok_or_else(|| {
            Error::Other(format!(
                "unknown Modrinth version_type {:?}",
                self.version_type
            ))
        })?;
        let dependencies = self
            .dependencies
            .into_iter()
            .map(|dependency| {
                let kind = match dependency.dependency_type.as_str() {
                    "required" => DependencyKind::Required,
                    "optional" => DependencyKind::Optional,
                    "embedded" => DependencyKind::Embedded,
                    "incompatible" => DependencyKind::Incompatible,
                    other => {
                        return Err(Error::Other(format!(
                            "unknown Modrinth dependency_type {other:?}"
                        )));
                    }
                };
                Ok(Dependency {
                    project_id: dependency.project_id.filter(|id| !id.is_empty()),
                    version_id: dependency.version_id.filter(|id| !id.is_empty()),
                    kind,
                })
            })
            .collect::<Result<_>>()?;
        Ok(ProjectVersion {
            platform: Platform::Modrinth,
            id: self.id,
            project_id: self.project_id,
            name: self.name,
            version_number: self.version_number,
            release_type,
            game_versions: self.game_versions,
            loaders: self
                .loaders
                .iter()
                .filter_map(|id| Loader::from_modrinth_id(id))
                .collect(),
            files: self
                .files
                .into_iter()
                .map(|file| VersionFile {
                    url: Some(file.url),
                    download_page: None,
                    filename: file.filename,
                    size: file.size,
                    hashes: file.hashes,
                    primary: file.primary,
                })
                .collect(),
            dependencies,
            published: self.date_published,
        })
    }
}

#[derive(Deserialize)]
struct RawCategory {
    name: String,
    project_type: String,
}

#[derive(Deserialize)]
struct RawLoader {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> SearchQuery {
        SearchQuery {
            query: "sodium".into(),
            kind: ResourceKind::Mod,
            game_version: Some("1.21.4".into()),
            loaders: vec![Loader::Fabric, Loader::Quilt],
            categories: vec!["optimization".into()],
            sort: SearchSort::Relevance,
            offset: 0,
            limit: 20,
        }
    }

    #[test]
    fn facets_follow_prism_group_order() {
        assert_eq!(
            build_facets(&query()).unwrap(),
            r#"[["categories:fabric","categories:quilt"],["versions:1.21.4"],["categories:optimization"],["project_type:mod"]]"#
        );
    }

    #[test]
    fn facets_without_filters_only_have_project_type() {
        let query = SearchQuery {
            game_version: None,
            loaders: Vec::new(),
            categories: Vec::new(),
            kind: ResourceKind::ShaderPack,
            ..query()
        };
        assert_eq!(
            build_facets(&query).unwrap(),
            r#"[["project_type:shader"]]"#
        );
    }

    #[test]
    fn worlds_are_not_on_modrinth() {
        let query = SearchQuery {
            kind: ResourceKind::World,
            ..query()
        };
        assert!(build_facets(&query).is_err());
    }

    #[test]
    fn pre_release_versions_are_renamed() {
        assert_eq!(game_version_id("1.21 Pre-Release 1"), "1.21-pre1");
        assert_eq!(game_version_id("1.21.4"), "1.21.4");
    }

    #[test]
    fn parses_live_version_shape() {
        let raw: RawVersion = serde_json::from_str(
            r#"{"game_versions":["1.20.1"],"loaders":["fabric","quilt","datapack"],"id":"OihdIimA",
            "project_id":"AANobbMI","name":"Sodium 0.5.13 for Fabric","version_number":"mc1.20.1-0.5.13-fabric",
            "date_published":"2025-03-03T17:45:49.132919Z","version_type":"release",
            "files":[{"hashes":{"sha1":"bc","sha512":"81"},"url":"https://cdn.modrinth.com/a.jar",
            "filename":"a.jar","primary":true,"size":971552}],
            "dependencies":[{"project_id":"P7dR8mSH","version_id":null,"file_name":null,"dependency_type":"required"}]}"#,
        )
        .unwrap();
        let version = raw.into_version().unwrap().unwrap();
        assert_eq!(version.loaders, [Loader::Fabric, Loader::Quilt]);
        assert_eq!(version.dependencies[0].kind, DependencyKind::Required);
        assert_eq!(version.dependencies[0].version_id, None);
        assert_eq!(
            version.install_file().unwrap().hashes.sha512.as_deref(),
            Some("81")
        );
    }
}
