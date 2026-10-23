# Prism install/launch guideline for _k0t1k (Rust/Tauri)

Prism source root: `ref/prism/launcher/` (called `L/` below). The paths below are relative to `/tmp/claude-1000/-home-user--k0t1k/04a26e0f-a5f4-4e1b-93a8-3a5d7ec20881/scratchpad/`. Live meta samples I curled are in `m/` under the same directory: `index.json`, `net.minecraft_index.json`, `net.minecraft_1.21.4.json`, `forge.json` (47.4.28), `neo.json` (21.4.161), `quilt.json`, `lwjgl3.json`, `lwjgl2.json`, `net.fabricmc.intermediary_1.21.4.json`. Fabric 0.16.9 and the Forge 14.23.5.2860 file were only printed and not saved. The meta numbers and versions below were seen on 2026-10-23.

## 0. Bottom line (details in section 8)
- Use Prism meta (`https://meta.prismlauncher.org/v1/`) for everything: vanilla, Fabric, Quilt, Forge and NeoForge.
- Prism has no Forge or NeoForge installer code. Modern Forge and NeoForge are launched through the ForgeWrapper jar, which Prism meta puts on the classpath as `mainClass`.
- ForgeWrapper runs the installer processors inside the game JVM on first launch.
- The only extra code you need is a component resolver, a library merger and a command-line builder.

## 1. Meta server shapes (verified by curl)
All URLs are under `https://meta.prismlauncher.org/v1/`. The base URL is `L/../CMakeLists.txt:205` (`Launcher_META_URL`).

**Package index**: `GET index.json`
- Shape: `{formatVersion:1, packages:[{name, sha256, uid}]}`.
- UIDs seen: `net.minecraft`, `org.lwjgl`, `org.lwjgl3`, `net.fabricmc.fabric-loader`, `net.fabricmc.intermediary`, `org.quiltmc.quilt-loader`, `net.minecraftforge`, `net.neoforged`, `com.mumfrey.liteloader`, and the Java runtime packages `com.azul.java`, `com.ibm.java`, `net.adoptium.java`.
- The package `sha256` is the hash of that package's `index.json`.

**Per-package version list**: `GET <uid>/index.json`
- Shape: `{formatVersion, name, uid, versions:[{version, type, releaseTime, recommended, requires:[...], conflicts?:[...], volatile?, sha256}]}`.
- The version `sha256` is the hash of `<uid>/<version>.json`. Use it for cache validation, so you don't need ETags.
- `net.minecraft`: 1026 versions. Each has `type` (`release`/`snapshot`/`old_beta`/`old_alpha`) and `requires:[{uid:"org.lwjgl3", suggests:"3.3.3"}]`.
  - Exactly one version has `recommended:true`, and it is the latest release (26.3 at the time).
  - There is no "latest" field. Sort by `releaseTime` (list order is newest first).
- Require entries have the form `{uid, equals?, suggests?}`.
  - `equals` is a hard pin.
  - `suggests` is a soft default.
  - A bare `{uid}` means "any version".
- Loader indexes also carry `requires`. The Fabric and Quilt index entries have `requires:[{uid:"net.fabricmc.intermediary"}]`, with no version.
  - Forge and NeoForge entries have `requires:[{equals:"<mc>", uid:"net.minecraft"}]`. This is how you list loader versions for an MC version: filter `versions` by `requires[0].equals == mc`. For `1.20.1`, Forge gives 47.4.28 and so on.
  - Quilt and Fabric versions are not tied to MC versions in the index. The intermediary version equals the MC version (see §2).
- `org.lwjgl3` versions: 3.4.3, 3.3.6, 3.3.3, 3.3.2, 3.3.1, 3.2.2, 3.2.1, 3.1.6, 3.1.2. Entries carry `conflicts:[{uid:"org.lwjgl"}]`.
- `org.lwjgl` (LWJGL 2) has `conflicts:[{uid:"org.lwjgl3"}]`.

**Version file**: `GET <uid>/<version>.json`. This is the "OneSix" format (`L/minecraft/OneSixVersionFormat.cpp:84`). Fields:
- `formatVersion:1`, `uid`, `name`, `version`, `releaseTime`, `type`, `order`, `volatile`, `requires[]`, `conflicts[]`
- `libraries[]`, `mavenFiles[]`, `mainClass`, `mainJar`, `minecraftArguments` (string)
- `+jvmArgs[]`, `+tweakers[]`, `+traits[]`, `+agents[]`, `jarMods[]`, `appletClass`
- `assetIndex`, `compatibleJavaMajors[]`, `compatibleJavaName`, `logging`, `runtimes[]`

**`net.minecraft/1.21.4.json` (curled)**
- `order:-2`, `mainClass:"net.minecraft.client.main.Main"`.
- `minecraftArguments` is a flat string, already flattened from Mojang's `arguments` block:
  - `--username ${auth_player_name} --version ${version_name} --gameDir ${game_directory} --assetsDir ${assets_root} --assetIndex ${assets_index_name} --uuid ${auth_uuid} --accessToken ${auth_access_token} --userType ${user_type} --versionType ${version_type}`
  - Mojang's conditional `rules`, `features` and the jvm array are dropped. Prism never reads Mojang's `arguments`: `MojangVersionFormat::readVersionProperties` at `L/minecraft/MojangVersionFormat.cpp:150` only reads `minecraftArguments`. `--width`, `--height`, quickPlay and `--demo` are added by the launcher (§5).
