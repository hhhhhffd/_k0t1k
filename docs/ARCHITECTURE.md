# _k0t1k architecture

Goal: a Minecraft launcher that matches Prism Launcher's functionality, built on the Modrinth App stack
(Tauri 2 + Rust core + Vue 3/TypeScript/Tailwind). Windows first. Accounts: offline, Microsoft and ely.by.

Working notes distilled from Prism's source (meta and launch pipeline, Java and instances, Modrinth/CurseForge)
are kept locally in `docs/prism/`, which is git-ignored.

## Layout

```
packages/core          k0t1k-core: all launcher logic, no UI dependency, testable on Linux/WSL
apps/app/src-tauri     k0t1k-app: Tauri shell, thin commands that call k0t1k-core and forward progress events
apps/app/src           Vue 3 UI
```

## Decisions

- **Game and loader metadata: Prism meta** (`https://meta.prismlauncher.org/v1/`). It is pre-normalised
  (flat `minecraftArguments`, `compatibleJavaMajors`) and launches modern Forge/NeoForge through ForgeWrapper,
  so we never run loader installers ourselves. Responses are cached under `meta/` and validated against the
  parent index's `sha256`; cached data is used when offline.
- **Java runtimes: Mojang's runtime index** (`all.json`), picked by the version's `compatibleJavaName` /
  `compatibleJavaMajors`, installed into `java/<component>`. Like Prism, a system Java is used only when automatic download is off.
- **Storage formats are JSON** (`settings.json`, `instance.json`), not Prism's INI. Mod origin metadata uses
  packwiz `.index/*.pw.toml` files, the same as Prism, so instances stay compatible with packwiz tools.
- **Per-instance settings are `Option<T>` groups**: `None` inherits the global value (Prism's `OverrideX` gates).
- **Shared folders**: game metadata, libraries, assets and Java runtimes are shared by every instance. Each
  instance owns its game dir (`mods`, `config`, `saves`...). `resourcepacks`, `shaderpacks` and `screenshots`
  (`instance::SharedFolder`) can be linked to `shared/<folder>` through a directory junction (Windows) or
  symlink. An instance stores the set as `sharedFolders` in `instance.json`; `sharedFoldersDefault` in the
  global settings (default: resourcepacks + shaderpacks) is applied to new and imported instances. Enabling
  moves the instance's existing files into the shared folder (clashes become `name (1).ext`, nothing is
  overwritten); disabling leaves a real folder with a copy. Deleting or copying an instance never touches or
  duplicates shared content, and modpack installs never overwrite existing files in a linked folder. Old files
  with `shareResourcepacks` are converted on read (true = only resourcepacks). Named pools (different shared
  sets for groups of instances) would go in as a pool id in the shared path (`shared/<pool>/<folder>`), with
  the current list being the implicit default pool, so no migration would be needed.
- **Shared server list**: `servers` is the fourth `SharedFolder` value, but `servers.dat` is a file the game replaces on
  save (temp file + rename), which breaks hardlinks and replaces file symlinks, so it is synchronised, not linked.
  `shared/servers.dat` is the master. Enabling merges the instance's entries into it (entries whose address, case-
  insensitive, is missing are appended in order; unknown NBT tags survive), then copies it into the game dir;
  disabling leaves the instance's copy. Before each launch `launch.rs` copies shared to the game dir and remembers
  the bytes; after the game exits `push_shared` stores the game dir's file in shared if it differs from that
  snapshot. If the shared file changed meanwhile (another session exited first), the exiting session's list wins
  and shared-only entries are appended: nothing is lost, but a deletion made by the earlier session can come back.
  Same-instance sessions overlap the same way (last exit wins). The launcher's server commands edit
  `shared/servers.dat` for sharing instances (`servers::edit_list`) and mirror it into the game dir; listing reads it.
- **Playtime** is recorded per instance (seconds, total and last session) and globally. Every session records its
  own duration when it ends, so overlapping sessions of one instance are counted separately.
