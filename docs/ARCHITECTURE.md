# _k0t1k architecture

Goal: a Minecraft launcher that matches Prism Launcher's functionality, built on the Modrinth App stack
(Tauri 2 + Rust core + Vue 3/TypeScript/Tailwind). Windows first. Microsoft auth comes later; until then
launches use offline accounts.

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
- **Playtime** is recorded per instance (seconds, total and last session) and globally. Every session records its
  own duration when it ends, so overlapping sessions of one instance are counted separately.
- **Sessions**: every launch is a session with an id that grows during the app run (`launch_instance` returns it).
  The running map is keyed by session id, so an instance may run any number of times at once. Each session has
  its own `instances/<id>/sessions/<session>/natives` and `launch-args.txt` next to it, removed when the session
  ends; leftovers (crash, Windows file locks) are removed by the next launch of the instance. Deleting an
  instance is refused while any of its sessions is live. Preparations run concurrently; `install_lock`
  serialises the installs.

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

## Later (ideas already accounted for)

Themes and a lightweight mode for low-end PCs, automatic JVM argument tuning, ely.by skins,
Windows 10 1607 support (WebView2 bootstrapper is embedded), custom clients (Lunar, LabyMod).