- `+traits:["XR:Initial","FirstThreadOnMacOS","feature:is_quick_play_singleplayer","feature:is_quick_play_multiplayer"]` and `+jvmArgs:[]`.
- `assetIndex:{id:"19", sha1, size, totalSize, url:"https://piston-meta.mojang.com/v1/packages/<sha1>/19.json"}`.
- `mainJar:{name:"com.mojang:minecraft:1.21.4:client", downloads:{artifact:{sha1,size,url:".../client.jar"}}}`.
- `compatibleJavaMajors:[21]`, `compatibleJavaName:"java-runtime-delta"`.
- `logging:{argument:"-Dlog4j.configurationFile=${path}", file:{id,sha1,size,url}, type:"log4j2-xml"}`.
  - Prism ignores this field. `grep configurationFile` in the whole Prism tree finds nothing, so it never downloads the log config or passes the flag.
- `libraries[]` (57 entries) uses Mojang library objects `{name, downloads:{artifact:{sha1,size,url}}, rules?:[{action,os:{name}}]}`.
  - The new-style natives are separate libraries, e.g. `com.mojang:jtracy-natives-linux:1.0.29` with `rules:[{action:"allow",os:{name:"linux"}}]`, or `io.netty:...:linux-aarch_64`. They are ordinary classpath jars.
- Old versions (1.12.2, 1.7.10) use the legacy form:
  - `downloads.classifiers{natives-linux,...}`, `natives:{linux:"natives-linux",windows:"natives-windows",osx:"natives-osx"}`, `extract:{exclude:["META-INF/"]}`.
  - 1.7.10 args: `... --userProperties ${user_properties} --userType ${user_type}`.
  - 1.6.4 args: `--session ${auth_session} ... --assetsDir ${game_assets}`, with `+traits:["legacyServices"]`.
  - b1.7.3: `+traits:["legacyLaunch","texturepacks","legacyServices"]`, `minecraftArguments:"${auth_player_name} ${auth_session} --gameDir ... --assetsDir ${game_assets}"`, applet launch.

**`net.fabricmc.fabric-loader/0.16.9.json`**
- `order:10`, `mainClass:"net.fabricmc.loader.impl.launch.knot.KnotClient"`, `requires:[{uid:"net.fabricmc.intermediary"}]`.
- `libraries` are Maven-style entries `{name:"org.ow2.asm:asm:9.7.1", url:"https://maven.fabricmc.net/"}`. They have no `downloads` and no sha1, so the URL is `url + gradle-path` (§3).
- It contains asm, asm-analysis, asm-commons, asm-tree, asm-util, sponge-mixin and fabric-loader itself.

**`net.fabricmc.intermediary/1.21.4.json`**
- `order:11`, `libraries:[{name:"net.fabricmc:intermediary:1.21.4", url:"https://maven.fabricmc.net"}]`.
- `requires:[{equals:"1.21.4", uid:"net.minecraft"}]`, `volatile:true`.

**`org.quiltmc.quilt-loader/0.29.2.json`**
- `order:10`, `mainClass:"org.quiltmc.loader.impl.launch.knot.KnotClient"`.
- `requires:[{uid:"net.fabricmc.intermediary"}]`. Quilt now uses the Fabric intermediary, not `org.quiltmc.hashed`.
- 9 libraries with `url`: maven.fabricmc.net, plus `https://maven.quiltmc.org/repository/release/` for quilt-json5, quilt-config and quilt-loader.

**`net.minecraftforge/47.4.28.json`** (MC 1.20.1; `requires:[{equals:"1.20.1",uid:"net.minecraft"}]`)
- `order:5`.
- `mainClass:"io.github.zekerzhayard.forgewrapper.installer.Main"`.
- `libraries` (30), the first being ForgeWrapper:
  - `io.github.zekerzhayard:ForgeWrapper:prism-2026-08-01`, with `downloads.artifact.url` `https://files.prismlauncher.org/maven/io/github/zekerzhayard/ForgeWrapper/prism-2026-08-01/ForgeWrapper-prism-2026-08-01.jar`.
  - Then securejarhandler, asm 9.9.1, accesstransformers, eventbus, forgespi, coremods, modlauncher, mixin 0.8.5, bootstraplauncher, JarJar*, and `net.minecraftforge:fmlloader:1.20.1-47.4.28`.
  - The rest come from `maven.minecraftforge.net`, with sha1 and size.
- `mavenFiles` (37) is the first entry: `net.minecraftforge:forge:1.20.1-47.4.28:installer` → `https://maven.minecraftforge.net/net/minecraftforge/forge/1.20.1-47.4.28/forge-1.20.1-47.4.28-installer.jar`.
  - The other 36 are the installer processors' own dependencies (lzma-java, gson, guava, asm, trove, ...).
  - The `mavenFiles` and `libraries` entries are all downloaded to `libraries/` but are NOT all on the classpath (see §2).