- **Sessions**: every launch is a session with an id that grows during the app run (`launch_instance` returns it).
  The running map is keyed by session id, so an instance may run any number of times at once. Each session has
  its own `instances/<id>/sessions/<session>/natives` and `launch-args.txt` next to it, removed when the session
  ends; leftovers (crash, Windows file locks) are removed by the next launch of the instance. Deleting an
  instance is refused while any of its sessions is live. Preparations run concurrently; `install_lock`
  serialises the installs.

## Game clients (`core/src/client`)

A client (LabyMod 4 today; Lunar and Feather are meant to follow) is an alternative to the mod loaders layered on
vanilla Minecraft. It is an instance component with a launcher-owned uid (`net.labymod.labymod4`, version = release
channel), not a Prism meta package, and it excludes the loaders and other clients (`client::check_exclusive`, run by
`resolve_components`, and the picker in the UI).

- **Extension point**: a provider resolves to a `VersionFile` (main class, classpath libraries, JVM and game arguments,
  required Java) plus `ClientFiles` (files that go into the game dir instead of `libraries/`, and an "installed" mark).
  A new client adds a `ClientKind` variant, a uid in `CLIENT_UIDS` and a branch in `client::load`; the profile build,
  install and launch code do not change. `client::apply` strips the launch parts of `net.minecraft` (libraries,
  main class, arguments, `requires`, Java) so only the client's own apply, while `net.minecraft` still gives the client
  jar, asset index and traits. `LaunchProfile::client_files` carries the files; `client::install_files` /
  `check_files` run after `install_game` / `check_installed` (create, version change and every launch).
- **LabyMod provider** (`client/labymod.rs`): the channel manifest (`latest.json`) names the build (`commitReference`)
  and the supported Minecraft versions; each version has a Mojang-format JSON. Its main class
  (`net.minecraft.launchwrapper.Launch`), plain game arguments (ending in `--dummyMinecraftClass`), plain JVM arguments
  (`-Dnet.labymod.running-version`, ...) and Java are used as they are; the arguments with rules are covered by the
  launcher (window, quick play, OS flags), and `-cp` / `java.library.path` are built as for any game. LabyMod's library
  list (everything, as LabyMod's launcher and XMCL do; natives of other systems are filtered by a rule derived from the
  `natives-<os>` name) and the jar (`net.labymod:LabyMod:<build>-<commit>`, sha1 from the manifest, the manifest size is
  not exact) are added to the classpath. The commit is part of the jar version because the build number can repeat
  between commits and a file on disk is trusted by its path. The asset jars go to `<game dir>/labymod-neo/assets/<name>.jar`
  and are re-downloaded when their sha1 differs. LWJGL libraries LabyMod loads on its own (`isolated_libraries`) are
  dropped from the version JSON; LabyMod downloads them itself into `labymod-neo/libraries` at start.
- **Updates**: online resolving re-reads `latest.json` when the cached copy is older than 10 minutes; a new commit
  fetches its version JSON and library list (cached per commit) and the changed files. When offline (or when the meta
  resolve falls back to the cache), the commit recorded in `meta/labymod/<channel>/installed-<mc>.txt` is used; that file
  is written by `install_files` only after every file of the build is on disk.
- **Terms**: the first use asks the user to accept LabyMod's terms (fetched from `dl.labymod.net`); the acceptance is
  `acceptedClientTerms` in `settings.json` (kept by `save_settings`), and create / change-versions refuse a client
  without it. ely.by (authlib-injector) with LabyMod is not verified.
- **UI**: the loader picker has a "Client" radio next to the loaders (disabled with a tooltip for versions missing from
  the manifest); the Mods tab of a LabyMod instance explains that LabyMod uses its own mods folder.

## Accounts (`core/src/auth`)

