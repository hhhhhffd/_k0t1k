/*
 * Frontend <-> backend CONTRACT. Every function below is one Tauri command (snake_case name).
 * Argument object keys are camelCase (Tauri converts them to the snake_case Rust parameters).
 * Errors: the command returns Err(String); the promise rejects with that string.
 *
 * Instances
 *   list_instances()                                         -> InstanceListing  (instances + unreadable ones)
 *   create_instance(request: CreateInstanceRequest)          -> Instance      (emits `progress` while downloading)
 *   rename_instance(instanceId, name)                        -> Instance
 *   delete_instance(instanceId)                              -> void
 *   copy_instance(instanceId, name)                          -> Instance
 *   set_instance_group(instanceId, group: string | null)     -> Instance
 *   update_instance_settings(instanceId, settings: InstanceSettingsUpdate) -> Instance
 *   change_instance_versions(instanceId, minecraftVersion, loader: {id, version} | null, client?: ClientId) -> Instance (refused while running; downloads the new game files in the background; a client excludes the loader)
 *   get_instances_dir()                                      -> string  (allows the asset protocol for it; custom icons are files in it)
 *   set_instance_icon(instanceId, path)                      -> Instance  (image file, png/jpeg/gif/webp up to 5 MB; icon becomes `custom:<file>`)
 *   reset_instance_icon(instanceId)                          -> Instance  (back to a built-in icon, removes the file)
 *   open_instance_folder(instanceId)                         -> void
 * Launch
 *   launch_instance(instanceId, accountId?, server?)         -> SessionId     (always starts a NEW session, also when the instance already
 *                                                                              runs; state via `instance-state`, output via `game-log`;
 *                                                                              `server` = host[:port] to join, else the instance's joinServer)
 *   kill_session(sessionId)                                  -> void          (stops the game or cancels its preparation; errors when the session is gone)
 *   kill_instance(instanceId)                                -> void          (= kill_session for every live session; errors when none)
 *   list_run_states()                                        -> RunSession[]  (live sessions + up to 3 finished ones per instance, `state: 'stopped'`,
 *                                                                              whose log is kept; for a window opened mid-game)
 *   get_game_logs(sessionId)                                 -> GameLogLine[]  (ring buffer of one session, up to MAX_LOG_LINES)
 *   upload_game_log(sessionId)                               -> UploadedLog   (POSTs the redacted log, plus the crash report if any, to mclo.gs; only on a user click)
 *   open_crash_report(sessionId)                             -> void          (errors when the session left no report)
 *   dismiss_crash(sessionId)                                 -> void          (list_run_states stops reporting the crash)
 *
 * Sessions: ids are numbers that grow during the app run and are never reused. Each session has its own natives
 * directory and log, records its own playtime when it ends (overlapping sessions of one instance are counted
 * separately, so the total may exceed wall-clock time), and counts as a running game for the tray and for
 * `behavior.onGameLaunch`. delete_instance errors while any session of the instance is live.
 * Versions
 *   list_minecraft_versions()                                -> MinecraftVersion[]
 *   list_loader_versions(minecraftVersion, loader: LoaderId) -> LoaderVersion[]
 *   list_client_versions(client: ClientId)                   -> string[]       (Minecraft versions the client supports; errors without network and cache)
 *   get_client_terms(client: ClientId)                       -> ClientTerms    (fetches the text; `accepted` = already accepted in settings)
 *   accept_client_terms(client: ClientId)                    -> void           (create_instance / change_instance_versions refuse a client until this was called)
 * Java
 *   detect_java()                                            -> JavaInstall[]  (system installs; rescans)
 *   list_managed_java()                                      -> JavaInstall[]
 * Settings / system
 *   get_settings()                                           -> GlobalSettings
 *   save_settings(settings: GlobalSettings)                  -> void
 *   get_system_info()                                        -> SystemInfo
 *   open_data_dir()                                          -> void
 *   open_launcher_folder(folder: LauncherFolder)            -> void  (creates the folder if missing)
 *   list_icon_library()                                      -> string[]  (images in icons/, for set_instance_icon; allows the asset protocol)
 *   list_cat_packs()                                         -> CatPack[]  (catpacks/; allows the asset protocol)
 *   list_icon_sets()                                         -> IconSet[]  (iconsets/<set>/<key>.png|svg|gif|webp; allows the asset protocol)
 *   list_theme_files()                                       -> ThemeFiles  (themes/*.json, ids `file:<name>`)
 *   open_shared_folder(folder: SharedFolder)                 -> void  (creates shared/<folder> if missing)
 *   set_data_dir(path: string | null)                        -> boolean  (true = restart needed; data is not moved; null = default location)
 * Onboarding / install (Windows; the launcher asks where to live when its exe sits in Desktop, Downloads or Documents,
 *   or in the temp folder, and starts on a temporary data root until the user decides)
 *   get_onboarding()                                         -> OnboardingInfo | null  (null = no decision pending; show the normal UI)
 *   relocate(target, desktopShortcut, startMenuShortcut)     -> void  (copies the exe to `target`, or `target\_k0t1k` when the folder is
 *                                                                      not empty, restarts from there and exits; errors leave the old exe working)
 *   keep_portable_here(remember)                             -> void  (restarts in place; `remember` stores the choice in settings.onboarding)
 *   get_relocation_report()                                  -> RelocationReport | null  (once, after a restart caused by `relocate`)
 *   create_shortcuts(desktop, startMenu)                     -> string[]  (created .lnk paths; recorded for uninstall)
 *   get_uninstall_plan()                                     -> UninstallPlan  (errors when uninstalling is refused: user folder, running games)
 *   uninstall()                                              -> void  (a detached cmd deletes the launcher after exit)
 * Accounts (settings.accounts mirrors the result; tokens live in accounts.json and never reach the UI)
 *   add_offline_account(name)                                -> Account
 *   remove_account(accountId)                                -> void  (any kind; also deletes the stored tokens)
 *   select_account(accountId)                                -> void
 *   start_microsoft_login()                                  -> DeviceCodeInfo  (errors in builds without K0T1K_MSA_CLIENT_ID)
 *   finish_microsoft_login()                                 -> Account  (waits until the code is entered; errors when cancelled,
 *                                                                          declined or expired; re-adding an account updates it)
 *   cancel_microsoft_login()                                 -> void
 *   elyby_login(username, password, totp: string | null)     -> ElybyLoginResult  (retry with the code when twoFactorRequired)
 * Content
 *   search_content(query: SearchQuery)                       -> SearchPage
 *   get_project_versions(platform, projectId, filter: VersionFilter) -> ProjectVersion[]
 *   install_version(instanceId, platform, projectId, versionId, kind: LocalResourceKind) -> InstallReport
 *   list_local_resources(instanceId, kind: LocalResourceKind) -> LocalResource[]
 *   toggle_resource(instanceId, kind, fileName, enabled)     -> void
 *   delete_resource(instanceId, kind, fileName)              -> void
 *   check_updates(instanceId, kind: LocalResourceKind)       -> AvailableUpdate[]
 *   import_modpack_file(path, name?)                         -> ModpackImport (.mrpack / CurseForge .zip; emits `progress`)
 *   install_modpack_version(platform, projectId, versionId, name?) -> ModpackImport (emits `progress`)
 * Worlds (the modifying commands fail while the instance is running)
 *   list_worlds(instanceId)                                  -> World[]  (folders with a level.dat, last played first)
 *   rename_world(instanceId, dirName, name, renameFolder)    -> string   (the folder name afterwards)
 *   copy_world(instanceId, dirName, name)                    -> string   (folder name of the copy)
 *   delete_world(instanceId, dirName)                        -> void     (permanent)
 *   backup_world(instanceId, dirName, destination?)          -> string   (zip path; default <instance>/backups/)
 *   open_world_folder(instanceId, dirName)                   -> void
 *   import_world(instanceId, path)                           -> string   (.zip; folder name it got)
 *   list_datapacks(instanceId, world)                        -> LocalResource[]  (world = dirName)
 *   toggle_datapack(instanceId, world, fileName, enabled)    -> void
 *   delete_datapack(instanceId, world, fileName)             -> void
 *   add_datapack(instanceId, world, path)                    -> string   (.zip file copied into the world)
 *   install_datapack(instanceId, world, platform, projectId, versionId) -> InstallReport (emits `progress`)
 * Screenshots
 *   list_screenshots(instanceId)                             -> Screenshot[]  (newest first; allows the asset protocol for the folder)
 *   open_screenshot(instanceId, fileName)                    -> void
 *   delete_screenshot(instanceId, fileName)                  -> void
 *   open_screenshots_folder(instanceId)                      -> void
 * Servers (servers.dat; the modifying commands fail while the instance is running)
 *   list_servers(instanceId)                                 -> Server[]
 *   add_server(instanceId, server: Server)                   -> void
 *   update_server(instanceId, index, server: Server)         -> void
 *   remove_server(instanceId, index)                         -> void
 *   move_server(instanceId, from, to)                        -> void
 *   ping_server(address)                                     -> ServerStatus  (host or host:port; rejects on timeout/refusal)
 *   update_modpack(instanceId, versionId)                    -> ModpackImport (in-place update of a managed pack; backs up and rolls back; emits `progress`)
 * Transfer
 *   list_export_entries(instanceId, dir)                     -> ExportEntry[]  (dir relative to the game folder, '' = root)
 *   export_instance(instanceId, request: ExportRequest)      -> PackExportReport | null (null for an instance zip; emits `progress`)
 *   add_local_files_to_instance(instanceId, kind: LocalResourceKind | null, paths) -> AddFilesReport (null kind = identify each file)
 *   scan_blocked_files(instanceId, files: BlockedFile[], folder: string | null) -> BlockedScan (copies matches by SHA-1; null folder = Downloads)
 *   get_downloads_dir()                                      -> string
 *   set_update_lock(instanceId, kind, fileName, locked)      -> void
 * Launcher update (GitHub Releases, Ed25519-signed exe)
 *   check_for_update(force)                                  -> UpdateStatus   (also installs newer plugins; without force, the result of the earlier check of this run; failures are logged by the backend)
 *   install_update(version)                                  -> void           (installs exactly that release (refused when the latest differs): downloads, verifies, replaces the exe and restarts;
 *                                                                              emits `progress` as task `update`; refused while a game runs or anything is downloading)
 * Plugins (wasm plugins run in a Web Worker of the UI; see ARCHITECTURE.md)
 *   list_plugins()                                           -> PluginInfo[]  (version and source of the copy that runs)
 *   read_plugin_file(id, version, name)                      -> ArrayBuffer   (a file of the installed copy; bundled copies are read from the UI assets)
 *   ratscan_status(instanceId)                               -> JarScan[]     (all mod jars, enabled and disabled, with cached results)
 *   ratscan_store(instanceId, sha1, pluginVersion, result)   -> void
 *   ratscan_report(instanceId, sha1)                         -> ScanResult | null
 *   read_resource_file(instanceId, kind, fileName)           -> ArrayBuffer   (binary IPC, up to 256 MiB)
 *   save_resource_file(raw body, header x-args)              -> string        (new file in the resource folder, never replaces; resolves to the name used)
 *   resourcepack_compat(instanceId)                          -> PackCompat[]  (zip packs: declared releases and whether they differ from the instance)
 * Events (backend -> frontend, via emit)
 *   microsoft-login-stage  LoginStage  (a step of finish_microsoft_login has started)
 *   progress        ProgressEvent    (a task ends with `finished` or `error`)
 *   game-log        GameLogBatch     (lines of one session, batched)
 *   instance-state  InstanceRunState (one session; `stopped` is the last event of a session)
 *   game-crashed    GameCrashed      (after `stopped`, when the game exited on its own with a non-zero code; a window created later
 *                                     finds the crash in `list_run_states` until `dismiss_crash`)
 */