- `minecraftArguments` is the vanilla string plus `--launchTarget forgeclient --fml.forgeVersion 47.4.28 --fml.mcVersion 1.20.1 --fml.forgeGroup net.minecraftforge --fml.mcpVersion 20230612.114412`.

**`net.neoforged/21.4.161.json`** (MC 1.21.4)
- Same structure: ForgeWrapper as `mainClass`, 45 libraries from `maven.neoforged.net` and `libraries.minecraft.net`, 68 `mavenFiles`.
- The installer entry is `net.neoforged:neoforge:21.4.161:installer`.
- The args are the vanilla string plus `--fml.neoForgeVersion 21.4.161 --fml.fmlVersion 6.0.18 --fml.mcVersion 1.21.4 --fml.neoFormVersion 20241203.161809 --launchTarget neoforgeclient`.

**Old Forge (`14.23.5.2860`, MC 1.12.2)**
- `mainClass:"net.minecraft.launchwrapper.Launch"`, `+tweakers:["net.minecraftforge.fml.common.launcher.FMLTweaker"]`. No ForgeWrapper.
- Apply `+tweakers` as `--tweakClass X` args (`L/minecraft/MinecraftInstance.cpp:782`).
- The `legacyFML` trait triggers `L/minecraft/update/LegacyFMLLibrariesTask.cpp` for MC ≤ 1.5.2. The mapping table is in `L/minecraft/VersionFilterData.cpp:20-55`.

**LWJGL**
- `org.lwjgl3/3.3.3.json`: `order:-1`, `conflicts:[{uid:"org.lwjgl"}]`, `volatile:true`, 72 libraries.
- Each native is its own library, e.g. `org.lwjgl:lwjgl-glfw-natives-windows-arm64:3.3.3`, with a `rules:[{action:"allow",os:{name:"<X>"}}]`.
- The `<X>` values used are `windows`, `windows-arm64`, `linux`, `linux-arm64`, `linux-arm32`, `osx`, `osx-arm64`.
  - The artifact `lwjgl-glfw-natives-windows-x86` carries rule `windows`, not `windows-x86`. So on x64 Windows both the `natives-windows` and `natives-windows-x86` jars are active. Do not "fix" this.
  - Jars are served from `libraries.minecraft.net`.
- `org.lwjgl/2.9.4-nightly-20150209` (LWJGL 2) uses classifier natives. Alongside the stock `natives-linux` entries it has `natives-linux-arm32` and `natives-linux-arm64` classifiers, hosted on github.com/theofficialgman.

## 2. Component model

**Instance file** `<instance>/mmc-pack.json` (`L/minecraft/PackProfile.cpp:105-170`, `loadPackProfile` at ~L190)
- `{formatVersion:1, components:[{uid, version, important?, dependencyOnly?, cachedName, cachedVersion, cachedRequires[], cachedVolatile?}]}`.
- Typical Fabric instance, in order: `org.lwjgl3` (dependencyOnly), `net.minecraft` (important), `net.fabricmc.intermediary` (dependencyOnly), `net.fabricmc.fabric-loader`.
- **List order is the apply order.** The version file's `order` field is NOT used for sorting (`Component::getOrder` at `Component.cpp:117` has no caller).
- A local override `<instance>/patches/<uid>.json` replaces the meta file for that uid (`ComponentUpdateTask.cpp:100-130`).

**Resolution** (`L/minecraft/ComponentUpdateTask.cpp:463`, `resolveDependencies`). The user adds `net.minecraft` plus one loader (`PackProfile::setComponentVersion`, L979, which appends to the list). Then, in a loop:
1. Collect the `requires` of every component (`gatherRequirementsFromComponents`, L295).
   - Merge per uid (`composeRequirement`, L265): two different `equals` is a hard error; `suggests` takes the greater version.
2. Remove components where `dependencyOnly && volatile` and nothing requires them any more (L333).
3. For each requirement (`getTrivialComponentChanges`, L357):
   - No `equals`: add the component if it is missing.
   - `equals` and the component is missing: add it at that version.
   - `equals` mismatch and the component is `dependencyOnly`: change its version.
   - `equals` mismatch on a user-chosen component: error "conflicting dependencies".
4. New components are inserted at the index of the first dependee, so before the loader or Minecraft. They are marked `dependencyOnly:true`.
5. **Hardcoded version picking** (L531-547):
   - If `suggests` is set, use it.
   - Else `org.lwjgl` → `2.9.1`, `org.lwjgl3` → `3.1.2`.
   - Else `net.fabricmc.intermediary` or `org.quiltmc.hashed` → the current `net.minecraft` version.
   - So Fabric/Quilt intermediary = MC version is a hack in the launcher. Meta only says `{uid:intermediary}` on the loader.