- `settings.json` holds only the public list `{id, name, kind}` (`offline` | `microsoft` | `elyby`); ids are the offline
  UUID, `ms-<uuid>` and `elyby-<uuid>`, so kinds cannot collide. Tokens live in `accounts.json` (`auth::Secrets`, own
  lock, never sent to the UI; on Windows encrypted with DPAPI for the current user plus app entropy, `auth/protect.rs`, so
  nothing lands in the system store and a portable folder moved to another user/PC asks to sign in again; a plain file
  from an older version is encrypted at startup; both names are in `placement::OWNED_NAMES`). Saving an account that exists updates it;
  `auth::remove_account` drops its tokens.
- **Microsoft**: OAuth device code flow (client id baked in by build.rs from `K0T1K_MSA_CLIENT_ID`, env or `.env`, like the
  CurseForge key), then Xbox Live, XSTS and Minecraft services. `start_microsoft_login` returns the code at once,
  `finish_microsoft_login` polls until it is entered, `cancel_microsoft_login` stops it (the dialog closing). The
  Minecraft token is refreshed from the stored refresh token (which Microsoft rotates) when it expires within 5 minutes;
  a refresh lock keeps concurrent launches from racing. User type `msa`.
- **ely.by**: Yggdrasil at authserver.ely.by (password never stored; two-factor accounts are asked for the code and sent
  as `password:code`). Before a launch the token is validated and refreshed. The game gets authlib-injector as
  `-javaagent:<jar>=ely.by` (`LaunchOptions::java_agent_args`, placed first); the jar is downloaded from the
  authlib-injector artifact index, sha256-checked and cached in `libraries/moe/yushi/authlib-injector/<version>`, the
  newest cached one is used offline. User type `mojang`.
- `launch_instance` resolves the account inside the session's preparation (`auth::resolve_launch_identity`), so refresh
  errors fail the launch with a readable message in the session log.

## Worlds, data packs, screenshots, servers

- `level.dat` / `servers.dat` are read and written by a small NBT codec (`instance/nbt.rs`, with `flate2` for gzip)
  instead of an NBT crate: files must be written back with every unknown tag preserved in order, and strings need
  Java's modified UTF-8. Servers are edited one entry at a time (add/update/remove/move), so the favicon cache
  and other tags the game stored survive.
- Commands that rewrite saves or `servers.dat` fail while the instance is running (`worlds::ensure_stopped`).
  World deletion is permanent (the UI asks for confirmation); backups are zips of the world folder in
  `<instance>/backups/` or a chosen path. World import rejects archives with `..`/absolute/drive-letter entries.
- A data pack belongs to a world: `saves/<world>/datapacks`. The world folder is passed as the `game_dir` of the
  regular resource scan/toggle/delete and of `content::install_version`, so the `.disabled` and packwiz-index
  conventions are the same as for mods.
- Screenshots are shown through Tauri's asset protocol (`convertFileSrc`). The scope is empty in config and the
  backend allows only the listed instance's `screenshots/` folder at runtime (the data dir is not known statically).
- Custom instance icons (`instance/icon.rs`): `instance.json` keeps `icon: "custom:<file>"` (any other value is a built-in
  key, so old files stay valid) and the image is `instances/<id>/icon-<millis>.<ext>` (PNG/JPEG/GIF/WebP by magic bytes,
  at most 5 MB). The name changes with every icon, so no cache busting is needed, and `InstanceManager::update` deletes
  the previous custom file whenever the icon value changes. Copy and our own export zip carry the file (import stores
  it as a new icon); Prism zips and mrpack/CurseForge imports keep the default icon. The UI loads it through the asset
  protocol; `get_instances_dir` allows the instances folder (recursively) at startup, and the icon component joins
  `<instances dir>/<id>/<file>` itself.