import { convertFileSrc } from '@tauri-apps/api/core'
import type {
  Account,
  AddFilesReport,
  AvailableUpdate,
  BlockedFile,
  BlockedScan,
  DeviceCodeInfo,
  ElybyLoginResult,
  DragDropEvent,
  ExportEntry,
  ExportRequest,
  PackExportReport,
  ClientId,
  ClientTerms,
  CreateInstanceRequest,
  FtbPack,
  CatPack,
  IconSet,
  LauncherFolder,
  ThemeFiles,
  ManagedJavaUsage,
  FtbVersion,
  GameLogBatch,
  GameCrashed,
  GameLogLine,
  GlobalSettings,
  Instance,
  InstanceListing,
  InstanceRunState,
  InstanceSettingsUpdate,
  InstallReport,
  JavaInstall,
  LoaderId,
  LoaderVersion,
  LocalResource,
  LocalResourceKind,
  JarScan,
  PackCompat,
  PluginId,
  PluginInfo,
  ScanResult,
  MinecraftVersion,
  ModpackImport,
  OnboardingInfo,
  Platform,
  LoginStage,
  ProgressEvent,
  ProjectVersion,
  RelocationReport,
  RunSession,
  Screenshot,
  SearchPage,
  SearchQuery,
  Server,
  ServerStatus,
  SessionId,
  SharedFolder,
  SystemInfo,
  UpdateStatus,
  UninstallPlan,
  UploadedLog,
  VersionFilter,
  World,
} from './types'