6. Reload the new components and repeat until nothing changes.
- Loader-version defaults for MC: use the meta `requires.equals` filter for Forge/NeoForge. For Fabric/Quilt take the `recommended` version or the newest `type:"release"`. Prism also has `getRecommendedForParent` / `getLatestForParent` (L610).
- When the user changes the MC version (`UpdateActionImportantChanged`, L643), components required by the old MC version are removed and the linked ones are re-picked. For a loader that no longer matches, it picks the recommended version compatible with the new MC.
- `conflicts` (lwjgl vs lwjgl3, plus `knownConflictingComponents`) only produce warnings (L727).

**Merge into LaunchProfile** (`L/minecraft/VersionFile.cpp:54`, `LaunchProfile.cpp`). Components are applied in list order.

| Field | Merge rule |
|---|---|
| minecraftVersion, versionType, assetIndex | Only from uid `net.minecraft` (L57-64). If no asset index is set, default to id `"legacy"` (LaunchProfile.cpp:295). |
| mainJar, mainClass, appletClass, minecraftArguments | Last non-empty wins. |
| `+jvmArgs` | Appended. |
| `+tweakers` | Appended; an existing duplicate is removed and re-appended (moves later, L107). |
| `+traits` | Set union. |
| `compatibleJavaMajors` | Appended. |
| `compatibleJavaName` | Last non-empty wins. |
| libraries | Skipped if `!isActive(ctx)` (rules). Native-classifier libs go to a separate list. Dedupe by `group:artifact[:classifier]`: the higher `Version` replaces in place, otherwise the first stays (L174-200). |
| mavenFiles | Active ones appended with no dedupe (L202). Native ones are ignored. |
| agents | Appended. |
| jarMods | Appended. |

- Resulting classpath order (`LaunchProfile::getLibraryFiles`, L348): all non-native libs in component order, then the main jar, then native-classifier libs. The main jar comes after loader libs; Knot/modlauncher cope with this.
- Multi-component instances (Forge etc.): Forge's `mainClass` and `minecraftArguments` replace vanilla's, because Forge comes later in the list. Its arguments are the full vanilla string plus the extra args, not a diff.
- `mavenFiles` are downloaded but never put on the classpath. They exist so that ForgeWrapper finds the installer jar and its processor libraries in `libraries/`.

## 3. Libraries (`L/minecraft/Library.cpp`, `Rule.cpp`, `RuntimeContext.h`, `GradleSpecifier.h`)

**Runtime context**
- `system`: `linux`, `osx` (macOS is `osx`!), `windows`, `freebsd`, `openbsd` (`SysInfo.cpp:59`).
- The arch is that of the **Java binary**, not the OS (`JavaRealArchitecture`). It is mapped (RuntimeContext.h:31-42): `amd64`→`x86_64`, `i386`/`i686`→`x86`, `aarch64`→`arm64`, `arm`/`armhf`→`arm32`.
- `classifier = system + "-" + arch`, e.g. `windows-arm64`, `linux-x86_64`.
- `classifierMatches(name)` (L59): true if `name == classifier`, OR (arch is x86_64/x86, "legacy arch") `name == system`.
  - On arm64, a rule named plain `osx` or `windows` does NOT match, only `osx-arm64` and `windows-arm64`. This is why the LWJGL entries above work.

**Rules** (`Rule.cpp:42-91`)
- Only `{action, os:{name}}` is parsed. `os.version`, `os.arch` and `features` are ignored (the stored `version` is not evaluated).
- `apply` returns `Defer` if the os name does not match, otherwise the action.
- `Library::isActive` (L239): with no rules the library is active. Otherwise iterate all rules, the **last non-Defer result wins**, and the starting value is `Disallow`.
  - This means a library whose rules match nothing is excluded.
  - A `[allow, disallow(os=osx)]` pair works correctly.
  - A native-classifier library is additionally inactive if it has no classifier for the current context.

**Paths**
- Storage is `libraries/` + Maven path from the gradle name `group:artifact:version[:classifier][@ext]` → `group/with/slashes/artifact/version/artifact-version[-classifier].ext` (`GradleSpecifier.h:100`).
- Prism ignores `downloads.artifact.path` and always derives the path from `name`. The MMC extension fields `MMC-filename`, `MMC-displayname`, `MMC-hint` (`local` or `always-stale`) and `MMC-absoluteUrl` are parsed at `OneSixVersionFormat.cpp:55`. Neither fixture I curled uses them.
- The main jar is also a library: `com.mojang:minecraft:<ver>:client` → `libraries/com/mojang/minecraft/<ver>/minecraft-<ver>-client.jar`. ForgeWrapper depends on this exact path.

**Legacy classifier natives** (`getCompatibleNative`, L288; `storageSuffix`, L390)
- Look up `natives[classifier]`, then `natives[system]` on legacy arches. Value like `natives-windows` or `natives-windows-${arch}`.
- Replace the library's classifier with it → `...-natives-linux.jar`.
- If the value contains `${arch}`, both 32 and 64 variants are downloaded (L166-182) and the one matching the **Java** bitness goes to the natives list (`getLibraryFiles` L374-378).
- Files are extracted into `<instance>/natives/` (§5). `extract.exclude` is parsed but **not applied** by `ExtractNatives`.