- Discord Rich Presence (`src-tauri/src/discord.rs`, crate `discord-rich-presence`): `K0T1K_DISCORD_CLIENT_ID` is
  embedded by `build.rs` like the CurseForge key (env or repo-root `.env`, warning when missing; without it the
  feature is inert). While any session is Running the activity shows details = instance name, state =
  "<mc version> · <loader>", start = session start, large image key `logo` (upload an asset named `logo` under
  Rich Presence in the Discord developer portal); the most recently started session wins, and with none running
  the activity is cleared. One dedicated thread owns the IPC client and is fed through a channel; connect/send
  failures are only logged at debug level and the next state change reconnects. Setting: `discord.enabled`
  (default true), read when a game starts.

- Server status is a Server List Ping over TCP in `instance/servers.rs` (5 s timeout, no SRV lookup).

## Data directory (portable)

Resolved once at startup by the app (`src-tauri/src/datadir.rs`): the folder in `<exe dir>/data-dir.txt` if
present, otherwise the executable's own folder (Prism-portable style) when it is writable, otherwise
`%APPDATA%/_k0t1k`. WebView2's profile lives in `<root>/webview`, so nothing is written elsewhere.

```
settings.json            global settings
accounts.json            account tokens (DPAPI-encrypted on Windows)
meta/                    cached Prism meta JSON (meta/<uid>/index.json, meta/<uid>/<version>.json)
libraries/               Maven layout; must be named "libraries" (ForgeWrapper walks up to find it)
assets/                  indexes/, objects/, virtual/
java/<component>/        managed Java runtimes
shared/<folder>/         resourcepacks, shaderpacks, screenshots shared between instances
shared/servers.dat       master copy of the shared server list
instances/<id>/          instance.json + minecraft/ (game dir) + sessions/<session>/ (natives and launch-args.txt
                         of a live launch)
cache/                   downloads cache
webview/                 WebView2 profile
logs/                    launcher.log for this run, launcher.1.log ... launcher.5.log for earlier runs
icons/                   user pictures offered as instance icons
catpacks/, iconsets/,    user cat pictures, toolbar icon sets and theme .json files (created on demand)
themes/
```

`data-dir.txt` lives next to the exe, not in the data folder.

### First run, install and uninstall (`onboarding.rs`, `core::placement`)

On Windows, when the exe's folder is Desktop, Downloads or Documents (Known Folder API, so OneDrive-redirected
folders count) or inside `%TEMP%` (started from a zip), and no `data-dir.txt` or stored "keep here" choice
(`settings.onboarding.keepPortableAt == exe dir`) exists, the launcher starts on a temporary root
`%TEMP%\_k0t1k-onboarding-<pid>` (`DataDirSource::Onboarding`, WebView2 profile included), shows only the onboarding
screen, has no tray and exits when the window closes. Nothing is written next to the exe. Stale onboarding roots are
swept at the next start.

- `relocate`: copies the exe to the target (`%LOCALAPPDATA%\Programs\_k0t1k`, or `<chosen>\_k0t1k` when the chosen
  folder is not empty) as `.part` + rename, starts it with `--wait-for <pid> --finish-relocation <old exe>
  [--desktop-shortcut]` and exits. The new process waits for the pid *before* the
  single-instance plugin exists (the mutex dies with the old process), moves our known entries
  (`placement::OWNED_NAMES`) from the old folder, deletes the old exe (retries), creates the desktop `.lnk` through
  `IShellLinkW` and records them in `settings.onboarding.createdShortcuts`. Problems are returned once by
  `get_relocation_report` and shown in a dialog. A failure before the spawn leaves the old exe working.
- `keep_portable_here(remember)`: stores the choice (or only passes `--portable-here` for one run) and restarts.
- Uninstall (Settings, Storage): `RemovalPlan` refuses a drive root or any folder that is or contains the profile,
  Desktop, Downloads, Documents or temp; it deletes only the launcher's own entries (so foreign files survive) and the
  shortcuts, through a detached `cmd /c` that waits, deletes twice and removes the then-empty folders.

## Window lifecycle and tray (`src-tauri/src/window.rs`, `tray.rs`)