export type Unlisten = () => void
type Args = Record<string, unknown>

interface Backend {
  /** `args` is a Uint8Array for commands that take a raw body; the other arguments then travel in `headers`. */
  invoke: (cmd: string, args?: Args | Uint8Array, headers?: Record<string, string>) => Promise<unknown>
  listen: (event: string, cb: (payload: unknown) => void) => Promise<Unlisten>
  dragDrop: (cb: (event: DragDropEvent) => void) => Promise<Unlisten>
}

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

let backendPromise: Promise<Backend> | undefined

function loadBackend(): Promise<Backend> {
  backendPromise ??= isTauri
    ? Promise.all([
        import('@tauri-apps/api/core'),
        import('@tauri-apps/api/event'),
        import('@tauri-apps/api/webviewWindow'),
      ]).then(([core, event, webviewWindow]) => ({
        invoke: (cmd, args, headers) => core.invoke(cmd, args, headers ? { headers } : undefined),
        listen: (name, cb) => event.listen(name, (e) => cb(e.payload)),
        dragDrop: (cb) =>
          webviewWindow.getCurrentWebviewWindow().onDragDropEvent((e) => {
            const p = e.payload
            cb(p.type === 'enter' || p.type === 'drop' ? { type: p.type, paths: p.paths } : { type: p.type })
          }),
      }))
    : import('./mock').then((m) => m.mockBackend)
  return backendPromise
}

