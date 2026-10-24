# _k0t1k architecture

Goal: a Minecraft launcher that matches Prism Launcher's functionality, built on the Modrinth App stack
(Tauri 2 + Rust core + Vue 3/TypeScript/Tailwind). Windows first. Accounts: offline, Microsoft and ely.by.

Reference guides distilled from Prism's source live in `docs/prism/`:
- `launch-pipeline.md`: meta, components, libraries, assets, the launch command line, Forge/NeoForge via ForgeWrapper.
- `java-instances.md`: Java detection/download, JVM settings, instances, playtime, links, launch extras.
- `mod-platforms.md`: Modrinth/CurseForge APIs, murmur2, packwiz metadata, jar parsing, modpack import/export.

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
- **Shared server list**: `servers` is a fifth `SharedFolder` value, but `servers.dat` is a file the game replaces on
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
meta/                    cached Prism meta JSON (meta/<uid>/index.json, meta/<uid>/<version>.json)
libraries/               Maven layout; must be named "libraries" (ForgeWrapper walks up to find it)
assets/                  indexes/, objects/, virtual/
java/<component>/        managed Java runtimes
shared/<folder>/         resourcepacks, shaderpacks, screenshots shared between instances
instances/<id>/          instance.json + minecraft/ (game dir) + sessions/<session>/ (natives of a live launch)
cache/                   downloads cache
```

### First run, install and uninstall (`onboarding.rs`, `core::placement`)

On Windows, when the exe's folder is Desktop, Downloads or Documents (Known Folder API, so OneDrive-redirected
folders count) or inside `%TEMP%` (started from a zip), and no `data-dir.txt` or stored "keep here" choice
(`settings.onboarding.keepPortableAt == exe dir`) exists, the launcher starts on a temporary root
`%TEMP%\_k0t1k-onboarding-<pid>` (`DataDirSource::Onboarding`, WebView2 profile included), shows only the onboarding
screen, has no tray and exits when the window closes. Nothing is written next to the exe. Stale onboarding roots are
swept at the next start.

- `relocate`: copies the exe to the target (`%LOCALAPPDATA%\Programs\_k0t1k`, or `<chosen>\_k0t1k` when the chosen
  folder is not empty) as `.part` + rename, starts it with `--wait-for <pid> --finish-relocation <old exe>
  [--desktop-shortcut] [--start-menu-shortcut]` and exits. The new process waits for the pid *before* the
  single-instance plugin exists (the mutex dies with the old process), moves our known entries
  (`placement::OWNED_NAMES`) from the old folder, deletes the old exe (retries), creates `.lnk` files through
  `IShellLinkW` and records them in `settings.onboarding.createdShortcuts`. Problems are returned once by
  `get_relocation_report` and shown in a dialog. A failure before the spawn leaves the old exe working.
- `keep_portable_here(remember)`: stores the choice (or only passes `--portable-here` for one run) and restarts.
- Uninstall (Settings, Storage): `RemovalPlan` refuses a drive root or any folder that is or contains the profile,
  Desktop, Downloads, Documents or temp; it deletes only the launcher's own entries (so foreign files survive) and the
  shortcuts, through a detached `cmd /c` that waits, deletes twice and removes the then-empty folders.

## Window lifecycle and tray (`src-tauri/src/window.rs`, `tray.rs`)

Goal: while a game runs the launcher costs ~15-30 MB and no CPU. WebView2's processes are the heavy part, so
the main window is destroyed (not hidden) and the Rust process alone keeps running behind a tray icon.

- The tray icon exists for the whole run. Left click opens the window (recreating it); the menu has "Открыть",
  a submenu of running sessions ("<name>", or "<name> #<n>" for later launches and for instances with several
  sessions) with a stop item each (rebuilt on every `instance-state`, no timers) and "Выход".
- `behavior.onGameLaunch`: `free_memory` (default) destroys the window when a session reaches `running`,
  `hide_to_tray` hides it, `keep_open` does nothing. When the last session of all instances ends, a window released this way is
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
- Tuning: `MemorySettings.auto` (default false, global or per-instance override). At launch `core::launch::tuning::tune`
  picks the heap from total RAM, the number of enabled `mods/*.jar` and whether a loader component exists:
  2048 MiB vanilla; 3072 + 40 per mod over 50 with a loader; capped at 8192 and at RAM minus 3072 (never below 1024);
  min heap = half of max (at least 512). G1 flags (Aikar subset valid for Java 8-21+) are added unless the JVM
  arguments already contain a `-XX:+Use...GC`. The choice is written to the session log ("Автоподбор: ...").

## Later (ideas already accounted for)

Themes and a lightweight mode for low-end PCs, ely.by skins,
Windows 10 1607 support (WebView2 bootstrapper is embedded), custom clients (Lunar, LabyMod).
