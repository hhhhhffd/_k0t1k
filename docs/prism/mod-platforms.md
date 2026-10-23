# Prism to _k0t1k: Modrinth and CurseForge guideline

Paths are relative to `<ref>/prism/launcher/` unless noted. `file:line` refers to the shallow clone. I called only the live Modrinth API. Every JSON field name under "Live Modrinth" below comes from real responses. The CurseForge shapes come from Prism's parsing code only.

## 0. Common conventions
- Modrinth base is `https://api.modrinth.com/v2` (`buildconfig/BuildConfig.h:196`). No auth. Send a descriptive `User-Agent`.
- CurseForge base is `https://api.curseforge.com/v1` (`BuildConfig.h:200`). The auth header is `x-api-key: <key>` (`net/ApiHeaderProxy.h:72`). Never log it.
- CurseForge `gameId` for Minecraft is `432` (`modplatform/flame/FlameAPI.cpp:295, 328`).
- Internal modloader flags (`modplatform/ModIndex.h:39-48`): NeoForge=1<<0, Forge=1<<1, Cauldron=1<<2, LiteLoader=1<<3, Fabric=1<<4, Quilt=1<<5, DataPack=1<<6, LegacyFabric=1<<9.
- Modrinth loader strings (`ModIndex.cpp:87-118`): neoforge, forge, cauldron, liteloader, fabric, quilt, datapack, babric, bta-babric, legacy-fabric, ornithe, rift.
- Modrinth lists `datapack` as a loader, so it must be filtered out of CurseForge queries (`FlameAPI.cpp:277`).
- Minecraft version strings need mapping for Modrinth (`ModrinthAPI.cpp:53-63`). The substring " Pre-Release " becomes "-pre", and spaces become "-". The reverse mapping is at `ModrinthPackIndex.cpp` ~L57-70.

## 1. Modrinth API v2

### 1.1 Search
`GET /search?offset=N&limit=25&query=<q>&index=<sort>&facets=<json>` (`ModrinthAPI.cpp:342-355`).

- Sort values for `index` (`:270-274`): relevance, downloads, follows, newest, updated.
- `facets` is a JSON array of OR-groups. The groups are ANDed together. Build it as in `createFacets` (`:105-144`):
  - Loader group: `["categories:fabric","categories:quilt"]`. Loaders are matched through `categories:`, not `loaders:`.
  - Game-version group: `["versions:1.20.1"]`.
  - Environment group (`:85-103`): client uses `environment:client_only`, `client_only_server_optional`, `singleplayer_only`, `client_or_server`, `client_or_server_prefers_both`. Server and universal groups are in the same function.
  - Category: one group per category, `["categories:<c>"]`.
  - Optional exclusion: `["disclosure_types!=<x>"]`, and `["open_source:true"]`.
  - Project type, always the last group: `["project_type:<t>"]`.
- `project_type` values (`ModrinthPackIndex.cpp:91-95`): mod, resourcepack, shader, datapack, modpack.
- Categories: `GET /tag/category`. Keep the entries whose `project_type` matches. Loaders: `GET /tag/loader`.

