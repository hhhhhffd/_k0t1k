# Prism Launcher to _k0t1k (Rust/Tauri 2, Windows-first): porting guideline

Paths are relative to `.../scratchpad/ref/prism/launcher/` unless noted. Live endpoints were curled on 2026-10-23, and the JSON shapes below are real. Nothing was built or run from the Prism tree.

## 0. Key takeaways

- **Prism has no Adoptium/Azul API client.** Java downloads come from Prism's own meta server (`https://meta.prismlauncher.org/v1/`, `CMakeLists.txt:205`), package `net.minecraft.java`.
  - On Windows, every entry in that package is a Mojang runtime manifest.
  - Azul and Eclipse entries exist only for Linux x86, arm and riscv.
  - For Windows you can skip meta and read Mojang's own runtime index directly (section 1.5).
- **The required Java major comes from the version JSON.** Prism's meta adds `compatibleJavaMajors` (array of ints) and `compatibleJavaName` (Mojang component name). Mojang's own `version.json` has `javaVersion:{component,majorVersion}`, for example `{'component':'java-runtime-epsilon','majorVersion':25}`.
- **Java probing runs a tiny `JavaCheck.jar`** (`libraries/javacheck`). It prints `os.arch`, `java.version` and `java.vendor`. In Rust you can instead run `java -XshowSettings:properties -version`, or ship the same 15-line jar.
- **Overrides use a gate setting.** Each instance has an `OverrideX` boolean. When it is true the instance value is used, otherwise the global value (section 2).
- **Prism launches the game through its own `NewLaunch.jar`** (`minecraft/launch/LauncherPartLaunch.cpp:95-130`). It writes a launch script to stdin and later sends `launch\n` or `abort\n`. A Rust launcher should build `java <jvmargs> -Djava.library.path=... -cp ... mainClass <mcargs>` itself. Keep only the logic described below.

## 1. Java management

### 1.1 Detection on Windows (`java/JavaUtils.cpp`)

`JavaUtils::FindJavaPaths` (about lines 250-362) builds an ordered candidate list of `javaw.exe` paths. It is probed concurrently and deduplicated.

