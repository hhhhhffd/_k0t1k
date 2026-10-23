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
  instance owns its game dir (`mods`, `config`, `saves`...). `resourcepacks` is shared by default through a
  directory junction (Windows) or symlink to `shared/resourcepacks`.
- **Playtime** is recorded per instance (seconds, total and last session) and globally.

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
shared/resourcepacks/    resource packs shared between instances
instances/<id>/          instance.json + minecraft/ (game dir) + natives/
cache/                   downloads cache
```

## Later (ideas already accounted for)

Themes and a lightweight mode for low-end PCs, automatic JVM argument tuning, ely.by skins,
Windows 10 1607 support (WebView2 bootstrapper is embedded), custom clients (Lunar, LabyMod).