**New-style natives** (`jtracy-natives-linux`, `lwjgl-*-natives-*`) are plain classpath jars selected by `rules`. Nothing is extracted. LWJGL 3 extracts them itself.

**Download URL** (`Library::getDownloads`, L108)
1. If `downloads` exists (Mojang style):
   - For a native, use `classifiers[<chosen classifier>]`; skip it if missing.
   - Otherwise use `artifact.url` with `artifact.sha1`. Verify sha1.
   - A library with no `artifact` and no natives is skipped.
2. Else `MMC-absoluteUrl` if set, otherwise `url`, otherwise `https://libraries.minecraft.net/` (`buildconfig/BuildConfig.h:177`).
   - Join as `url + "/" + path` (add the slash only if it is missing).
   - No checksum is available for these.
- Skip the download if the file is already there with the right hash.
- If `isLocal()` (hint `local`), the file must already exist in `<instance>/libraries/<filename>`, otherwise fail with a message (`LibrariesTask.cpp:42-50`).

## 4. Assets (`L/minecraft/AssetsUtils.cpp`, `update/AssetUpdateTask.cpp`)

1. Take `assetIndex.url` from the profile (a Mojang `piston-meta` URL) and save it as `assets/indexes/<assetIndex.id>.json`, verifying sha1 (`AssetUpdateTask.cpp:20-40`).
2. Parse (`loadAssetsIndexJson`, L82): `{objects:{"<name>":{hash,size}}, virtual?:bool, map_to_resources?:bool}`.
3. For each object, the local path is `assets/objects/<hash[0:2]>/<hash>`. Download it from `https://resources.download.minecraft.net/<hash[0:2]>/<hash>` (L269-297; base at `BuildConfig.h:176`) if it is missing or its size differs. Verify sha1.
4. ReconstructAssets step (`reconstructAssets`, L193, called from `launch/ReconstructAssets.cpp`), only for old indexes:
   - `virtual:true` → copy each object to `assets/virtual/<id>/<name>`.
   - `map_to_resources:true` → copy to `<gameRoot>/resources/<name>`.
   - Existing files are left alone, and leftovers are never deleted.
5. Token values (`MinecraftInstance.cpp:1075`):
   - `${assets_root}` is always the absolute `<data>/assets`.
   - `${assets_index_name}` is the asset id.
   - `${game_assets}` (used in 1.6-era args, L1087) is `assets/virtual/<id>` if the index is virtual, `<gameRoot>/resources` if `map_to_resources`, else `assets/virtual/<id>` as the fallback.
6. All paths are relative to the Prism data dir. Prism's cwd for the launcher is the data dir, which is why `QDir("assets/")` works.

## 5. Launch pipeline

**Steps** (`MinecraftInstance::createLaunchTask`, `L/minecraft/MinecraftInstance.cpp:1156`), in order:
1. Print header.
2. CreateGameFolders: `<gameRoot>` and `server-resource-packs`.
3. Resolve the server address (SRV) if a join target is set.
4. Pre-load command.
5. MinecraftLoadAndCheck: reload meta and resolve components; offline mode uses cached meta.
6. AutoInstallJava, CheckJava, VerifyJavaInstall.
   - Compare `compatibleJavaMajors` with the java major. An empty list means anything goes.
   - Otherwise fail, unless the user ignores the check (then warn). (`VerifyJavaInstall.cpp:57-90`)
   - Meta gives `compatibleJavaName`, and the `com.azul.java` / `net.adoptium.java` / `com.ibm.java` packages are the Java download catalogs.
7. Pre-launch command.
8. Online: ClaimAccount, then the update steps (L1142): FoldersTask → LibrariesTask (libraries + natives + mavenFiles + agents + main jar + jarMods) → LegacyFMLLibrariesTask → AssetUpdateTask. Offline: EnsureOfflineLibraries, which only verifies that all files exist (`EnsureOfflineLibraries.cpp:20-53`).
9. ModMinecraftJar (builds `minecraft.jar` in `<gameRoot>/bin` if there are jarMods; legacy).
10. ScanModFolders, EnsureAvailableMemory, PrintInstanceInfo.
11. ExtractNatives.
12. ReconstructAssets.
13. LauncherPartLaunch.
14. Post-exit command.

**Natives** (`ExtractNatives.cpp:63`)
- Unzip every native jar into `<instance>/natives/`.
- If the Java major is ≥ 8, rename `*.jnilib` → `*.dylib`.
- There is no exclude handling. `finalize()` deletes the folder after the game stops.