Goal: while a game runs the launcher costs ~15-30 MB and no CPU. WebView2's processes are the heavy part, so
the main window is destroyed (not hidden) and the Rust process alone keeps running behind a tray icon.

- Linux has no tray (GNOME shows none without an extension; `window::HAS_TRAY`): the window stays open while a game
  runs, closing it exits, and the two settings below are hidden there.
- The tray icon exists for the whole run. Left click opens the window (recreating it); the menu has "Открыть _k0t1k",
  a "Запущенные игры" submenu with an "Остановить: <label>" item per running session (label "<name>", or
  "<name> #<n>" for later launches and for instances with several sessions; rebuilt on every `instance-state`,
  no timers) and "Выход".
- `behavior.onGameLaunch`: `hide_to_tray` (default) destroys the window when a session reaches `running`, leaving
  only the tray icon; `keep_open` does nothing. `free_memory` in older settings files is read as `hide_to_tray`. When the last session of all instances ends, a window released this way is
  recreated/shown. Opening the window yourself clears the mark, so a window you closed afterwards stays closed.
- `behavior.onClose`: `tray` (default) destroys the window on close, `exit` quits. With games running, exit
  (and the tray's "Выход") asks in a native dialog: Да = kill all sessions (waits until playtime is saved), Нет = leave
  them running, Отмена = stay. Closing the last window never exits (`RunEvent::ExitRequested` with no code is
  prevented); only `AppHandle::exit` does.
- A recreated window restores state from the backend: `list_run_states`, `list_instances` and
  `get_game_logs(sessionId)` (ring buffer of the last 5000 lines per session; the logs of the last 3 finished
  sessions per instance are kept until the app exits, and `list_run_states` lists them as `stopped`).
  Log lines carry a global `seq` so lines delivered both live and by the backfill are not duplicated.
  While no window exists the log batcher keeps recording but does not emit.
- A second launch of the exe shows the running launcher (`tauri-plugin-single-instance`: a named mutex and a
  hidden window on Windows, nothing on disk).

## Crash reports, join server, automatic tuning

- Crash: when the game exits on its own with a code other than 0 (or by a signal), `run_session` looks for the newest
  `crash-reports/crash-*.txt` or `hs_err_pid*.log` in the game dir modified since the spawn
  (`core::launch::crash`). The result (`CrashInfo`: exit code, report path) is stored with the session log,
  emitted as `game-crashed` after `instance-state: stopped` and also returned by `list_run_states` (`crash`) until
  the UI calls `dismiss_crash`, so a window recreated after the crash still shows the dialog.
- Log upload (`core::mclogs`, commands `upload_game_log`, `open_crash_report`): only on a user click. Content =
  session log lines plus the crash report after a separator; `--accessToken <x>` and the session's account token are
  masked; oldest lines are dropped over mclo.gs's 25000 lines / 10 MiB. POST JSON `{content, source}` to
  `https://api.mclo.gs/1/log`.
- Join server: `Instance.joinServer` (instance.json, set through `update_instance_settings`) or the `server` argument
  of `launch_instance` (the "Играть" button of the servers tab; wins). `LaunchOptions.join_server` becomes
  `--quickPlayMultiplayer host:port` when the profile has the `feature:is_quick_play_multiplayer` trait (1.20+),
  else `--server host --port port`; the port defaults to 25565.
- Tuning: `MemorySettings.auto` (default false, global or per-instance override) follows Prism Launcher: at launch
  `core::launch::tuning::tune` gives 4096 MiB, or two thirds of the RAM below 6 GiB (`default_max_memory_mib`, Prism's
  `defaultMaxJvmMem`; Mojang's launcher also defaults to 4 GiB), with a 512 MiB minimum, and adds no GC flags (Java 17+
  uses G1 by default). The choice is written to the session log. A CurseForge pack's `recommendedRam` is applied at
  import as an instance memory override, capped at 90% of the RAM (`cap_recommended_ram`, as in Prism).

## Appearance

- Themes: built-in ones plus custom themes (`ui.customThemes`, edited in Settings, or `themes/*.json`); lite mode
  (`ui.liteMode`) turns off blur, shadows and animations. Cat pictures (`ui.catPack`) and toolbar icon sets
  (`ui.iconSet`) come from `catpacks/` and `iconsets/`.
- Built-in instance icons are SVGs in the repo-root `icons/instances/`; `index.json` there gives their keys (Prism's
  icon keys, so imported Prism instances keep their icon) and the picker order. Unknown keys show `grass`.

## Language

- English (default) and Russian, chosen in Settings and stored as `ui.language` (`en` | `ru`; missing means `en`).
  No i18n library. UI: `i18n/en.ts` is the source dictionary, `ru.ts` is typed against it (a missing key is a type
  error), and `i18n/index.ts` exports `t`, a reactive object whose sections are replaced on a switch. `App.vue`
  remounts its tree on a change, so values computed once in `setup` follow too. The choice is also cached in
  localStorage for the first paint.
- Backend: `k0t1k_core::i18n` holds the language in an atomic (set at startup and in `save_settings`) with
  `tr(en, ru)` and `tr_fmt!(en, ru, args)`. Used for everything the user sees (errors, task titles, launch log lines,
  tray menu, dialogs); the tray is rebuilt on a change. Tracing and internal errors stay English.

## Updates (`core::update`, `src-tauri/src/update.rs`)

- Self-update from GitHub Releases  not `tauri-plugin-updater` (it expects an installer; the launcher is a portable exe).
  No GitHub REST API (unauthenticated it is 60 requests an hour per IP, which testers behind one address exhaust):
  `GET https://github.com/hhhhhffd/_k0t1k/releases/latest` is sent with redirects disabled (`update::latest_version`, a
  separate client) and answers 302 with `Location: .../releases/tag/vX.Y.Z`. Only that exact shape is accepted (host, repo,
  `v` + canonical `X.Y.Z`); the tag is compared with the app version as plain numbers (no pre-release tags). A 404 or a
  redirect to `.../releases` (private repository, no release) is `unavailable`, not an error; any other location is
  refused. Asset URLs are built from the version: `.../releases/download/vX.Y.Z/<asset>`. The Settings UI shows a
  "Release notes" button that opens `.../releases/tag/vX.Y.Z` externally.
  The result of a check is kept for the run, so a recreated window shows the notice without asking again; a failed
  check is logged by the backend and shown only for the "Check now" button.
- **Transient problems** (`update::is_transient`: HTTP 403/429/5xx, no answer, timeout) are not errors: the check answers
  `unavailable` with `delayed: true` (not remembered, so the next check asks again) and Settings shows a calm warn-colored
  "GitHub is limiting requests right now" line. Other failures reach the UI as a short localized text (`tr`) and the
  technical error goes to `launcher.log`. There is no throttle on the launcher's check (once per run, and on request).
- Assets: `_k0t1k.exe` and `_k0t1k.exe.sig` (Windows), `_k0t1k-x86_64.AppImage` and `_k0t1k-x86_64.AppImage.sig`
  (Linux), picked at compile time. The signature is the base64 of a raw Ed25519 signature over
  `"_k0t1k-update\n<version>\n<hex sha256 of the exe>"` (`core::update::signed_message`), so a signed exe cannot be
  republished under a newer tag. The client rebuilds the message from the tag, hashes the download while streaming and
  verifies with the embedded public key (`ed25519-dalek` `verify_strict`); a file that fails is deleted, never installed.
  The release must be newer than the running version. Limits: asset URLs must start with
  `https://github.com/hhhhhffd/_k0t1k/releases/download/` (checked again in `download`; URLs are not sent to the UI), the
  exe is at most 200 MiB and, when the response has `Content-Length`, the download must match it exactly (the signature
  is the integrity check, there is no API `size` any more), the signature file at most 1 KiB; update requests use their
  own client with connect and read timeouts. Downloads follow redirects (GitHub serves assets from
  `release-assets.githubusercontent.com`) but only over https and at most 5 of them. `scripts/release.sh [--appimage <AppImage>] <exe> [notes]` checks that the exe (and the binary
  extracted from the AppImage) contains `k0t1k-launcher/<version>`, that the tree is clean and HEAD is pushed, signs with `K0T1K_UPDATE_KEY` (default
  `~/.config/k0t1k/update-signing-key.pem`, never in the repo), refuses when the tag exists or the signature does not
  verify against the embedded key, and runs `gh release create ... --target <HEAD>`.
- Install (`install_update(version)`): installs exactly the release the user saw (refused when the latest differs). Refused
  in debug builds, off Windows and Linux, on Linux when not started from an AppImage (`$APPIMAGE` names the file to
  replace; `current_exe` is inside its mount), or when the folder is not writable (`installBlocked` carries the reason), and while a
  game runs, a game or Java install runs, or any `tracked` task is active (re-checked after the download, after raising
  the `QUITTING` flag that `AppState::reserve` and `spawn_game_install` check under the locks the updater takes, so nothing
  starts between the last check and exit). The download goes to `<exe>.new` (synced to disk), then the running exe is renamed
  to `<exe>.old`, `.new` takes its place (any failure renames `.old` back; if even that fails `.new` is kept and the error
  names `.old` for manual recovery) and the new exe is started with `--wait-for <pid>`, the same mechanism as onboarding,
  so it waits for this process to exit before the single-instance mutex is created (on Linux the wait polls
  `/proc/<pid>/stat`; the new AppImage is made executable before the swap). The next start deletes `.old` and
  `.new`, logging failures. Progress is the `update` task of the `progress` event.
- Manual recovery: if the new exe starts but is broken, it is not rolled back automatically; until the next successful
  start `<exe>.old` is the previous version, so deleting the new exe and renaming `.old` back restores it.
- Setting `updates.checkOnStartup` (default true) only controls the silent startup check, which shows a notice in the
  status bar; Settings, About has the Updates block.

## Plugins (`core::plugins`, `src-tauri/src/plugins.rs`, `src/plugins`)

- Two wasm plugins from their own public repositories: **RatScan** (`ratscan`, `hhhhhffd/ratka_checker`: scans mod jars for
  stealers and RATs) and **packconv** (`packconv`, `hhhhhffd/conventor-mc-rp`: converts and repairs resource packs between
  Minecraft versions). Both are wasm-bindgen `--target web` builds with the export contract of plugin `api` 1 (`scan`;
  `convert`, `repair`, `supportedVersions`). Breaking the contract means a new `api`; the launcher refuses an unknown one.
- **Sandbox**: a plugin only runs in a dedicated module Web Worker (`src/plugins/worker.ts`, created lazily by
  `host.ts`, one per plugin, kept alive). The main thread reads the glue and wasm and sends them over; the worker imports
  the glue from a blob URL and initialises it with the wasm bytes. A worker has no Tauri IPC and no page access, so a plugin
  sees only the bytes of the call. A call that runs longer than 120 s terminates the worker (the next call starts a new one).
  The CSP gained `script-src blob: 'wasm-unsafe-eval'` and `worker-src 'self' blob:`. This is what the blob import needs if the
  worker inherits the page policy; it also lets the main page run blob scripts, which nothing in the app creates. Not verified
  inside WebView2.
- **Bundled baseline**: `apps/app/public/plugins/<id>/` (glue, wasm, `plugin.json` without a signature), part of the frontend
  assets and so of the signed exe. `plugins/SOURCES.md` records the repository commits they were built from. The Rust side
  embeds the manifests with `include_str!`.
- **Release format** (per plugin repo): tag `vX.Y.Z`; assets `plugin.json`, `plugin.json.sig` and each file listed in it.
  `plugin.json` is `{id, version, api, minLauncher, files: {name: sha256}}`. The signature is the base64 of a raw Ed25519
  signature over `"_k0t1k-plugin\n" + the exact bytes of plugin.json` (`plugins::manifest_message`), checked with a plugin
  key embedded in `core::plugins`; it is not the self-update key. Asset URLs are built as
  `https://github.com/<repo>/releases/download/<tag>/<name>` for the plugin's own repo (file names in the manifest are
  plain names, so they cannot leave that folder).