async function call<T>(cmd: string, args?: Args): Promise<T> {
  const backend = await loadBackend()
  return (await backend.invoke(cmd, args)) as T
}

async function callBinary<T>(cmd: string, body: Uint8Array, headers: Record<string, string>): Promise<T> {
  const backend = await loadBackend()
  return (await backend.invoke(cmd, body, headers)) as T
}

async function subscribe<T>(event: string, cb: (payload: T) => void): Promise<Unlisten> {
  const backend = await loadBackend()
  return backend.listen(event, (payload) => cb(payload as T))
}

// ---- instances ----
export const listInstances = () => call<InstanceListing>('list_instances')
export const createInstance = (request: CreateInstanceRequest) =>
  call<Instance>('create_instance', { request })
export const renameInstance = (instanceId: string, name: string) =>
  call<Instance>('rename_instance', { instanceId, name })
export const deleteInstance = (instanceId: string) => call<void>('delete_instance', { instanceId })
export const copyInstance = (instanceId: string, name: string) =>
  call<Instance>('copy_instance', { instanceId, name })
export const setInstanceGroup = (instanceId: string, group: string | null) =>
  call<Instance>('set_instance_group', { instanceId, group })
export const updateInstanceSettings = (instanceId: string, settings: InstanceSettingsUpdate) =>
  call<Instance>('update_instance_settings', { instanceId, settings })
export const changeInstanceVersions = (
  instanceId: string,
  minecraftVersion: string,
  loader: CreateInstanceRequest['loader'] | null,
  client?: ClientId,
) => call<Instance>('change_instance_versions', { instanceId, minecraftVersion, loader, client })
export const getInstancesDir = () => call<string>('get_instances_dir')
export const setInstanceIcon = (instanceId: string, path: string) => call<Instance>('set_instance_icon', { instanceId, path })
export const resetInstanceIcon = (instanceId: string) => call<Instance>('reset_instance_icon', { instanceId })
export const openInstanceFolder = (instanceId: string) =>
  call<void>('open_instance_folder', { instanceId })