**Java command line** (`LauncherPartLaunch::executeTask`, `L/minecraft/launch/LauncherPartLaunch.cpp:71-150`; `javaArguments`, `MinecraftInstance.cpp:600`)
```
<java>
  -Duser.language=en
  <custom JVM args from settings>
  <+jarMods: -Dfml.ignoreInvalidMinecraftCertificates=true -Dfml.ignorePatchDiscrepancies=true>
  <profile +jvmArgs, with ${...} tokens expanded>
  <agents: -javaagent:<jar>[=arg]>
  <-Dorg.lwjgl.{openal,glfw,sdl}.libname=<path> if native lib overrides enabled>
  [macOS: -Xdock:icon / -Xdock:name; -XstartOnFirstThread if trait FirstThreadOnMacOS]
  [Windows: -XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump]
  [Linux + LWJGL2 component + no xrandr on PATH: -DLWJGL_DISABLE_XRANDR=true]
  -Xms<min>m -Xmx<max>m        (swapped if min >= max)
  [Java ≤ 7: -XX:PermSize=<n>m if != 64]
  [Java ≥ 9 with legacyServices+OnlineFixes: --add-opens java.base/java.net=ALL-UNNAMED]
  -Djava.library.path=<instance>/natives
  -cp <NewLaunch.jar>:<libs…>:<main jar>      (';' on Windows)
  org.prismlauncher.EntryPoint
```
- The game JVM starts with cwd = `<gameRoot>` (`<instance>/minecraft`, or `<instance>/.minecraft` if only that exists, `MinecraftInstance.cpp:370`).
- On Windows, convert paths to the 8.3 form (`getPathNameInLocal8bit`) to survive non-ASCII paths.
- Optional wrapper command is prepended before `java`.
- Mojang's `arguments.jvm` is NOT used, and neither are `-Dminecraft.launcher.brand/version` or `-XX:+UseG1GC`.

**Launch script on stdin** (`createLaunchScript`, `MinecraftInstance.cpp:822`; `m_process.write` at `LauncherPartLaunch.cpp:202`)
- Prism writes a line-based script to the JVM's stdin (`mainClass X`, `param <arg>` per game arg, `windowParams 854x480`, `traits T`, `launcher standard|legacy`, ...), then `launch\n` once the game is ready to go (`abort\n` cancels).
- `org.prismlauncher.EntryPoint` (`libraries/launcher/org/prismlauncher/EntryPoint.java`) reads it. `StandardLauncher.launch` (`libraries/launcher/.../impl/StandardLauncher.java`) appends `--width W --height H`.
  - When joining a server it appends `--quickPlayMultiplayer host:port` if the trait `feature:is_quick_play_multiplayer` is set, else `--server H --port P`.
  - When opening a world it appends `--quickPlaySingleplayer <world>` if the trait `feature:is_quick_play_singleplayer` is set.
  - It sets the system properties `org.prismlauncher.launch.mainclass` and `org.prismlauncher.launch.gameargs` (unit-separator `\u001F` joined), then invokes `main` via reflection.
- **For `_k0t1k` you can skip this wrapper for the standard launch.** Run `<mainClass>` directly with `[game args] --width --height [quickPlay…]`. Prism's wrapper only adds the stdin protocol, the window-size args and the two system properties above.
- The legacy launcher (`launcher legacy`, via `NewLaunchLegacy.jar`) is only for the traits `legacyLaunch` or `alphaLaunch`. The applet frame, window title and `appletClass` are Prism-specific. Use it only if you want to support pre-1.6 (b1.x / a1.x) versions; Prism triggers it via `isLegacy()` at `L/BaseInstance.cpp:506`.
- `windowParams` for "maximized" is the screen's available size (non-legacy) or the literal `maximized` (legacy), L857-882.

**Game args** (`processMinecraftArgs`, `MinecraftInstance.cpp:778`)
1. Split `minecraftArguments` on spaces and drop empties.
2. Append `--tweakClass <t>` for each tweaker.
3. Append the server or world target args (see above).
4. Substitute `${key}` in each arg (`replaceTokensIn`, L514). **An unknown token is replaced by the empty string**, not left as is.
5. If the launch mode is demo, append `--demo`.

Token values:
- From the profile (`makeProfileVarMapping`, L1075): `profile_name` = instance name, `version_name` = MC version, `version_type` = the Minecraft `type` (release/snapshot/old_beta/old_alpha), `game_directory` = absolute gameRoot, `game_assets` and `assets_root` / `assets_index_name` as in §4, `library_directory` = `<data>/libraries`.
- From the session (`MinecraftAccount::fillSession`, `MinecraftAccount.cpp:244-260`): `auth_player_name`, `auth_uuid` (id without dashes), `auth_access_token`, `auth_session` (`token:<access>:<uuid>`, or `-` if there is no token), `user_type` (`msa` | `offline`), `user_properties` = `{}`.
- Not defined by Prism: `${clientid}`, `${auth_xuid}`, `${resolution_width}`, ... These only appear in Mojang's `arguments` block, which is already flattened away.

**Offline account** (`MinecraftAccount::createOffline`, L77; `uuidFromUsername`, L282)
- `uuid = md5("OfflinePlayer:" + username)`, then set version 3 (`digest[6] = (digest[6] & 0x0f) | 0x30`) and the IETF variant (`digest[8] = (digest[8] & 0x3f) | 0x80`). This is Java's `UUID.nameUUIDFromBytes`.
- Format it as 32 hex characters with no dashes.
- Session values: `access_token = "0"`, `session = "-"` (`AuthSession.cpp:21-26`), `user_type = "offline"`.