- **Update** (`plugins::update`, reuses `update::http_client`, `parse_version`, `verify_with`, `fetch_capped`): the latest tag is read from the
  redirect of `github.com/<repo>/releases/latest` exactly as for the launcher (no REST API); an update is accepted only when the manifest signature is valid, `id` matches, `api` is 1, `minLauncher` is not above the
  launcher version, the tag equals `version`, `version` is newer than the running copy, and every file (at most 32 MiB each)
  hashes to its manifest entry. It is installed into `<data>/plugins/<id>/<version>/` (staging folder, then rename), then the
  `active` pointer file is replaced (temp file and rename). The previous version is kept, older ones are deleted. Any failure is
  logged with its technical detail and leaves `active` as it was. In `plugins/state.json` a rejected release or a bad download
  is stored as a short localized `error`; a transient problem (`update::is_transient`: 403/429/5xx, network, timeout) is stored
  as `delayed: true` instead and Settings shows a calm warn-colored "GitHub is limiting requests right now" notice.
- **Which copy runs** (`plugins::resolve`): the installed active version when its stored manifest still verifies, all files still
  hash right (checked at every load) and it is not older than the bundled baseline; otherwise the bundled copy (a damaged installed
  copy is logged and shown as the plugin's error). Installed files reach the UI as binary IPC (`read_plugin_file`), bundled ones
  are fetched from the app assets.
- **Checks**: part of the launcher's update check (`check_for_update`): on every start when `updates.checkOnStartup` is on, and
  on "Check now" in Settings, About, Updates. Newer plugins are installed right away; the launcher itself only on confirmation.
  `last_check`, the last error and `delayed` are kept in `state.json`; Settings, About, Plugins shows them with versions and source.
- **RatScan in the Mods tab**: "Scan mods" scans enabled and disabled jars; the backend reads each jar and sends it as raw bytes
  (`read_resource_file`), the UI shows progress. Badges map the verdict: `clean`, `suspicious`, `malicious` is shown as
  "dangerous", `unknown` as "not analyzed" (never as clean). A click opens the findings. Results are cached in
  `<instance>/ratscan.json` by SHA-1 and plugin version (`core::scancache`, at most 1000 entries). Jars added by a drop, an
  import, an install or update from Modrinth/CurseForge and the blocked-files scan are scanned in the background. Before launch
  (`instances.launch`) an enabled dangerous mod asks "Launch anyway / Disable the mod and launch / Cancel"; only cached results
  count, so a jar that was never scanned does not stop the game.
- **packconv in the Resource packs tab**: the backend reads `pack.mcmeta` of zip packs (`core::packformat`, ranges as in
  packconv: `pack_format`, `supported_formats`, `min_format`/`max_format`) and compares them with the format of the instance's
  release (a table copied from packconv, since the wasm does not export it; snapshots have no format, so nothing is claimed).
  A mismatch shows "made for X". "Convert to V" uses the newest supported version not above the instance version and saves
  `<name> (V).zip`; "Repair" saves `<name> (repaired).zip`. Output goes through `save_resource_file` (raw body, arguments in an
  `x-args` header) and never replaces a file (` (2)` is inserted when the name is taken); the original stays. A mismatched pack
  that is dropped or installed gets a toast with a convert button.
- The browser mock runs the same bundled wasm in the same worker; only file access is in memory (`api/mockPlugins.ts`).

## Later

ely.by skins, more clients (Lunar, Feather) through the client extension point.