// ---- launch ----
export const launchInstance = (instanceId: string, accountId?: string, server?: string) =>
  call<SessionId>('launch_instance', { instanceId, accountId, server })
export const killSession = (sessionId: SessionId) => call<void>('kill_session', { sessionId })
export const killInstance = (instanceId: string) => call<void>('kill_instance', { instanceId })
export const listRunStates = () => call<RunSession[]>('list_run_states')
export const getGameLogs = (sessionId: SessionId) => call<GameLogLine[]>('get_game_logs', { sessionId })
export const uploadGameLog = (sessionId: SessionId) => call<UploadedLog>('upload_game_log', { sessionId })
export const openCrashReport = (sessionId: SessionId) => call<void>('open_crash_report', { sessionId })
export const dismissCrash = (sessionId: SessionId) => call<void>('dismiss_crash', { sessionId })

// ---- versions ----
export const listMinecraftVersions = () => call<MinecraftVersion[]>('list_minecraft_versions')
export const listLoaderVersions = (minecraftVersion: string, loader: LoaderId) =>
  call<LoaderVersion[]>('list_loader_versions', { minecraftVersion, loader })

export const listClientVersions = (client: ClientId) => call<string[]>('list_client_versions', { client })
export const getClientTerms = (client: ClientId) => call<ClientTerms>('get_client_terms', { client })
export const acceptClientTerms = (client: ClientId) => call<void>('accept_client_terms', { client })

// ---- java ----
export const detectJava = () => call<JavaInstall[]>('detect_java')
export const listManagedJava = () => call<JavaInstall[]>('list_managed_java')
export const listManagedJavaUsage = () => call<ManagedJavaUsage[]>('list_managed_java_usage')
export const deleteManagedJava = (dir: string) => call<void>('delete_managed_java', { dir })

// ---- settings / system ----
export const getSettings = () => call<GlobalSettings>('get_settings')
export const saveSettings = (settings: GlobalSettings) => call<void>('save_settings', { settings })
export const getSystemInfo = () => call<SystemInfo>('get_system_info')
export const checkForUpdate = (force: boolean) => call<UpdateStatus>('check_for_update', { force })
export const installUpdate = (version: string) => call<void>('install_update', { version })
export const openDataDir = () => call<void>('open_data_dir')
export const openLauncherFolder = (folder: LauncherFolder) => call<void>('open_launcher_folder', { folder })
export const listIconLibrary = () => call<string[]>('list_icon_library')
export const listCatPacks = () => call<CatPack[]>('list_cat_packs')
export const listIconSets = () => call<IconSet[]>('list_icon_sets')
export const listThemeFiles = () => call<ThemeFiles>('list_theme_files')
export const openSharedFolder = (folder: SharedFolder) => call<void>('open_shared_folder', { folder })
export const setDataDir = (path: string | null) => call<boolean>('set_data_dir', { path })

// ---- onboarding / install ----
export const getOnboarding = () => call<OnboardingInfo | null>('get_onboarding')
export const relocate = (target: string, desktopShortcut: boolean, startMenuShortcut: boolean) =>
  call<void>('relocate', { target, desktopShortcut, startMenuShortcut })
export const keepPortableHere = (remember: boolean) => call<void>('keep_portable_here', { remember })
export const getRelocationReport = () => call<RelocationReport | null>('get_relocation_report')
export const createShortcuts = (desktop: boolean, startMenu: boolean) =>
  call<string[]>('create_shortcuts', { desktop, startMenu })
export const getUninstallPlan = () => call<UninstallPlan>('get_uninstall_plan')
export const uninstall = () => call<void>('uninstall')

// ---- accounts ----
export const addOfflineAccount = (name: string) => call<Account>('add_offline_account', { name })
export const removeAccount = (accountId: string) => call<void>('remove_account', { accountId })
export const startMicrosoftLogin = () => call<DeviceCodeInfo>('start_microsoft_login')
export const finishMicrosoftLogin = () => call<Account>('finish_microsoft_login')
export const cancelMicrosoftLogin = () => call<void>('cancel_microsoft_login')
export const elybyLogin = (username: string, password: string, totp: string | null) =>
  call<ElybyLoginResult>('elyby_login', { username, password, totp })