Live example (Prism's facet format). The URL-encoded `facets` value was `[["project_type:mod"],["categories:fabric"],["versions:1.20.1"]]`:
```
GET /v2/search?query=sodium&limit=1&facets=...
{"hits":[{"project_id":"AANobbMI","project_type":"mod","slug":"sodium","author":"jellysquid3",
 "title":"Sodium","description":"...","categories":["fabric","neoforge","optimization","quilt"],
 "versions":["1.16.3",...,"26.3"],"downloads":252026850,"follows":42057,
 "icon_url":"https://cdn.modrinth.com/data/AANobbMI/<hash>_96.webp",
 "date_created":"...","date_modified":"...","latest_version":"OihdIimA","license":"...",
 "client_side":"required","server_side":"unsupported","gallery":[...],"color":8703084}],
 "offset":0,"limit":1,"total_hits":39}
```
Search hits use `project_id`, but `/project/{id}` and `/projects` use `id` (Prism handles both, `ModrinthPackIndex.cpp` ~L102-104).

### 1.2 Projects
- `GET /project/{id|slug}` (`ModrinthAPI.cpp:312`).
- `GET /projects?ids=["a","b"]` (`ModrinthAPI.h:58-61`). The ids array is JSON in the query string.
- Live `/projects` fields: `client_side`, `server_side`, `game_versions`, `id`, `slug`, `project_type`, `team`, `title`, `description`, `body`, `categories`, `additional_categories`, `loaders`, `versions`, `icon_url`, `issues_url`, `source_url`, `wiki_url`, `discord_url`, `donation_urls`, `gallery`, `status`, `license`, `downloads`, `followers`.
- Prism reads `title`, `slug`, `project_type`, `client_side`/`server_side`, `body`, `issues_url`, `source_url`, `wiki_url`, `discord_url` and `donation_urls[]{id,platform,url}`.
- Side mapping: `required` or `optional` on a side means "supported" (`ModrinthPackIndex.cpp:11, 34-42`). Both supported gives Universal. Client only gives Client, and server only gives Server.

### 1.3 Versions
`GET /project/{id}/version?game_versions=["1.20.1"]&loaders=["fabric"]&include_changelog=false` (`ModrinthAPI.cpp:287-295`).

- Both filters are JSON arrays in the query string.
- Add `version_types` only in the POST update bodies.
- `GET /version/{id}` fetches one version. It is used for a dependency that has a `version_id` (`:301`).

Live version object (Sodium for 1.20.1/fabric):
```
{"game_versions":["1.20.1"],"loaders":["fabric","quilt"],"environment":"client_only",
 "id":"OihdIimA","project_id":"AANobbMI","author_id":"...","featured":false,
 "name":"Sodium 0.5.13 for Fabric","version_number":"mc1.20.1-0.5.13-fabric",
 "changelog":"...","changelog_url":null,"date_published":"2025-03-03T17:45:49.132919Z",
 "downloads":12551616,"version_type":"release","status":"listed","requested_status":null,
 "files":[{"id":"KcvCzyWY","hashes":{"sha1":"bcdbf37d...","sha512":"81c64f9c..."},
   "url":"https://cdn.modrinth.com/data/AANobbMI/versions/OihdIimA/sodium-fabric-0.5.13%2Bmc1.20.1.jar",
   "filename":"sodium-fabric-0.5.13+mc1.20.1.jar","primary":true,"size":971552,"file_type":null}],
 "dependencies":[]}
```
Dependency entries use `project_id`, `version_id`, `file_name` and `dependency_type`. The `dependency_type` values are required, optional, incompatible and embedded (`ModrinthPackIndex.cpp:230-250`). `project_id` or `version_id` may be empty or null.

Prism's version parse (`ModrinthPackIndex.cpp:182-300`):
- A version with empty `game_versions` is skipped.
- Recognised loaders are only neoforge, forge, cauldron, liteloader, fabric and quilt.
- File choice (`:255-275`): scan for a file whose `filename` contains `preferredFileName`. Otherwise take the one with `primary=true`. Otherwise take the last file. Note the loop stops at `count-1`.
- Hash choice: use `preferredHashType` (sha512 first, else sha1).
- `fileName` goes through `FS::RemoveInvalidPathChars`.
- `isPreferred = primary || files.count()==1`.

### 1.4 Hash lookup and updates
- Single file: `GET /version_file/{hash}?algorithm=sha1|sha512` (`:152`). Live result: it returns a full Version object.
- Batch: `POST /version_files` with `{"hashes":[...],"algorithm":"sha512"}` (`:158-175`). The result is an object `{hash: Version}`. Missing hashes are absent from it.
- Single update: `POST /version_file/{hash}/update?algorithm=..` with `{"loaders":[..],"game_versions":[..],"version_types":["release","beta","alpha"]}` (`:186-216`).
- Batch update: `POST /version_files/update` with `{"hashes":[..],"algorithm":"sha512","loaders":[..],"game_versions":[..],"version_types":[..]}` (`:220-253`). I ran it live. It returns `{"<oldhash>": {Version}}`, where the key is the hash you sent.
- All hashes in one call must use one algorithm. Prism defaults to `hashType(MODRINTH).first()` = sha512 (`ModIndex.cpp:76`). If the stored hash is sha1, it re-hashes the file (`ModrinthCheckUpdate.cpp:64-70`).

Modrinth update algorithm (`ModrinthCheckUpdate.cpp:50-234`):
1. Map the stored hash to the resource.
2. Call batch update with the instance's loaders and the MC version. Then retry per remaining loader (`checkNextLoader`).
   - The initial loader list is the instance's loaders.
   - After it, the loaders found in resource metadata are appended.
   - Those extra loaders are queried only for resources whose metadata has that loader.
3. For each response: skip if the object is empty. Parse with `loaderFilter` = first loader name, which prefers the file whose name matches that loader.
4. Skip if `downloadUrl` is empty.
5. An update exists when `version.hash != oldHash && version.isPreferred`, or when the resource is NotInstalled.
6. Resources that match no loader pass end up as "No valid version found".
7. Skip resources with `lockUpdate=true` (flag in `.pw.toml`).

### 1.5 Dependencies (`minecraft/mod/tasks/GetModDependenciesTask.cpp`)
- Only `REQUIRED` dependencies are followed (`:148`).
- Skip a dependency if any of these hold: it is already in the candidate list, already selected, already installed (`provider` and `projectId`, or `fileId` when only a version is known), or already queued.
- Resolution: `/version/{version_id}` if a version is pinned. Otherwise `/project/{id}/version?game_versions=["<mc>"]&loaders=[...]&include_changelog=true`. Filter to versions whose `loaders` intersect the instance's loaders. Sort by `date_published` descending and take the first (`ResourceAPI.cpp:90-145`).
- Recursion depth is limited to 20 (`:121`).
- Fabric to Quilt overrides (`ModIndex.h:344-348`): Modrinth Fabric API `P7dR8mSH` maps to QFAPI `qvIfYCYJ`, Kotlin libs `Ha28R6CL` to `lwVhp9o5`. CurseForge uses 306612/634179 and 308769/720410. Quilt can use Fabric API. If the QFAPI lookup fails, retry with the Fabric one.

### 1.6 Other Modrinth notes
- Prism sends a `ModrinthDownloadMeta` (reason, gameVersion, loader, dependentOn) with downloads. It is telemetry-like and optional. See `ResourceDownloadTask.cpp:38-51`.
- mrpack hosts allowed without a prompt (`BuildConfig.h:197`): `cdn.modrinth.com`, `github.com`, `raw.githubusercontent.com`, `gitlab.com`. Other hosts trigger an "untrusted" warning (`ModrinthInstanceCreationTask.cpp:462-495`).

## 2. CurseForge API

### 2.1 Endpoints (`modplatform/flame/FlameAPI.cpp`)
- Search: `GET /mods/search?gameId=432&classId=<c>&index=<offset>&pageSize=25&searchFilter=<q>&sortField=<n>&sortOrder=desc&modLoaderTypes=[1,4]&categoryIds=[..]&gameVersion=<one version>` (`:268-295`). `index` is the offset. Only the first game version is sent. Loaders are sent as a JSON list.
- `sortField` values (`:128-135`): 1 Featured, 2 Popularity, 3 LastUpdated, 4 Name, 5 Author, 6 TotalDownloads, 7 Category, 8 GameVersion.
- `classId` values (`:142-146`): Mod 6, ResourcePack 12, World 17, ShaderPack 6552, Modpack 4471, DataPack 6945.
- `modLoaderType` enum (`:151-175`): 1 Forge, 2 Cauldron, 3 LiteLoader, 4 Fabric, 5 Quilt, 6 NeoForge. Only Forge, Fabric, Quilt and NeoForge are used for search (`getModLoaderStrings`).
- Project: `GET /mods/{id}`. Response is `{data:{...}}`. Fields read: `id`, `name`, `slug`, `summary`, `classId`, `links.{websiteUrl,issuesUrl,sourceUrl,wikiUrl}`, `logo.{title,thumbnailUrl,url}`, `authors[].{name,url}` (`FlamePackIndex.cpp:11-70`).
- Batch projects: `POST /mods` with `{"modIds":[..]}`.
- Description (HTML): `GET /mods/{id}/description`, field `data`.
- Files for a project: `GET /mods/{id}/files?pageSize=10000[&gameVersion=<v>][&modLoaderType=<n>]` (`:218-228`). `modLoaderType` is sent only when exactly one loader is selected and it is not datapack.
- Batch files: `POST /mods/files` with `{"fileIds":[..]}`. Response `data[]` (`:84-100`).
- Single file: `GET /mods/{modId}/files/{fileId}`. Changelog: `GET /mods/{modId}/files/{fileId}/changelog`, field `data` (HTML).
- Categories: `GET /categories?gameId=432&classId=<c>`, entries `{id,name}`.
- Fingerprints: `POST /fingerprints` with `{"fingerprints":["123",..]}` (decimal strings). Response `data.exactMatches[].file`; the match key is `file.fileFingerprint` (`EnsureMetadataTask.cpp:372-396`). The official docs also describe `/fingerprints/432`. Prism uses the generic path.

### 2.2 File JSON (`FlameModIndex.cpp:50-141`)
- `id` is the fileId and `modId` the projectId.
- Other fields: `displayName`, `fileName`, `fileDate` (RFC3339, sorted as a string), `downloadUrl` (null when disabled), `releaseType` (1 release, 2 beta, 3 alpha), `gameVersions[]`, `hashes[]`, `dependencies[]`.
- `gameVersions` mixes real MC versions, loader names and side tags. Keep entries that contain "." as MC versions. Lowercase-match neoforge, forge, cauldron, liteloader, fabric, quilt as loaders. Entries "client" and "server" give the side. If both appear, the side is Universal.
- `hashes[]` is `{algo,value}`. algo 1 means sha1 and algo 2 means md5. Take the first one that is known. Prism prefers sha1.
- `dependencies[]` is `{modId, relationType}`. Mapping: 1 Embedded, 2 Optional, 3 Required, 4 Tool, 5 Incompatible, 6 Include.
- Sorting versions: by `fileDate` descending (`loadIndexedPackVersions`, `:32-37`).
- Latest version choice (`FlameAPI.cpp:~385-430`): filter by release types, then take the newest `date` per loader. The `checkLoaders=false` path takes the newest of all. Update-check files are fetched with the MC version only, with no loader parameter (`FlameCheckUpdate.cpp:50`). Loader filtering happens client-side.
- Update check also compares `hash != latest.hash`. A file with empty `downloadUrl` and a different fileId is reported as "blocked", with the recover URL `<websiteUrl>/download/<fileId>` (`FlameCheckUpdate.cpp:101-103, 125-195`).

### 2.3 Fingerprint (murmur2 variant)
- Call site: `modplatform/helpers/HashUtils.cpp:104-110`. `Murmur2::hash(reader, 4*MiB, should_filter_out)` with filter `c==9||c==10||c==13||c==32` (tab, LF, CR, space), seed 1. The result is a decimal string of the uint32.
- Algorithm (`<ref>/prism/libraries/murmur2/src/MurmurHash2.cpp:18-106`):
  1. Pass 1: count bytes that are not filtered. This is `len`.
  2. Initial `h = 1 ^ len`. `m = 0x5bd1e995`, `r = 24`.
  3. Pass 2: skip filtered bytes and gather the rest in groups of 4. For each full 4-byte group, read a little-endian u32 `k`: `k*=m; k^=k>>r; k*=m; h*=m; h^=k`.
  4. Tail (len%4 bytes): `h ^= bytes << 8*i`, then `h*=m`, only when len%4 is not 0.
  5. Finalise: `h^=h>>13; h*=m; h^=h>>15`. All arithmetic is u32 wrapping.
- Note: it must be computed over the whole file, jar included. There is no extra lowercasing. Prism's loop stops reading on `eof()` after each chunk, which in Rust just becomes a single streaming pass.

### 2.4 Blocked mods (`downloadUrl == null`)
Flow in `modplatform/flame/FileResolvingTask.cpp`:
1. `POST /mods/files` with all manifest fileIds.
2. For a missing `downloadUrl` where a sha1 exists, the optional setting `FallbackMRBlockedMods` makes Prism look the sha1 up with `POST /version_files` (`algorithm: sha1`) on Modrinth. If found, it uses the Modrinth `downloadUrl` (`:104-170`).
3. `POST /mods` for the project info (`getFlameProjects`, `:190-257`). A project of class World gets `targetFolder="saves"`.

In `FlameInstanceCreationTask.cpp:255-325` (`idResolverSucceeded`):
- Optional files (`required=false`) go to an optional-mod dialog first.
- Anything with an empty `downloadUrl` becomes a `BlockedMod{name=fileName, websiteUrl=<project.websiteUrl>/download/<fileId>, hash=sha1, targetFolder, disabled=!required && !selected}`.
- `BlockedModsDialog` (`ui/dialogs/BlockedModsDialog.*`) watches the Downloads dir (plus global mods dir). It hashes dropped or new files with sha1 (default `hashType="sha1"`) and matches `mod.hash.compare(hash, CaseInsensitive)==0` (`BlockedModsDialog.cpp:283-300`).
- When a file's `hash` is empty, any file whose name matches counts as a match (`:356-362`).
- Matched files are moved from the Downloads dir and copied from elsewhere, to `<root>/<targetFolder>/<name>[.disabled]` (`copyBlockedMods`, `:~350-390`). Unmatched ones are skipped.
- A downloaded file that Prism cannot place gets moved by `validateOtherResources` using `ResourceUtils::identify`.
- Not done by Prism: the CurseForge CDN URL `https://edge.forgecdn.net/files/{id/1000}/{id%1000}/{fileName}`. I believe it is commonly used, but it is not in this code, so treat it as an unverified suggestion.

## 3. Local metadata (packwiz style)

Location: `<resource folder>/.index/<slug>.pw.toml` (`ResourceFolderModel.h:83`). For the shader pack model, `indexDir()` is the shaderpacks folder itself (`ShaderPackFolderModel.h:24`).

Writer: `modplatform/packwiz/Packwiz.cpp:145-247`. Reader: `:269-391`. Example:
```toml
name = "Sodium"
filename = "sodium-fabric-0.5.13+mc1.20.1.jar"
side = "client"                       # client | server | both | ""
x-prismlauncher-loaders = ["fabric", "quilt"]
x-prismlauncher-mc-versions = ["1.20.1"]
x-prismlauncher-release-type = "release"   # case-insensitive; Unknown|Release|Beta|Alpha
x-prismlauncher-version-number = "mc1.20.1-0.5.13-fabric"
x-prismlauncher-lock-update = false
x-prismlauncher-dependencies = [ { addonId = "...", type = "REQUIRED", version = "..." } ]  # version optional
[download]
mode = "url"                          # "metadata:curseforge" for CurseForge
url = "https://cdn.modrinth.com/..."  # empty for CurseForge
hash-format = "sha512"                # sha1|sha512|md5
hash = "..."
[update.modrinth]                     # exactly one provider table
mod-id = "AANobbMI"
version = "OihdIimA"
# or:
[update.curseforge]
file-id = 123
project-id = 456
```
- Provider table keys are `ProviderCapabilities::name`, i.e. `modrinth` and `curseforge`. The reader checks `curseforge` first.
- `createModFormat` (`Packwiz.cpp:107-143`):
  - `mode`/`url` for CurseForge versus Modrinth, as above.
  - `side` comes from the version, else from the project.
  - `mcVersions` are deduplicated and sorted by version comparison.
  - `versionNumber` falls back to the version name (`displayName`) on CurseForge.
- Validity (`Packwiz.h`): `slug` non-empty and `projectId` non-null. CurseForge writes nothing if fileId or projectId is 0. Modrinth writes nothing if mod-id or version is empty.
- File name is `<slug>.pw.toml`. Lookup is case-insensitive (`getRealIndexName`, `:41-63`). A differently-cased file gets renamed.
- Lookup by project id scans every index file (`getIndexForMod(indexDir, modId)`, `:393-404`). Writing deletes the existing file first.
- Orphan handling: metadata with no matching file gets status `NotInstalled`. With `m_clean_orphan` (`ResourceFolderLoadTask.cpp:115-126`), orphans are deleted. Matching is by `metadata.filename`, which is why `.disabled` files get matched via `internalId().chopped(9)` (`:96-111`).
- Installing/updating a resource (`LocalResourceUpdateTask.cpp:52-73`): reuse the old slug if the project slug is empty, write the new `.pw.toml`, and `lockUpdate` is set to the flag (true for modpack-installed files). After download, the old file is removed unless the name is unchanged (`ResourceDownloadTask.cpp:105-130`). For shaderpacks the `<name>.txt` config is renamed along with it.
- Metadata for resources with no `.pw.toml` is created by hashing (EnsureMetadataTask, section 3.1).

### 3.1 EnsureMetadataTask (`modplatform/EnsureMetadataTask.cpp`)
- Modrinth: sha512 each file. `POST /version_files`. Then `/projects?ids=[..]` (or `/project/{id}` when there is only one). Then write the `.pw.toml`.
- CurseForge: murmur2 each file. `POST /fingerprints`. Then `POST /mods` (or `GET /mods/{id}`). Then write the `.pw.toml`.
- Skip resources that already have metadata from the same provider, and folders.
- Run Modrinth first and CurseForge for the leftovers. Prism's UI does this per provider selection, so that ordering is a suggestion.

## 4. Resource types and files

Folders under the instance game root (`MinecraftInstance.cpp:425-455`):
- `mods` (loader mods; `coremods` and `nilmods` also exist)
- `resourcepacks`
- `texturepacks` (pre-1.6)
- `shaderpacks`
- `datapacks` (this path can be overridden by the `GlobalDataPacksPath` setting)
- `saves` for worlds

Enable and disable (`minecraft/mod/Resource.cpp:44-70, 262-310`):
- Disabled means the file name has the suffix `.disabled`. Disabling appends it. If the target name already exists, `FS::getUniqueResourceName` picks a new one. Enabling chops 9 characters, and fails if the suffix is absent.
- Directories cannot be toggled.
- The loader strips `.disabled` first, then `.zip`/`.jar` (also `.nilmod`, `.litemod`) to build the display name.
- The load task (`ResourceFolderLoadTask.cpp:72-113`) moves files with duplicate names to a unique name.

Identification order (`minecraft/mod/tasks/LocalResourceParse.cpp:25-59`): mod, resource pack, texture pack, data pack, world, shader pack. Mods are tested first because mod jars may contain packs.
- Shader pack: has a `shaders/` folder at the root, or one level deep (`LocalShaderPackParseTask.cpp:46-90`).
- Texture pack: `pack.png` (`LocalTexturePackParseTask.cpp`).
- Data pack: `pack.mcmeta` with `pack_format`, optional `supported_formats` or min/max, `description`, icon `pack.png` (`LocalDataPackParseTask.cpp`).

Mod jar parsing (`LocalModParseTask.cpp:581-680`). Inspect zip entries by exact path:
- `META-INF/mods.toml` gives Forge. `META-INF/neoforge.mods.toml` gives NeoForge (`:598-610`).
  - Read the first `[[mods]]` table only (`:132-282`): `modId`, `version`, `displayName`, `description`, `authors`, `displayURL`, `logoFile` (also checked at top level), `issueTrackerURL`, `license`.
  - `version = "${file.jarVersion}"` is replaced by the manifest's `Implementation-Version` from `META-INF/MANIFEST.MF` (`:605, 611-625`). The value NONE is used if that is missing.
  - Dependencies: type "required" or `mandatory=true`. Ignore the ids "", forge, neoforge, minecraft.
- `fabric.mod.json` (`:285-380`): `id`, `version`, `name` (falls back to id), `description`, `authors[]` (string or `{name}`), `contact.homepage`/`issues`, `license`, `icon`, `depends`. `icon` is a string, or an object keyed "NxN" where the largest key wins. `schemaVersion >= 1` is required for the icon and extra fields. Ignore `fabricloader`, `minecraft` and `fabric-*` in `depends`.
- `quilt.mod.json` (`:383-495`): everything is under `quilt_loader` (`id`, `version`, `metadata.{name,description,contributors,contact,license,icon}`, `depends` with items that may be strings or `{id,optional}`). Ignore `minecraft` and `quilt_*`.
- `mcmod.info` (`:27-130`): it may be a JSON array or a `modList`/`modlist` object. Use the first entry: `modid`, `name`, `version`, `description`, `authorList` or `authors`, `url`, `logoFile`. The name "Example Mod" is ignored.
- `litemod.json` for liteloader (`:518-542`). `forgeversion.properties` for old Forge. `*.nilmod.css` and `META-INF/nil/mappings.json` for nilmod.
- The icon is loaded from the zip entry named by `icon_file` and decoded as a PNG.
- Prism checks one file entry at a time and stops as soon as a mod descriptor is accepted. It does not guess the loader from the file name.

## 5. Modpack import

Detection (`InstanceImportTask.cpp:143-172`) scans zip entries in this priority:
1. `modrinth.index.json` anywhere at the zip root (exact path) means Modrinth.
2. `bin/modpack.jar` or `bin/version.json` means Technic.
3. `manifest.json` (exact root path) means CurseForge. Prioritise this check after Modrinth so an `overrides/manifest.json` does not confuse it.
4. Any `instance.cfg` means MultiMC/Prism. `root` is that file's directory.

If nothing matches, fail with "Archive does not contain a recognized modpack type."

Extraction (`:183-255`): extract the whole zip to a staging dir (for MultiMC only below `root`). Then fix permissions (+rw files, +rwx dirs for the user). Dispatch by type.

### 5.1 .mrpack (`ModrinthInstanceCreationTask.cpp:167-442`)
`modrinth.index.json`:
- `formatVersion` must be 1. `game` must be "minecraft".
- `versionId`, `name`, optional `summary`.
- `files[]` entries: `path`, `hashes.{sha1,sha512}`, `downloads[]`, `fileSize`, and optional `env.{client,server}`. The `env` values are "required", "optional" or "unsupported".
- `dependencies{}` keys: `minecraft`, `fabric-loader`, `quilt-loader`, `forge`, `neoforge`. Any other key is an error in Prism (`:401-403`). I would warn instead of failing.

Rules:
- `path` has backslashes replaced with "/". Strip invalid chars, then confirm it stays inside the game root (`:257-266`). Reject the pack otherwise.
- Client install: `env.client == "unsupported"` is skipped. `"optional"` goes to an optional dialog and unselected ones get `path + ".disabled"`. The server side is not checked.
- The sha512 hash is required (`:359-362`). Download with a checksum validator. `downloads` must not be empty. URLs are parsed in tolerant mode because Modrinth may emit unescaped spaces. Try `downloads` in order as fallbacks.
- Extraction order: `overrides/` is moved into the game root. Then `client-overrides/` is merged over it (`FS::overrideFolder`). Both are recorded in `mrpack/overrides.txt` and `mrpack/client-overrides.txt` (`OverrideUtils`) so a later update can delete the old overrides. The index is kept at `mrpack/modrinth.index.json`.
- Components: `net.minecraft` is set to `dependencies.minecraft`. Loader mapping:
  - `fabric-loader` to `net.fabricmc.fabric-loader`
  - `quilt-loader` to `org.quiltmc.quilt-loader`
  - `forge` to `net.minecraftforge`
  - `neoforge` to `net.neoforged`
- After download, for files under `mods/`, Prism runs EnsureMetadata over sha512 to create the `.pw.toml` (`:444-460`). It marks them lock-update.
- Update flow: diff old and new `files[]` by hash, delete removed files, and delete the old overrides.
- Hosts outside the `MODRINTH_MRPACK_HOSTS` list trigger an untrusted dialog. Existing files in `mods` from overrides are also listed.

### 5.2 CurseForge zip (`flame/PackManifest.cpp`, `FlameInstanceCreationTask.cpp`)
`manifest.json`:
- `manifestType` must be "minecraftModpack". `manifestVersion` must be 1.
- `name` (default "Unnamed"), `version`, `author` (default "Anonymous"), `overrides` (default "overrides").
- `minecraft.version`, `minecraft.modLoaders[].{id,primary}`, `minecraft.recommendedRam`.
- `files[].{projectID,fileID,required}`. `required` defaults to true.

Rules (`FlameInstanceCreationTask.cpp:89-253`):
- `overrides` must resolve inside the staging dir. Otherwise reject it. It is moved into the game root.
- Loader id prefix mapping (`:135-165`). Iterate all `modLoaders` and let the last recognised one win.
  - `neoforge-<ver>` becomes `net.neoforged`, and `neoforge-1.20.1-<ver>` is stripped to `<ver>` too.
  - `forge-<ver>` becomes `net.minecraftforge`.
  - `fabric-<ver>` becomes `net.fabricmc.fabric-loader`.
  - `quilt-<ver>` becomes `org.quiltmc.quilt-loader`.
  - Unknown ids are logged and skipped.
- `minecraft.version` may end in dots. Trim them (`:172-177`).
- `recommended` loader version is resolved via Prism's metadata index (`getVersionForLoader`, `:290-340`). Prism's meta server is not available to _k0t1k, so it needs its own meta source.
- Jarmods: files in `jarmods/` are installed via the pack profile.
- RAM: if `recommendedRam > 0` and the instance is new, set `MaxMemAlloc`. Cap at 90% of system RAM (`:202-218`).
- Resolve files: `POST /mods/files` (all fileIds), then Modrinth fallback for blocked ones, then `POST /mods` for the project info.
- Target folder: `targetFolder` defaults to `mods`. A World class gets `saves`. Everything else is corrected after download (`validateOtherResources`, `:~690-760`) by identifying the file type. Resource packs, texture packs, data packs and shader packs are moved to their own folders. Worlds are installed into `saves`.
- Download each file to `<root>/<targetFolder>/<fileName>`. Unselected optional files get `.disabled`. Use `getSafePathInRoot` to prevent path escape (`:327-347`).
- Metadata: `.pw.toml` files are written into `mods/.index` for entries whose target is `mods` (skipping `.zip` that did not identify as a mod) with `lockUpdate=true` (`:~770-790`).
- The manifest is kept at `flame/manifest.json`. Update flow is the same as Modrinth: diff `fileID` per `projectID`.

### 5.3 Prism/MultiMC zip (`InstanceImportTask.cpp:352-377`)
- The zip root (or the directory holding `instance.cfg`) becomes the instance dir. Expected layout: `instance.cfg`, `mmc-pack.json`, `.minecraft/` or `minecraft/`, an optional icon.
- On import, reset time played, regenerate the instance UUID, set the new name, and install the icon.
- The instance gets imported mostly as is.

### 5.4 Export (brief)
- mrpack (`ModrinthPackExportTask.cpp`): include files under `mods/`, `coremods/`, `resourcepacks/`, `texturepacks/`, `shaderpacks/` with extensions jar, litemod, zip (and `.disabled`). Hash each with sha512. Resolve via `/version_files`, then pick the sha1 and url. Write `modrinth.index.json` with `formatVersion:1`, `game:"minecraft"`, `versionId`, `name`, `summary`, `files[]{path,hashes{sha1,sha512},fileSize,downloads[url],env{client,server}}`, `dependencies{minecraft, fabric-loader, ...}`. Optional files: strip `.disabled` and set env optional. Client-only mods get `env.server = "unsupported"`. The rest goes under `overrides/` in the zip.
- CurseForge zip (`FlamePackExportTask.cpp:~376-432`): `manifest.json` with `manifestType:"minecraftModpack"`, `manifestVersion:1`, `overrides:"overrides"`, `minecraft.version`, `minecraft.modLoaders:[{id:"forge-47.2.0", primary:true}]`, `files[{projectID,fileID,required}]`. Hash with murmur2 and call `/fingerprints`. Resource-pack zips are hashed too. Also writes an HTML modlist.

## 6. Recommendations for the Rust implementation
1. Model `Provider {Modrinth, CurseForge}`, `IndexedVersion {addon_id, file_id, version_name, version_number, release_type, mc_versions, loaders, url (Option), file_name, hash+algo, deps, side, date}`. Keep these as separate parsing paths for the two APIs, then unify.
2. Implement murmur2 (section 2.3) exactly. Cross-check against a known fingerprint from a CurseForge `fileFingerprint` for a real jar before trusting it.
3. Batch where Prism does: `/version_files`, `/version_files/update`, `/projects`, CurseForge `/fingerprints`, `/mods`, `/mods/files`. Never loop per file.
4. Treat `downloadUrl == null` as first class. Offer the Modrinth sha1 fallback, then a blocked-mods flow that matches dropped files by sha1.
5. Reuse the packwiz `.pw.toml` layout (section 3) for compatibility with Prism and packwiz-aware tools. Keep the `x-prismlauncher-*` keys. Use `toml` crate round trips, and keep reading tolerant (missing keys become defaults).
6. Always validate output paths with a `safe_join(root, rel)` check for both mrpack `path` and CurseForge `overrides` (zip-slip). Also reject absolute paths and `..`.
7. Downloads: verify the sha512 (mrpack) and the sha1/md5 from the CurseForge hash list, retry fallback URLs in order, and apply an allow-list (section 1.6) with a warning for other hosts.
8. Do not put the CurseForge key in logs, URLs or error messages. Keep it in a header only and redact when printing requests.
9. Uncertainties: (a) I could not check CurseForge response shapes against the live API without a key. (b) The Modrinth `/version_file/{hash}/update` response is not shown in detail, only the batch form was run. (c) The exact line numbers for `FlameAPI::getLatestVersion` and `validateOtherResources` are approximate (`~`).
