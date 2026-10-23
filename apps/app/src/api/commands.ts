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
 *   open_instance_folder(instanceId)                         -> void
 * Launch
 *   launch_instance(instanceId, accountId?)                  -> SessionId     (always starts a NEW session, also when the instance already
 *                                                                              runs; state via `instance-state`, output via `game-log`)
 *   kill_session(sessionId)                                  -> void          (stops the game or cancels its preparation; errors when the session is gone)
 *   kill_instance(instanceId)                                -> void          (= kill_session for every live session; errors when none)
 *   list_run_states()                                        -> RunSession[]  (live sessions + up to 3 finished ones per instance, `state: 'stopped'`,
 *                                                                              whose log is kept; for a window opened mid-game)
 *   get_game_logs(sessionId)                                 -> GameLogLine[]  (ring buffer of one session, up to MAX_LOG_LINES)
 *
 * Sessions: ids are numbers that grow during the app run and are never reused. Each session has its own natives
 * directory and log, records its own playtime when it ends (overlapping sessions of one instance are counted
 * separately, so the total may exceed wall-clock time), and counts as a running game for the tray and for
 * `behavior.onGameLaunch`. delete_instance errors while any session of the instance is live.
 * Versions
 *   list_minecraft_versions()                                -> MinecraftVersion[]
 *   list_loader_versions(minecraftVersion, loader: LoaderId) -> LoaderVersion[]
 * Java
 *   detect_java()                                            -> JavaInstall[]  (system installs; rescans)
 *   list_managed_java()                                      -> JavaInstall[]
 * Settings / system
 *   get_settings()                                           -> GlobalSettings
 *   save_settings(settings: GlobalSettings)                  -> void
 *   get_system_info()                                        -> SystemInfo
 *   open_data_dir()                                          -> void
 *   open_shared_folder(folder: SharedFolder)                 -> void  (creates shared/<folder> if missing)
 *   set_data_dir(path: string | null)                        -> boolean  (true = restart needed; data is not moved; null = default location)
 * Accounts (offline for now; settings.accounts mirrors the result)
 *   add_offline_account(name)                                -> Account
 *   remove_offline_account(accountId)                        -> void
 *   select_account(accountId)                                -> void
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
 * Events (backend -> frontend, via emit)
 *   progress        ProgressEvent    (a task ends with `finished` or `error`)
 *   game-log        GameLogBatch     (lines of one session, batched)
 *   instance-state  InstanceRunState (one session; `stopped` is the last event of a session)
 */
import { convertFileSrc } from '@tauri-apps/api/core'
import type {
  Account,
  AddFilesReport,
  AvailableUpdate,
  BlockedFile,
  BlockedScan,
  DragDropEvent,
  ExportEntry,
  ExportRequest,
  PackExportReport,
  CreateInstanceRequest,
  GameLogBatch,
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
  MinecraftVersion,
  ModpackImport,
  Platform,
  ProgressEvent,
  ProjectVersion,
  RunSession,
  Screenshot,
  SearchPage,
  SearchQuery,
  Server,
  ServerStatus,
  SessionId,
  SharedFolder,
  SystemInfo,
  VersionFilter,
  World,
} from './types'

export type Unlisten = () => void
type Args = Record<string, unknown>

interface Backend {
  invoke: (cmd: string, args?: Args) => Promise<unknown>
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
        invoke: core.invoke,
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
export const openInstanceFolder = (instanceId: string) =>
  call<void>('open_instance_folder', { instanceId })

// ---- launch ----
export const launchInstance = (instanceId: string, accountId?: string) =>
  call<SessionId>('launch_instance', { instanceId, accountId })
export const killSession = (sessionId: SessionId) => call<void>('kill_session', { sessionId })
export const killInstance = (instanceId: string) => call<void>('kill_instance', { instanceId })
export const listRunStates = () => call<RunSession[]>('list_run_states')
export const getGameLogs = (sessionId: SessionId) => call<GameLogLine[]>('get_game_logs', { sessionId })

// ---- versions ----
export const listMinecraftVersions = () => call<MinecraftVersion[]>('list_minecraft_versions')
export const listLoaderVersions = (minecraftVersion: string, loader: LoaderId) =>
  call<LoaderVersion[]>('list_loader_versions', { minecraftVersion, loader })

// ---- java ----
export const detectJava = () => call<JavaInstall[]>('detect_java')
export const listManagedJava = () => call<JavaInstall[]>('list_managed_java')

// ---- settings / system ----
export const getSettings = () => call<GlobalSettings>('get_settings')
export const saveSettings = (settings: GlobalSettings) => call<void>('save_settings', { settings })
export const getSystemInfo = () => call<SystemInfo>('get_system_info')
export const openDataDir = () => call<void>('open_data_dir')
export const openSharedFolder = (folder: SharedFolder) => call<void>('open_shared_folder', { folder })
export const setDataDir = (path: string | null) => call<boolean>('set_data_dir', { path })

// ---- accounts ----
export const addOfflineAccount = (name: string) => call<Account>('add_offline_account', { name })
export const removeOfflineAccount = (accountId: string) =>
  call<void>('remove_offline_account', { accountId })
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

// ---- events ----
export const onProgress = (cb: (e: ProgressEvent) => void) => subscribe('progress', cb)
export const onGameLog = (cb: (e: GameLogBatch) => void) => subscribe('game-log', cb)
export const onInstanceState = (cb: (e: InstanceRunState) => void) =>
  subscribe('instance-state', cb)
/** Files dropped onto the window; a no-op in the browser mock unless it is driven by hand. */
export async function onDragDrop(cb: (event: DragDropEvent) => void): Promise<Unlisten> {
  const backend = await loadBackend()
  return backend.dragDrop(cb)
}