export const selectAccount = (accountId: string) => call<void>('select_account', { accountId })

// ---- content ----
export const searchContent = (query: SearchQuery) => call<SearchPage>('search_content', { query })
export const getProjectVersions = (platform: Platform, projectId: string, filter: VersionFilter) =>
  call<ProjectVersion[]>('get_project_versions', { platform, projectId, filter })
export const installVersion = (
  instanceId: string,
  version: ProjectVersion,
  kind: LocalResourceKind,
) =>
  call<InstallReport>('install_version', {
    instanceId,
    platform: version.platform,
    projectId: version.projectId,
    versionId: version.id,
    kind,
  })
export const listLocalResources = (instanceId: string, kind: LocalResourceKind) =>
  call<LocalResource[]>('list_local_resources', { instanceId, kind })
export const toggleResource = (
  instanceId: string,
  kind: LocalResourceKind,
  fileName: string,
  enabled: boolean,
) => call<void>('toggle_resource', { instanceId, kind, fileName, enabled })
export const deleteResource = (instanceId: string, kind: LocalResourceKind, fileName: string) =>
  call<void>('delete_resource', { instanceId, kind, fileName })
export const checkUpdates = (instanceId: string, kind: LocalResourceKind) =>
  call<AvailableUpdate[]>('check_updates', { instanceId, kind })
export const importModpackFile = (path: string, name?: string) =>
  call<ModpackImport>('import_modpack_file', { path, name })
export const installModpackVersion = (version: ProjectVersion, name?: string) =>
  call<ModpackImport>('install_modpack_version', {
    platform: version.platform,
    projectId: version.projectId,
    versionId: version.id,
    name,
  })

// ---- worlds ----
export const importModpackUrl = (url: string, name?: string) => call<ModpackImport>('import_modpack_url', { url, name })
export const ftbSearchPacks = (query: string) => call<FtbPack[]>('ftb_search_packs', { query })
export const ftbPackVersions = (packId: number) => call<FtbVersion[]>('ftb_pack_versions', { packId })
export const installFtbPack = (packId: number, versionId: number, name?: string) =>
  call<ModpackImport>('install_ftb_pack', { packId, versionId, name })
export const listWorlds = (instanceId: string) => call<World[]>('list_worlds', { instanceId })
export const renameWorld = (instanceId: string, dirName: string, name: string, renameFolder: boolean) =>
  call<string>('rename_world', { instanceId, dirName, name, renameFolder })
export const copyWorld = (instanceId: string, dirName: string, name: string) =>
  call<string>('copy_world', { instanceId, dirName, name })
export const deleteWorld = (instanceId: string, dirName: string) =>
  call<void>('delete_world', { instanceId, dirName })
export const backupWorld = (instanceId: string, dirName: string, destination?: string) =>
  call<string>('backup_world', { instanceId, dirName, destination })
export const openWorldFolder = (instanceId: string, dirName: string) =>
  call<void>('open_world_folder', { instanceId, dirName })
export const importWorld = (instanceId: string, path: string) =>
  call<string>('import_world', { instanceId, path })
export const listDatapacks = (instanceId: string, world: string) =>
  call<LocalResource[]>('list_datapacks', { instanceId, world })
export const toggleDatapack = (instanceId: string, world: string, fileName: string, enabled: boolean) =>
  call<void>('toggle_datapack', { instanceId, world, fileName, enabled })
export const deleteDatapack = (instanceId: string, world: string, fileName: string) =>
  call<void>('delete_datapack', { instanceId, world, fileName })
export const addDatapack = (instanceId: string, world: string, path: string) =>
  call<string>('add_datapack', { instanceId, world, path })
export const installDatapack = (instanceId: string, world: string, version: ProjectVersion) =>
  call<InstallReport>('install_datapack', {
    instanceId,
    world,
    platform: version.platform,
    projectId: version.projectId,
    versionId: version.id,
  })

// ---- screenshots ----
export const listScreenshots = (instanceId: string) => call<Screenshot[]>('list_screenshots', { instanceId })
export const openScreenshot = (instanceId: string, fileName: string) =>
  call<void>('open_screenshot', { instanceId, fileName })
export const deleteScreenshot = (instanceId: string, fileName: string) =>
  call<void>('delete_screenshot', { instanceId, fileName })