**Process environment** (`createEnvironment`, L700)
- Cleaned environment plus `INST_NAME`, `INST_ID`, `INST_DIR`, `INST_MC_DIR`, `INST_JAVA`, `INST_JAVA_ARGS` and `NO_COLOR=1`.

**Logging** (`launcher/logs/LogParser.cpp`, `MessageLevel.h:32-35`)
- Prism reads stdout/stderr line by line. Lines are either `<log4j:Event logger= timestamp= level= thread=><log4j:Message>…</log4j:Message></log4j:Event>`, which can span several lines, or plain text.
- A line starting `!![LEVEL]!` is a level prefix from Prism's own Java launcher (`MessageLevel::takeFromLine`, `MessageLevel.cpp:24`).
- Level names are matched case-insensitively: TRACE, DEBUG, INFO, MESSAGE, WARN/WARNING, ERROR/CRITICAL, FATAL. A plain-text line has no level (UNKNOWN), and stderr lines are flagged StdErr (`LoggedProcess.cpp:78-84`).
- Because Prism never passes Mojang's `-Dlog4j.configurationFile`, the game normally logs in its default plain format. The XML parser only matters for third-party XML output.
- If you want Mojang's log config (which uses `LegacyXMLLayout` and is also the Log4Shell mitigation for 1.7–1.18), download `logging.file.url` → `assets/log_configs/<id>` and add `logging.argument` with `${path}` replaced. This is an extra feature Prism skips, and the JSON for it is already in the meta.
- Exit code ≠ 0 is treated as a crash (`LauncherPartLaunch.cpp:178-195`).

## 6. How Forge and NeoForge actually launch with this meta

- The classpath is `[NewLaunch.jar] + all libraries (ForgeWrapper, modlauncher, bootstraplauncher, fmlloader, securejarhandler, asm...) + main jar`. `mainClass` is ForgeWrapper's `Main`.
- ForgeWrapper (Prism fork `PrismLauncher/ForgeWrapper`, source in `src/main/java/io/github/zekerzhayard/forgewrapper/installer/{Main,Installer,Bootstrap}.java`) does this on every start:
  1. Read `--fml.mcVersion`, `--fml.forgeVersion`, `--fml.forgeGroup` or `--fml.neoForgeVersion` from the game args. NeoForge is detected by the presence of `--fml.neoForgeVersion`.
  2. Find the libraries dir: `-Dforgewrapper.librariesDir`, or walk up from the `modlauncher`/`FMLLoader` jar path until a directory literally named `libraries` is found (`IFileDetector.getLibraryDir`).
     - Prism sets no such property, so **your libraries dir must be named `libraries`** and contain the loader jars, or you must pass `-Dforgewrapper.librariesDir=<abs path>`.
  3. Find the installer: `<libs>/<group path>/<artifact>/<fullVersion>/<artifact>-<fullVersion>-installer.jar` (e.g. `net/minecraftforge/forge/1.20.1-47.4.28/forge-1.20.1-47.4.28-installer.jar`; `MultiMCFileDetector`). This is the `mavenFiles[0]` entry.
  4. Find the vanilla client jar at `<libs>/com/mojang/minecraft/<mc>/minecraft-<mc>-client.jar`. This is the Prism meta `mainJar`.
  5. Load the installer's `Installer` class, read `install_profile.json` and `version.json`, and set the `data.jvmArgs`.
  6. Run the installer's post-processors if their outputs are missing. These are binpatcher, jarsplitter, MCP/NeoForm merging, access-transformer processors and so on. They produce the patched client jars in `libraries/net/minecraftforge/forge/<ver>/…-client.jar` etc.
     - The first launch is slow and needs Java at launch time, but later launches skip it (checks `checkProcessorFiles` + sha1).
     - The processors' own libraries are the `mavenFiles` that were pre-downloaded.
  7. Add the extra libraries to the classpath, then call the real main class (`cpw.mods.bootstraplauncher.BootstrapLauncher`) with the game args, including `--launchTarget forgeclient|neoforgeclient`.
- I did not verify whether the Mojang mappings or other processor inputs are fetched from the network during step 6 on a clean offline first launch. Plan for network access, or treat a first Forge/NeoForge launch as online-only.
- So Prism meta already gives you everything a Forge/NeoForge install needs. `ForgeWrapper.jar` and the installer JARs must be downloaded like any other library.
- Concurrency risk: if two instances share one libraries dir and both run the processors at once, they race on writing the generated jars. Prism has no guard for this. Use a per-loader-version lock in your launcher.