- **Registry scan**, `FindJavaFromRegistryKey` (lines 180-245).
  - For each root `HKCU` then `HKLM`, and each view (`KEY_WOW64_64KEY` / `KEY_WOW64_32KEY`), it enumerates subkeys of the base key.
  - For each subkey it reads a string value, then uses `<value>\bin\javaw.exe`.
  - Some vendors need a subkey suffix before the value is read.
  - Keys under `SOFTWARE\` (lines 256-312):

| Key | Value | Subkey suffix |
|---|---|---|
| `JavaSoft\Java Runtime Environment` | `JavaHome` | none |
| `JavaSoft\Java Development Kit` | `JavaHome` | none |
| `JavaSoft\JRE` | `JavaHome` | none |
| `JavaSoft\JDK` | `JavaHome` | none |
| `AdoptOpenJDK\JRE` | `Path` | `\hotspot\MSI` |
| `AdoptOpenJDK\JDK` | `Path` | `\hotspot\MSI` |
| `Eclipse Foundation\JDK` | `Path` | `\hotspot\MSI` |
| `Eclipse Adoptium\JRE` | `Path` | `\hotspot\MSI` |
| `Eclipse Adoptium\JDK` | `Path` | `\hotspot\MSI` |
| `Semeru\JRE` | `Path` | `\openj9\MSI` |
| `Semeru\JDK` | `Path` | `\openj9\MSI` |
| `Microsoft\JDK` | `Path` | `\hotspot\MSI` (64-bit view only) |
| `Azul Systems\Zulu` | `InstallationPath` | none |
| `BellSoft\Liberica` | `InstallationPath` | none |

- **Candidate order** (lines 314-350).
  - 64-bit JREs, then fixed `C:/Program Files/Java/jre{8,7,6}/bin/javaw.exe`, then 64-bit JDKs and vendors.
  - Then the 32-bit equivalents, including `C:/Program Files (x86)/Java/jre8..6`.
  - Then bare `javaw` (resolved through PATH).
- **Extra sources appended** (lines 359-362):
  - `getMinecraftJavaBundle()` (line 523) recursively scans for a `bin` directory under `%APPDATA%\.minecraft\runtime` and under `%LOCALAPPDATA%\Packages\Microsoft.4297127D64EC6_8wekyb3d8bbwe\LocalCache\Local\runtime`.
  - `getPrismJavaBundle()` (line 573) checks `<JavaDir>/*/{jre/bin,bin,.}/javaw.exe`.
  - `addJavasFromEnv` (line 164) adds the env var `<LAUNCHER_ENVNAME>_JAVA_PATHS` and every `PATH` entry + `/javaw.exe`.
- **Executable name**: `javaw.exe` on Windows, `java` elsewhere (`JavaUtils::javaExecutable`, line 566). Use `javaw` for the game. For the probe, run `java.exe` with `CREATE_NO_WINDOW` (Prism uses javaw for both).
- **Sorting of the result**: `JavaInstallList.cpp` `sortJavas` sorts by `JavaInstall::operator<` (`java/JavaInstall.cpp`: version, then arch, then path). The list is presented newest first.

### 1.2 Probing (`java/JavaChecker.cpp:46-190`)

Command: `<java> [extra args] -Xms..m -Xmx..m -jar JavaCheck.jar`, 15 s kill timer, with the cleaned environment (below). Stdout is `key=value` lines:

```
os.arch=amd64
java.version=21.0.7
java.vendor=Microsoft
```

- Exit code 1 or a crash gives `Errored`.
- A missing key gives `ReturnedInvalidData`.
- `is64 = os.arch in {x86_64, amd64, aarch64, arm64, riscv64, ppc64le, ppc64}`. `mojangPlatform` is "64" or "32".
- **Clean environment** (`CleanEnviroment`, `JavaUtils.cpp:79-136`). Drop `JAVA_ARGS, CLASSPATH, CONFIGPATH, JAVA_HOME, JRE_HOME, _JAVA_OPTIONS, JAVA_OPTIONS, JAVA_TOOL_OPTIONS`. This is used for both probing and the real launch. Keep it.
- **Version parsing** (`java/JavaVersion.cpp:20-70`).
  - If the string starts with `1.`, use `1[.](major)([.](minor))?(_(security))?(-prerelease)?`.
  - Otherwise use `(major)([.]minor)?([.]security)?(-prerelease)?`.
  - Comparison is by major, minor, security, then prerelease (a release beats a prerelease).
  - `requiresPermGen = !parseable || major < 8` (so PermGen applies to Java 7 and below).
  - `isModular = major >= 9`.
  - `defaultsToUtf8 = major >= 18`. This chooses the stdout decoder (UTF-8 vs system code page) for the game's log.
- **Per-instance cache** (`minecraft/launch/CheckJava` is `launch/steps/CheckJava.cpp:44-118`).
  - It stores the settings `JavaVersion`, `JavaArchitecture` (the "64"/"32" platform string), `JavaRealArchitecture` (the `os.arch` value), `JavaVendor` and `JavaSignature`.
  - `JavaSignature = sha1(mtime_ms_of_exe + resolved_path)`.
  - It re-probes only if the signature changed or any of those values is empty.
  - Fail if the exe is not found (distinct messages for a per-instance path vs the global one).
  - Fail if the probe is `Errored`.
  - On `ReturnedInvalidData`, warn and continue.

### 1.3 Required major version and the compatibility check

- Meta `net.minecraft.<version>.json` sample:
  - 1.21.1: `"compatibleJavaMajors":[21], "compatibleJavaName":"java-runtime-delta"`
  - 1.8.9: `[8]`, `jre-legacy`
- Parsing: `minecraft/MojangVersionFormat.cpp:178-186`. The values are merged across components in `LaunchProfile.cpp:163-171`: majors are appended, and the name is set by the component that provides it.
- If you use Mojang's `version.json` directly, map `javaVersion.majorVersion` to a single-element majors list and `javaVersion.component` to the name.
- Fallback when absent: legacy versions without `javaVersion` mean Java 8 (`jre-legacy`).
- `VerifyJavaInstall` (`minecraft/launch/VerifyJavaInstall.cpp:47-91`):
  - If the compat list is empty or contains the Java's major, succeed.
  - Else, if `IgnoreJavaCompatibility` is set, warn and continue.
  - Else fail and list the acceptable majors.
  - It also logs an error if the Java is 32-bit and `MaxMemAlloc > 2048`.

### 1.4 Selection order at launch (`MinecraftInstance.cpp:1204-1212`, `AutoInstallJava.cpp:62-130`)

Launch steps run in this order: `AutoInstallJava`, then `CheckJava`, then `VerifyJavaInstall`.

`AutoInstallJava`:

1. Skip if the global `AutomaticJavaSwitch` is false, or if the instance has `OverrideJavaLocation=true` and its `JavaPath` file exists.
2. If `AutomaticJavaDownload` is false: refresh the detected list and take the first entry (sorted) whose major is in `compatibleJavaMajors`. Set the instance `OverrideJavaLocation=true`, `JavaPath=<path>`, `AutomaticJava=true`. Warn if it is 32-bit. If none matches, log "Using the default one" and continue with whatever `JavaPath` or bare `java` resolves to.
3. If `AutomaticJavaDownload` is true:
   - If `compatibleJavaName` is empty, warn and use the default.
   - If `<JavaDir>/<compatibleJavaName>/bin/javaw.exe` exists, reuse it (same instance settings as above).
   - Else load `net.minecraft.java` from meta. For each major in `compatibleJavaMajors` in order, load `java<major>.json` and pick the runtime where `runtimeOS == SysInfo::getSupportedJavaArchitecture()` and `name == compatibleJavaName`. Download it (manifest type, or archive type with extraction) into `<JavaDir>/<name>`, then set the instance Java path. On failure, delete the partial directory and try the next major.
4. Defaults: `AutomaticJavaSwitch` and `AutomaticJavaDownload` default to `true` only if the global `JavaPath` is empty at startup (`Application.cpp:765-767`).
5. Quirk worth fixing: Prism writes the auto-chosen path into the instance settings (`OverrideJavaLocation=true`), so a later automatic switch is skipped as long as that file exists. Consider storing the auto-picked path separately and re-resolving on each launch.

`SysInfo::getSupportedJavaArchitecture` (`SysInfo.cpp:97-127`) returns `windows-x64` for x86_64, `windows-x86` for i386, and `windows-<arch>` otherwise (for example `windows-arm64`).

### 1.5 Download sources (live JSON)

1. **Prism meta index**: `GET https://meta.prismlauncher.org/v1/index.json` returns `{formatVersion:1, packages:[{name, sha256, uid}]}`. Relevant uids: `net.minecraft.java`, `com.azul.java`, `com.ibm.java`. Prism's launcher only uses `net.minecraft.java`.
2. **`GET .../net.minecraft.java/index.json`**:

```json
{"formatVersion":1,"name":"Java Runtimes","uid":"net.minecraft.java",
 "versions":[{"recommended":false,"releaseTime":"2025-12-10T14:14:17+00:00","sha256":"7a3b...","version":"java25"},
             {"version":"java21",...},{"version":"java17",...}, ...]}
```

3. **`GET .../net.minecraft.java/java17.json`** has keys `formatVersion,name,releaseTime,runtimes,uid,version`. The Windows x64 entries are:
   - Java 8: `jre-legacy` (8u51-cacert462b08)
   - Java 17: `java-runtime-beta`, `-gamma`, `-gamma-snapshot` (17.0.15)
   - Java 21: `java-runtime-delta` (21.0.7)
   - Java 25: `java-runtime-epsilon` (25.0.1)
   - Each `runtimes[]` item (parser: `java/JavaMetadata.cpp:50-126`):

```json
{"checksum":{"hash":"4be835c1384c4bf1ebcb5c3bfe999ad2b1fe88ac","type":"sha1"},
 "downloadType":"manifest","name":"java-runtime-beta","packageType":"jre",
 "releaseTime":"2025-05-19T08:11:57+00:00","runtimeOS":"linux-x64",
 "url":"https://piston-meta.mojang.com/v1/packages/4be8.../manifest.json","vendor":"mojang",
 "version":{"major":17,"minor":0,"name":"17.0.15","security":15}}
```

   - `windows-arm64` and `windows-x86` exist for Java 17, 21 and 25 but not Java 8 (windows-arm64). The Java 8 entry set includes `windows-x64` and `windows-x86`.
   - An archive-type entry (Linux only) looks like `{"downloadType":"archive","vendor":"azul","url":"https://cdn.azul.com/zulu/bin/zulu17...-linux_i686.zip","checksum":{"type":"sha256",...},"version":{"build":1,...}}`. Vendors seen are `mojang`, `azul` and `eclipse`.
4. **Mojang manifest** (the entry's `url`, validated against `checksum` with sha1 or sha256):
   - Shape: `{"files":{"bin":{"type":"directory"}, "bin/jar":{"type":"file","executable":true,"downloads":{"raw":{"sha1","size","url"},"lzma":{...}}}, "legal/x/y":{"type":"link","target":"../java.base/y"}}}`. The Java 17 manifest has 424 entries.
   - Algorithm (`java/download/ManifestDownloadTask.cpp:68-120`): create the directories, download each `file` via `downloads.raw.url` (sha1 validated, the lzma variant is ignored), and handle `link` entries (symlinks).
   - **Windows caveat**: Prism skips symlink creation on Windows (the link branch is "*nix only"), and the links are only under `legal/`. Do the same, or copy the target file.
   - Concurrency: a NetJob with `NumberOfConcurrentDownloads` (default 6).
5. **Direct Mojang source (simpler for a Windows-first launcher)**:
   `GET https://launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json` is shaped as `{ "<platform>": { "<component>": [ {availability:{group,progress}, manifest:{sha1,size,url}, version:{name:"21.0.7", released}} ] } }`.
   - Platforms: `gamecore, linux, linux-i386, mac-os, mac-os-arm64, windows-arm64, windows-x64, windows-x86`.
   - Windows components: `java-runtime-alpha, -beta, -delta, -epsilon, -gamma, -gamma-snapshot, jre-legacy, minecraft-java-exe`.
   - It is the same data Prism's meta mirrors and needs no Prism meta dependency. Pair it with the version JSON's `javaVersion.component`.
6. **Archive type** (`ArchiveDownloadTask.cpp:36-100`): download into the `cache/java` metacache, take the first path segment of the first zip entry as a prefix to strip, and extract into `<JavaDir>/<name>` (`ExtractZipTask` with that prefix). Only needed for non-Mojang vendors.

### 1.6 Storage

- Managed runtimes: global setting `JavaDir` (default `"java"`, relative to the launcher data dir), at `<JavaDir>/<componentName>/bin/javaw.exe` (`Application.cpp:711`, `javaPath()` at line 2086).
- Archive cache: `cache/java`.
- The Windows `.minecraft\runtime` bundles are only scanned, never written.

## 2. JVM and memory settings

### 2.1 Defaults (global, `Application.cpp:749-768`)

| Setting | Default |
|---|---|
| `MinMemAlloc` | 512 (MiB) |
| `MaxMemAlloc` | `SysInfo::defaultMaxJvmMem()` (see below) |
| `PermGen` | 128 |
| `LowMemWarning` | true |
| `JvmArgs` | "" |
| `JavaPath`, `JavaSignature`, `JavaArchitecture`, `JavaRealArchitecture`, `JavaVersion`, `JavaVendor` | "" |
| `IgnoreJavaCompatibility` | false |
| `LaunchMaximized` | false |
| `MinecraftWinWidth` | 854 |
| `MinecraftWinHeight` | 480 |

`defaultMaxJvmMem` (`SysInfo.cpp:80-86`): if total RAM in MiB is below 6144, use `totalRAM / 1.5`, otherwise 4096. Total and available RAM on Windows come from `GlobalMemoryStatusEx` (`HardwareInfo.cpp:87-106`).

### 2.2 Argument assembly (`MinecraftInstance::javaArguments`, `minecraft/MinecraftInstance.cpp:601-663`)

In order:

1. `-Duser.language=en`
2. User `JvmArgs` split like a command line (`Commandline::splitArgs`), then these additions:
   - If jar mods exist: `-Dfml.ignoreInvalidMinecraftCertificates=true -Dfml.ignorePatchDiscrepancies=true`.
   - Profile `+jvmArgs` with `${...}` tokens substituted.
   - `-javaagent:<jar>[=arg]` for each agent library.
3. **Windows only**: `-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump`. This is the Intel driver hack (MCL-767). Keep it.
4. Memory: if `min < max`, `-Xms<min>m -Xmx<max>m`, else `-Xms<max>m -Xmx<min>m` (swapped if misordered).
5. If `JavaVersion.requiresPermGen()` (Java 7 or lower) and `PermGen != 64`: `-XX:PermSize=<n>m`.
6. If modular Java and the online fixes are enabled: `--add-opens java.base/java.net=ALL-UNNAMED`.
7. Then at launch (`LauncherPartLaunch.cpp:100-113`): `-Djava.library.path=<natives>`, `-cp <classpath joined by ;>`, then the main class.
   - **Windows**: Prism converts the natives path and every classpath item with `FS::getPathNameInLocal8bit`, which is the short 8.3 path conversion. This matters for non-ASCII or very long paths. In Rust, consider `GetShortPathNameW` or use a `-cp @argfile` for long command lines.
- The macOS-only and Linux-only pieces (`-XstartOnFirstThread`, `-Xdock`, `LWJGL_DISABLE_XRANDR`) are not needed.
- There is no automatic tuning of GC or other JVM flags. The only automatic parts are the max-memory default, the min/max swap, and PermGen being version-gated.

### 2.3 Memory sanity (`minecraft/launch/EnsureAvailableMemory.cpp`, non-macOS)

- If `max(Min, Max) * 0.7 > availableRamMiB`, show a Yes/No dialog "Low free memory ... Launch anyway?". It is shown only if `LowMemWarning` is on, and the default button is No.
- If the user declines, fail with "Not enough RAM available to launch this instance". Otherwise log a warning and continue.
- If available RAM reads as 0, skip the check.

### 2.4 The override pattern (`settings/`, `MinecraftInstance.cpp:186-250`, `BaseInstance.cpp:75-135`)

- Instance and global settings are separate INI stores. An instance registers a gate boolean (`OverrideX`) and binds global settings to it.
- **`registerOverride(global, gate)`**: `get()` returns the instance value if `gate` is true, else the global value (`OverrideSetting.cpp:36-42`).
- **`registerPassthrough(global, gate)`**: the same on read. On write it also writes to the global setting (`PassthroughSetting.cpp`). It is used for `JavaSignature`, `JavaArchitecture`, `JavaRealArchitecture`, `JavaVersion` and `JavaVendor`, gated by `OverrideJavaLocation`, plus `ConsoleMaxLines` and `ConsoleOverflowStop`.
- **Gate to settings map**:

| Gate | Settings it controls |
|---|---|
| `OverrideJavaLocation` | `JavaPath`, `IgnoreJavaCompatibility` |
| `OverrideJavaArgs` | `JvmArgs` |
| `OverrideMemory` | `MinMemAlloc`, `MaxMemAlloc`, `PermGen`, `LowMemWarning` |
| `OverrideWindow` | `LaunchMaximized`, `MinecraftWinWidth`, `MinecraftWinHeight` |
| `OverrideCommands` (legacy name `OverrideLaunchCmd`) | `PreLoadCommand`, `PreLaunchCommand`, `WrapperCommand`, `PostExitCommand` |
| `OverrideConsole` | `ShowConsole`, `AutoCloseConsole`, `ShowConsoleOnError`, `LogPrePostOutput` |
| `OverrideGameTime` | `ShowGameTime`, `RecordGameTime` |
| `OverrideMiscellaneous` | `CloseAfterLaunch`, `QuitAfterGameStop` |
| `OverrideEnv` | `Env` |
| `OverrideLegacySettings` | `OnlineFixes` |

  Also: `OverrideNativeWorkarounds` (OpenAL/GLFW/SDL paths) and `OverridePerformance` (gamemode, mangohud, discrete GPU, zink, which are Linux-only).
- **Rust recommendation**: keep an `Option<T>` per overridable group in the instance config, for example `memory: Option<{min,max,permgen}>`. `None` means inherit the global value. This is equivalent to Prism's gate and avoids the "gate true but value stale" state.
- **INI format** (`settings/INIFile.cpp`, Qt `QSettings` IniFormat): Prism sets `ConfigVersion=1.3`, and a `[General]` section is written by `QSettings`. Booleans are `true`/`false`. Values with special characters are quoted or escaped. For the Rust port, use JSON or TOML instead, unless you want import of existing Prism instances.

## 3. Instances

### 3.1 On-disk layout

- Instance root: `<InstanceDir>/<id>/`. The default `InstanceDir` is `"instances"` (`Application.cpp:702`). `AdditionalInstanceDirs` is also supported.
- The id is the directory name. `FS::DirNameFromString` (`FileSystem.cpp:1037-1066`) replaces `\` and `/` with `-` and appends `(n)` on collision, up to 9000.
- An instance is discovered when its directory contains `instance.cfg` (`InstanceList.cpp:470-505`). Symlinked instance dirs are followed, but ignored if they point into any configured instance root.
- Files in the instance root:
  - `instance.cfg`: INI settings.
  - `mmc-pack.json`: the component list (`PackProfile.cpp:274`).
  - `patches/*.json`: custom component overrides.
  - `minecraft/` or `.minecraft/`: the game dir. `gameRoot()` (`MinecraftInstance.cpp:370-378`) uses `.minecraft` only if it exists and `minecraft` does not, otherwise `minecraft`. New instances use `minecraft`.
  - Under the game dir: `mods, coremods, config, saves, resourcepacks, texturepacks, shaderpacks, screenshots, logs, crash-reports, options.txt, servers.dat, bin/` (natives).
  - `icon.png`: refreshed from the icon key before every launch (`MinecraftInstance.cpp:1162`).
  - `<root>/.tmp/<6 hex>/`: staging area for creation, flagged hidden and not-content-indexed on Windows (`InstanceList.cpp:1060-1090`).
- **`instance.cfg` keys** (all registered in `BaseInstance.cpp:75-135` and `MinecraftInstance.cpp:189-277`):

| Group | Keys |
|---|---|
| Identity | `InstanceType=OneSix` (empty is also accepted), `name`, `iconKey` (default "default", which maps to "grass"), `notes`, `uuid`, `linkedInstances` (JSON array string), `shortcuts` |
| Time | `lastLaunchTime`, `totalTimePlayed`, `lastTimePlayed`, `CountGameTime`, `OverrideGameTime` |
| Java | `OverrideJavaLocation`, `OverrideJavaArgs`, `AutomaticJava`, `JavaPath`, `JvmArgs`, `JavaVersion`, `JavaSignature`, `JavaArchitecture`, `JavaRealArchitecture`, `JavaVendor`, `IgnoreJavaCompatibility` |
| Memory | `OverrideMemory`, `MinMemAlloc`, `MaxMemAlloc`, `PermGen`, `LowMemWarning` |
| Window | `OverrideWindow`, `LaunchMaximized`, `MinecraftWinWidth`, `MinecraftWinHeight` |
| Console and commands | `OverrideConsole`, `ShowConsole`, `AutoCloseConsole`, `ShowConsoleOnError`, `LogPrePostOutput`, `OverrideCommands`, `PreLoadCommand`, `PreLaunchCommand`, `WrapperCommand`, `PostExitCommand` |
| Misc | `OverrideMiscellaneous`, `CloseAfterLaunch`, `QuitAfterGameStop`, `OverrideEnv`, `Env` (JSON map as a string) |
| Launch target | `JoinServerOnLaunch`, `JoinServerOnLaunchAddress`, `JoinWorldOnLaunch` |
| Account | `UseAccountForInstance`, `InstanceAccountId` |
| Managed pack | `ManagedPack`, `ManagedPackType`, `ManagedPackID`, `ManagedPackName`, `ManagedPackVersionID`, `ManagedPackVersionName`, `ManagedPackURL` |
| Other | `GlobalDataPacksEnabled`, `GlobalDataPacksPath`, `UseLatestMinecraftVersion`, `UseLatestMinecraftVersionType` |

- **`mmc-pack.json`** (`PackProfile.cpp:104-170`):

```json
{"formatVersion":1,"components":[
 {"uid":"net.minecraft","version":"1.21.1","important":true,
  "cachedName":"Minecraft","cachedVersion":"1.21.1",
  "cachedRequires":[{"uid":"org.lwjgl3","suggests":"3.3.3"}]},
 {"uid":"net.fabricmc.fabric-loader","version":"0.16.0","dependencyOnly":false}]}
```

  - Optional per-component keys: `dependencyOnly`, `important`, `disabled`, `cachedVolatile`.
  - Meta dependencies are the `requires` entries in meta JSON, like `{"suggests":"3.3.3","uid":"org.lwjgl3"}`.
- Creating a vanilla instance (`minecraft/VanillaInstanceCreationTask.cpp:24-42`): create a staging directory, write `mmc-pack.json` with `net.minecraft` (plus a loader component if chosen), set `name` and `iconKey`, download the game files, then commit.
- A simpler Rust approach: store your own `instance.json` and skip the component graph, unless loaders such as Fabric, Forge or Quilt are in scope.

### 3.2 Create, rename, delete (`InstanceList.cpp`)

- **Create or commit** (`commitStagedInstance`, lines 1093-1150):
  - Work in `.tmp/<key>`, then `FS::move` it to `<dir>/<DirNameFromString(name)>`.
  - Record the group and save `instgroups.json`.
  - Staging means an interrupted creation leaves no half instance (deleted on fail or abort, lines 1030-1037).
- **Rename**: only `name` in `instance.cfg` changes. The directory is optionally renamed (`InstanceDirUpdate.cpp`).
  - Setting `InstRenamingMode`: `AskEverytime` (default) or `MetadataOnly`.
  - New dir name is `DirNameFromString`. Refuse if it exists or the instance is running, falling back to a metadata-only rename.
- **Delete** (`deleteInstance`, line 428): remove the group index entry, save groups, recursively delete the root, and delete any shortcuts.
- **Trash** (`trashInstance`, `undoTrashInstance`, lines 333-420): the group is remembered so the delete can be undone.

### 3.3 Groups: `instgroups.json` (in the launcher data dir root, not the instances dir)

Written by `saveGroupList` (line 750), read by `loadGroupList` (line 800). It also reads a legacy copy in the instances dir.

```json
{"formatVersion":"1",
 "groups":{"My Group":{"hidden":false,"instances":["inst-id-1","inst-id-2"]}},
 "ungrouped":{"hidden":true}}
```

- `hidden` means the group is collapsed in the UI. `ungrouped.hidden` is written only when true. The empty string stands for the ungrouped bucket.
- The `formatVersion` is written as a string, but the loader compares an int (`rootObj.value("formatVersion").toVariant().toInt()`). Accept both.
- Groups are never saved before the instance scan has completed (`m_instancesProbed` guard, line 745).
- A group is dropped when its member count reaches zero.

### 3.4 Copying (`InstanceCopyTask.cpp`, `InstanceCopyPrefs.cpp`)

Options (`InstanceCopyPrefs.h:44-56`):

| Option | Meaning | Default |
|---|---|---|
| `copySaves`, `keepPlaytime`, `copyGameOptions`, `copyResourcePacks`, `copyShaderPacks`, `copyServers`, `copyMods`, `copyScreenshots` | what to copy | all true |
| `useSymLinks`, `linkRecursively`, `useHardLinks`, `dontLinkSaves`, `useClone` | how to copy | all false |

- The filters are a case-insensitive regex against paths. Each selected-off item prepends `[.]?minecraft/` (`InstanceCopyPrefs.cpp:10-60`):
  - `saves`
  - `options.txt`
  - `resourcepacks` and `texturepacks`
  - `shaderpacks`
  - `servers.dat`, `servers.dat_old` and `server-resource-packs`
  - `coremods`, `mods` and `config`
  - `screenshots`
- After copying (`copyFinished`, lines 123-175): set the new `name` and `iconKey`, call `regenerateUuid`, and reset the playtime if `!keepPlaytime`.
- If symlinks were used: call `addLinkedInstanceId(orig.id())` and append the original `gameRoot` to `<gameRoot>/allowed_symlinks.txt`. Minecraft 1.20+ refuses to follow symlinks from world dirs unless the target is listed there.
- Copy modes:
  - Plain copy (default).
  - Link mode (see section 5).
  - Reflink or clone (`FS::clone`) when `useClone` is set. The Windows path uses `FSCTL_DUPLICATE_EXTENTS_TO_FILE`, which needs ReFS or a Dev Drive (`FileSystem.cpp:1611-1700`).
- In link mode `instance.cfg` is always excluded from the linked set so each instance keeps its own settings.
- **Keep** plain copy, the filters and the `allowed_symlinks.txt` handling. **Skip** clone and the privileged-run machinery at first.

### 3.5 Icons and notes

- `iconKey` selects a built-in icon (the default "default" maps to "grass") or a custom file in the icons dir (`IconsDir`, default `"icons"`), matched by file base name. Unknown keys fall back to "grass" (`icons/IconList.cpp:440-470`).
- The icon is exported to `<gameRoot>/icon.png` before each launch. The launch script passes `instanceIconPath icon.png`.
- `notes` is a plain string in `instance.cfg`. Newlines and special characters are escaped by the INI layer (the format uses `\n`, `\t`, `\#`).

## 4. Playtime tracking

- Per instance (`BaseInstance.cpp:295-350`), driven by `setMinecraftRunning(bool)`:
  - It returns immediately if `RecordGameTime` is off.
  - On true, store `m_timeStarted = now` and set `lastLaunchTime` (epoch milliseconds).
  - On false, `secondsPlayed = secsTo(now)`. Then `totalTimePlayed += secondsPlayed` and `lastTimePlayed = secondsPlayed`. If `CountGameTime`, also add to the global `TotalPlayTime`.
  - `totalTimePlayed` and `lastTimePlayed` are stored in seconds. A negative stored total is reset to 0 on load (line 81).
- **When it triggers**: `proceed()` in `LauncherPartLaunch.cpp:196` calls `setMinecraftRunning(true)`, which is when the game is actually released to start (`launch\n`). `setMinecraftRunning(false)` runs on `Crashed`, `Aborted` or `Finished`.
- **Live display**: while the game runs, `totalTimePlayed()` and `lastTimePlayed()` return the stored value plus `now - m_timeStarted`.
- **Global total**: stored in a separate settings file via `playtimeSettings()`, with keys `TotalPlayTime` (seconds, default 0) and `TotalPlayTimeMigrated` (`Application.cpp:944-945`). It is shown in the status bar if `ShowGlobalGameTime`.
- **Format** (`MMCTime.cpp:30-40`): `"%1h %2min"` for under a day and `"%1d %2h %3min"` otherwise. `ShowGameTimeWithoutDays` (default false) shows the hour total without splitting days.
- Per-instance toggles: `ShowGameTime` and `RecordGameTime` (override group `OverrideGameTime`), plus `CountGameTime` (default true). `resetTimePlayed` resets the two values.

## 5. Shared content (links)

- **Prism has no "share a folder between instances" feature.** The only link usage is at copy time (`InstanceCopyTask` link mode), plus the optional per-instance global data-packs path (`GlobalDataPacksEnabled`, `GlobalDataPacksPath`). Everything is a one-shot copy-time operation, with no sync or live sharing.
- Link mode (`InstanceCopyTask.cpp:53-100`, `FileSystem.cpp:529-653`, `create_link`):
  - With `useSymLinks` only: the instance root's top-level entries are symlinked (`maxDepth=0`). Directories use `create_directory_symlink`, files use `create_symlink`.
  - With `linkRecursively`: every file is linked individually, with the directory structure recreated.
  - With `useHardLinks` (implies recursive): `create_hard_link` per file. This does not work across volumes.
  - `dontLinkSaves` copies `saves/` into the new instance instead of linking it. This avoids two instances writing to the same world.
- **Windows specifics**:
  - It uses `std::filesystem` links, which are real symlinks, not junctions. Creating them requires `SeCreateSymbolicLinkPrivilege` (admin or Developer Mode).
  - On failure it re-runs the work elevated: `create_link::runPrivilegedAndWait` (`FileSystem.cpp:756-810`) launches `filelink.exe` via `ShellExecuteEx` with verb `runas`. `filelink.exe.manifest` declares `requireAdministrator`. Hard links need no privilege, so that mode skips the retry.
  - The helper is in `filelink/` (`FileLink.cpp`, `filelink_main.cpp`).
- **Recommendation for _k0t1k**:
  - For Windows directory sharing, prefer a **junction** (`mklink /J`), which needs no privilege and works only for directories on the same machine.
  - Use hard links for files on the same volume, and fall back to a copy.
  - Do not port the UAC helper. Try a symlink and fall back to a junction or copy.
  - Keep the `allowed_symlinks.txt` append when you create symlinks into `saves`, since Minecraft 1.20+ requires it.

## 6. Launch-time extras

### 6.1 Step order (`MinecraftInstance.cpp:1156-1286`)

1. Print a header with the game dir.
2. Create the game folders (`CreateGameFolders`).
3. Resolve a join-server target (`LookupServerAddress`) when the port is 25565.
4. `PreLoadCommand` (before meta loads).
5. Load and check meta (`MinecraftLoadAndCheck`).
6. `AutoInstallJava`, `CheckJava`, `VerifyJavaInstall`.
7. `PreLaunchCommand`.
8. Online: `ClaimAccount`, then update tasks. Offline: `EnsureOfflineLibraries`.
9. `ModMinecraftJar`, `ScanModFolders`, `EnsureAvailableMemory`, `PrintInstanceInfo`.
10. `ExtractNatives`, `ReconstructAssets`.
11. `LauncherPartLaunch`, then the `PostExitCommand` step, then `QuitAfterGameStop`.

- Any failing step stops the chain (`LaunchTask::onStepFinished`, `launch/LaunchTask.cpp:90-116`). Completed steps are finalized in reverse order.

### 6.2 Commands (`launch/steps/LaunchCommand.cpp:45-110`, `BaseInstance.cpp:110-115`)

- Pre-Load, Pre-Launch and Post-Exit are run as processes in `gameRoot`. The command string goes through `substituteVariables`, a shell-like split with `$VAR` expansion from the instance environment.
- A non-zero exit code fails the launch. Output is logged when `LogPrePostOutput`.
- Pre-Load runs before meta loads, Pre-Launch after Java checks, and Post-Exit after the game process ends.
- The Post-Exit step runs only if the game step succeeded (a crash fails the chain before it).
- **Wrapper command** (`LauncherPartLaunch.cpp:115-135`):
  - Substitute variables, take the first token and resolve it on PATH (`findExecutable`). Fail with "The wrapper command ... couldn't be found" if missing.
  - Run `wrapper wrapperArgs... <java> <args...>`.

### 6.3 Environment (`MinecraftInstance.cpp:680-725`)

- Start from `CleanEnviroment()`, then add:
  - `INST_NAME`, `INST_ID`, `INST_DIR`, `INST_MC_DIR`, `INST_JAVA`, `INST_JAVA_ARGS`
  - `NO_COLOR=1`
- Custom `Env` is a JSON map (string to string) from the global setting, or from the instance when `OverrideEnv` is on.
- The same variables are used for `$VAR` expansion in command strings.
- `INST_EXITCODE` is not set (a FIXME in `LauncherPartLaunch.cpp:178`).

### 6.4 Window and close behavior

- Window size: `LaunchMaximized` is ignored by modern clients, so Prism computes the screen's available size (minus the decorations of the Prism main window) and passes `WxH`. For legacy versions it passes `maximized`. Otherwise it uses `MinecraftWinWidth` x `MinecraftWinHeight` (default 854x480) (`MinecraftInstance.cpp:856-885`).
- `CloseAfterLaunch` (`LauncherPartLaunch.cpp:51-68`): wait for a log line matching `.*Setting user.+` (case-insensitive), then close all launcher windows, and show the main window again when the game exits. `QuitAfterGameStop` appends a step that quits the launcher.
- `ShowConsole` opens the console window at launch. `ShowConsoleOnError` (default true) opens it on failure (`LaunchController.cpp:475`). `AutoCloseConsole` closes it after a successful run. `ConsoleMaxLines` (default 100000) and `ConsoleOverflowStop` (true) are global only.

### 6.5 Logs and crash handling

- `LoggedProcess` (`LoggedProcess.cpp`):
  - Wraps the process. Reads stdout and stderr separately and splits into lines, keeping the partial last line until the next chunk and stripping `\r`.
  - Decodes as UTF-8 for Java 18+ (`defaultsToUtf8`), else the system code page.
  - State machine: `Starting`, `Running`, `Finished`, `Crashed`, `Aborted`, `FailedToStart`.
  - A normal exit is "Process exited with code N". A crash exit is "Process crashed ...". If killed by the user the state is `Aborted` and the log says "Process was killed by user."
- **Exit code handling** (`LauncherPartLaunch.cpp:160-185`): a non-zero exit code is treated as a crash ("Game crashed.") and the launch fails.
- **Log levels** (`MessageLevel.h`): `Unknown, Launcher, Trace, Debug, Info, Message, Warning, Error, Fatal` (with `WARN` and `CRITICAL` as aliases). Game stdout and stderr are `StdOut` and `StdErr`.
  - Lines prefixed `!![LEVEL]!` carry a level from the launch jar (`takeFromLine`).
  - `LogParser` parses `log4j2` XML events from the game's log into entries `{level, timestamp, thread, message}`. If you configure `-Dlog4j.configurationFile` with Mojang's `logging.file` from the version JSON, you can use that stream.
  - `AnonymizeLog` and the censor filter hide the access token and similar secrets in the console (`LaunchTask::censorPrivateInfo`).
- **Crash report locations scanned**: `crash-reports`, `logs`, and the game root (`MinecraftInstance.cpp:1100`).
- **Kill** (`LauncherPartLaunch.cpp:208-221`):
  - If the process is still waiting for the `launch` command, send `abort\n` over stdin.
  - Else call `kill()` (a hard `TerminateProcess` on Windows) when it is `Running` or `Starting`. There is no graceful close.
  - The process is created detachable, so it keeps running if the launcher object is destroyed.
- **Rust notes**:
  - Read stdout and stderr on separate threads.
  - Use `CREATE_NO_WINDOW` for helper processes.
  - Keep the "is the game running" flag per instance, and set it on process spawn rather than on step success.

## 7. Themes and UI (brief inspiration)

- Built-in widget themes (`ui/themes/ThemeManager.cpp:143-160`):
  - `system` (the platform default), `dark`, `bright`, and every Qt style (for example `fusion`, `windows`, `windows11`).
  - Ids: `dark` and `bright` from `DarkTheme::id()` and `BrightTheme::id()`; system styles use the style name.
  - Icon themes and a "background cat" feature are separate settings (`IconTheme`, `ApplicationTheme`, `BackgroundCat`, `EnableCat`, `CatOpacity`, `CatFit`).
- Custom theme = a directory `themes/<id>/` with `theme.json`, an optional QSS file, and a `resources/` dir. A bare `.qss` file in the themes dir also works.
  - `theme.json` keys (`ui/themes/CustomTheme.cpp:185-262`):
    - `name` (required) and `widgets` (required: the base Qt style name).
    - `qssFilePath` (default `themeStyle.css`).
    - `colors`: `Window, WindowText, Base, AlternateBase, ToolTipBase, ToolTipText, Text, Button, ButtonText, BrightText, Link, Highlight, HighlightedText`, plus `fadeColor` and `fadeAmount` (default 0.5, used for disabled states).
    - `logColors`: foreground `Message, Launcher, Debug, Warning, Error, Fatal`; backgrounds `MessageHighlight, LauncherHighlight, DebugHighlight, WarningHighlight, ErrorHighlight, FatalHighlight`.
- **For a Tauri UI**: a theme is just a JSON of CSS custom properties. Use the Prism keys above as the variable set, and include the per-level log colors. Ship dark and light, and follow the system theme.

## 8. Recommended implementation surface (what to keep or simplify)

**Keep:**
- The Windows registry scan list, with HKCU+HKLM and both WOW64 views, plus the `.minecraft\runtime` and managed-runtime directory scans.
- The probe (`os.arch`, `java.version`, `java.vendor`) plus the sha1 signature cache.
- The `JavaVersion` parse and compare rules.
- The compat check (`compatibleJavaMajors`, `IgnoreJavaCompatibility`).
- The auto-select, then auto-download, then default order.
- The Mojang runtime manifest download with sha1 validation and atomic cleanup on failure.
- The JVM argument order, the memory swap, the Intel heap-dump hack, and the PermGen gate.
- The clean environment.
- The staged instance creation (`.tmp/<key>` then move), `DirNameFromString`, and `instgroups.json`.
- The playtime semantics, and `allowed_symlinks.txt`.

**Simplify or skip:**
- The Prism meta layer. Use Mojang's `all.json` and `version_manifest_v2.json` directly.
- The component graph, unless loaders are in scope.
- Archive-type runtimes (Linux only).
- `NewLaunch.jar` and its stdin protocol.
- macOS, Linux, gamemode and mangohud code.
- `filelink.exe` elevation, clone, and the `NewLaunchLegacy` path.
- INI via `QSettings` (use JSON or TOML for new data).

**Risks:**
- The auto-chosen Java path is persisted into the instance settings (section 1.4).
- Windows short-path handling for the classpath (section 2.2).
- Symlink privilege, so prefer junctions and hard links (section 5).
- Prism's meta server is a third-party dependency, and `compatibleJavaName` may be empty for old versions.
- `JavaCheck` needs a working Java, so a missing or broken Java must still produce a clear error.

**Uncertainty:** I did not trace `LaunchController` crash dialogs beyond the `ShowConsoleOnError` check, nor the full `FS::copy` and `FS::clone` implementations. I did not check how Prism handles `windows-arm64` auto-download beyond the platform string above.