export const openScreenshotsFolder = (instanceId: string) =>
  call<void>('open_screenshots_folder', { instanceId })
/** URL an <img> can load for a file the backend allowed (asset protocol); the mock hands out ready URLs. */
export const fileUrl = (path: string) => (isTauri ? convertFileSrc(path) : path)

// ---- servers ----
export const listServers = (instanceId: string) => call<Server[]>('list_servers', { instanceId })
export const addServer = (instanceId: string, server: Server) => call<void>('add_server', { instanceId, server })
export const updateServer = (instanceId: string, index: number, server: Server) =>
  call<void>('update_server', { instanceId, index, server })
export const removeServer = (instanceId: string, index: number) =>
  call<void>('remove_server', { instanceId, index })
export const moveServer = (instanceId: string, from: number, to: number) =>
  call<void>('move_server', { instanceId, from, to })
export const pingServer = (address: string) => call<ServerStatus>('ping_server', { address })
export const updateModpack = (instanceId: string, versionId: string) =>
  call<ModpackImport>('update_modpack', { instanceId, versionId })

// ---- transfer ----
export const listExportEntries = (instanceId: string, dir: string) =>
  call<ExportEntry[]>('list_export_entries', { instanceId, dir })
export const exportInstance = (instanceId: string, request: ExportRequest) =>
  call<PackExportReport | null>('export_instance', { instanceId, request })
export const addLocalFiles = (instanceId: string, kind: LocalResourceKind | null, paths: string[]) =>
  call<AddFilesReport>('add_local_files_to_instance', { instanceId, kind, paths })
export const scanBlockedFiles = (instanceId: string, files: BlockedFile[], folder: string | null) =>
  call<BlockedScan>('scan_blocked_files', { instanceId, files, folder })
export const getDownloadsDir = () => call<string>('get_downloads_dir')
export const setUpdateLock = (instanceId: string, kind: LocalResourceKind, fileName: string, locked: boolean) =>
  call<void>('set_update_lock', { instanceId, kind, fileName, locked })

// ---- plugins ----
export const listPlugins = () => call<PluginInfo[]>('list_plugins')
export const readPluginFile = (id: PluginId, version: string, name: string) =>
  call<ArrayBuffer>('read_plugin_file', { id, version, name })
export const ratscanStatus = (instanceId: string) => call<JarScan[]>('ratscan_status', { instanceId })
export const ratscanStore = (instanceId: string, sha1: string, pluginVersion: string, result: ScanResult) =>
  call<void>('ratscan_store', { instanceId, sha1, pluginVersion, result })
export const ratscanReport = (instanceId: string, sha1: string) => call<ScanResult | null>('ratscan_report', { instanceId, sha1 })
export const readResourceFile = (instanceId: string, kind: LocalResourceKind, fileName: string) =>
  call<ArrayBuffer>('read_resource_file', { instanceId, kind, fileName })
/** Saves `bytes` as a new file (an existing file is never replaced); resolves to the name used. */
export function saveResourceFile(instanceId: string, kind: LocalResourceKind, fileName: string, bytes: Uint8Array) {
  const json = new TextEncoder().encode(JSON.stringify({ instanceId, kind, fileName }))
  return callBinary<string>('save_resource_file', bytes, { 'x-args': btoa(String.fromCharCode(...json)) })
}
export const resourcepackCompat = (instanceId: string) => call<PackCompat[]>('resourcepack_compat', { instanceId })

// ---- events ----
export const onMicrosoftLoginStage = (cb: (stage: LoginStage) => void) => subscribe('microsoft-login-stage', cb)
export const onProgress = (cb: (e: ProgressEvent) => void) => subscribe('progress', cb)
export const onGameLog = (cb: (e: GameLogBatch) => void) => subscribe('game-log', cb)
export const onInstanceState = (cb: (e: InstanceRunState) => void) =>
  subscribe('instance-state', cb)
export const onGameCrashed = (cb: (e: GameCrashed) => void) => subscribe('game-crashed', cb)
/** Files dropped onto the window; a no-op in the browser mock unless it is driven by hand. */
export async function onDragDrop(cb: (event: DragDropEvent) => void): Promise<Unlisten> {
  const backend = await loadBackend()
  return backend.dragDrop(cb)
}