## 7. Edge cases and gotchas (with refs)
- `${...}` replacement quirk: unknown tokens become empty (`MinecraftInstance.cpp:514-534`).
- Library dedupe is per `group:artifact[:classifier]`, higher `Version` wins, and the first position is kept (`LaunchProfile.cpp:174-200`). It uses `Version` comparison, not string comparison.
- A library with no `rules` and no `downloads`/`url` falls back to `libraries.minecraft.net`.
- The Windows 8.3 path conversion matters only for `-cp` and `-Djava.library.path`.
- `volatile:true` components (lwjgl, intermediary) are auto-removed when nothing requires them (`ComponentUpdateTask.cpp:333`).
- Prism treats "which LWJGL" as `net.minecraft.requires`. Vanilla suggests `org.lwjgl3` ≥ 3.3.x for new versions and `org.lwjgl` 2.9.x for old ones (old vanilla `requires:[{suggests:"2.9.4-nightly-20150209", uid:"org.lwjgl"}]`).
- ARM: Linux arm64/arm32 LWJGL2 natives come from github.com/theofficialgman, not Mojang.
- Windows-on-ARM with an x64 Java picks the x64 natives, because the classifier uses the Java architecture, not the OS.
- Meta `+traits` that matter: `legacyLaunch`, `alphaLaunch` (applet launcher), `legacyServices` (online-fixes), `texturepacks`, `FirstThreadOnMacOS`, `XR:Initial`, `feature:is_quick_play_*`, `legacyFML` (≤1.5.2).
- `requires.suggests` is used only when a dependency is missing or the MC version changes. A user-chosen version is never overridden.
- Offline launch needs all libraries already present (`EnsureOfflineLibraries.cpp`). It does not re-resolve meta online.
- Mojang rule features (`is_demo_user`, `has_custom_resolution`, `has_quick_plays_support`) have no meaning here. The launcher adds `--demo`, `--width/--height` and quickPlay itself.
- Jar-mod instances (pre-1.6 modding): `ModMinecraftJar` builds `<gameRoot>/bin/minecraft.jar` and puts it in place of the main jar (`LaunchProfile.cpp:362-370`). This is only needed for jar-mod support.

## 8. Recommendation: "use Prism meta"
- Vanilla: Prism meta. It is already normalised to `minecraftArguments` + `+traits` + `compatibleJavaMajors`, so you don't parse Mojang's `arguments` rules. It still links the Mojang piston-meta asset index and the Mojang asset/library hosts.
  - If you query Mojang directly instead, you must re-implement the `arguments` rules/features and the old `minecraftArguments` fallbacks. Prism does neither.
  - Mojang's `piston-meta` `version_manifest_v2.json` (HTTP 200, 278 KB) remains a fallback for brand-new snapshots that Prism meta has not yet picked up. The 26.x snapshots appear to be in Prism meta within a few days.
- Fabric and Quilt: Prism meta. It carries the full library lists, `mainClass` and the intermediary requirement. The loader-specific API alternative, `https://meta.fabricmc.net/v2/versions/loader/<mc>/<loader>/profile/json` (HTTP 200 for 1.21.4/0.16.9), returns a Mojang-style profile with `inheritsFrom`, `arguments.jvm:["-DFabricMcEmu= net.minecraft.client.main.Main "]` and per-library md5/sha1/sha256/sha512. Using it means writing an `inheritsFrom` merger plus the Mojang `arguments` handling.
  - Quilt has the same API shape at `https://meta.quiltmc.org/v3/versions/loader/<mc>/<loader>/profile/json` (HTTP 200).
- Forge and NeoForge: **Prism meta strongly recommended**. Using the loaders' own sources instead (`files.minecraftforge.net/.../promotions_slim.json`, the NeoForge `maven-metadata.xml`, installer JARs) means:
  - running the installer headless (Forge's `--installClient` or your own processor engine),
  - handling the many different `install_profile.json` generations,
  - patching the client jars,
  - merging `version.json` `arguments`.
  Prism avoids all of this with ForgeWrapper plus `mavenFiles`. The cost is that you depend on Prism's meta and on the hosted ForgeWrapper jar at `files.prismlauncher.org/maven/`. Cache both locally, because if Prism's meta is down you cannot install new versions.
- Java runtimes: meta also has `com.azul.java`, `com.ibm.java` and `net.adoptium.java` for auto-download. They were not examined further.
- One policy risk: Prism meta data is hosted by the Prism project. If you ship the app, check their terms (`meta.prismlauncher.org` is a public service) and keep a User-Agent and a caching layer. I did not research their usage policy.

**Suggested Rust implementation surface** (the structure follows Prism's flow, with simpler caching than Prism's `HttpMetaCache`):
1. Meta client: fetch `index.json` plus `<uid>/index.json` and `<uid>/<version>.json`, check each against the parent's `sha256`.
2. Component list + resolver as in §2 (the intermediary = MC version rule is a code hack).
3. Merge → `LaunchProfile` (§2 table).
4. Library/rule/native evaluator (§3). Use the Java binary's arch for rule matching.
5. Downloader with sha1 verification (libraries, mavenFiles, main jar, asset index and objects).
6. Natives extractor, asset reconstructor for `virtual` / `map_to_resources`.
7. Command builder (§5): JVM args + `-Djava.library.path` + `-cp` + mainClass + substituted game args + `--width/--height`, with cwd = gameRoot.
8. Offline UUID (§5) and log parsing.
