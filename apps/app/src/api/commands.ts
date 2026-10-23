/*
 * Frontend <-> backend CONTRACT. Every function below is one Tauri command (snake_case name).
 * Argument object keys are camelCase (Tauri converts them to the snake_case Rust parameters).
 * Errors: the command returns Err(String); the promise rejects with that string.
 *
 * Instances
 *   list_instances()                                         -> Instance[]
 *   create_instance(request: CreateInstanceRequest)          -> Instance      (emits `progress` while downloading)
 *   rename_instance(instanceId, name)                        -> Instance
 *   delete_instance(instanceId)                              -> void
 *   copy_instance(instanceId, name)                          -> Instance
 *   set_instance_group(instanceId, group: string | null)     -> Instance
 *   update_instance_settings(instanceId, settings: InstanceSettings) -> Instance
 *   open_instance_folder(instanceId)                         -> void
 * Launch
 *   launch_instance(instanceId, accountId?)                  -> void          (state via `instance-state`, output via `game-log`)
 *   kill_instance(instanceId)                                -> void
 *   list_run_states()                                        -> InstanceRunState[]
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
 * Accounts (offline for now; settings.accounts mirrors the result)
 *   add_offline_account(name)                                -> Account
 *   remove_offline_account(accountId)                        -> void
 *   select_account(accountId)                                -> void
 * Content
 *   search_content(query: SearchQuery)                       -> SearchResult
 *   get_project_versions(platform, projectId, filter: VersionFilter) -> ProjectVersion[]
 *   install_version(instanceId, version: ProjectVersion, kind: LocalResourceKind) -> InstallOutcome
 *   list_local_resources(instanceId, kind: LocalResourceKind) -> LocalResource[]
 *   toggle_resource(instanceId, kind, fileName, enabled)     -> void
 *   delete_resource(instanceId, kind, fileName)              -> void
 *   check_updates(instanceId, kind: LocalResourceKind)       -> ResourceUpdate[]
 *   list_worlds(instanceId)                                  -> World[]
 *   import_modpack_file(path, name?)                         -> Instance      (.mrpack / CurseForge .zip; emits `progress`)
 *   install_modpack_version(version: ProjectVersion, name?)  -> Instance      (emits `progress`)
 * Events (backend -> frontend, via emit)
 *   progress        ProgressEvent
 *   game-log        GameLogLine
 *   instance-state  InstanceRunState
 */
import type {
  CreateInstanceRequest,
  GameLogLine,
  GlobalSettings,
  Instance,
  InstanceRunState,
  InstanceSettings,
  InstallOutcome,
  JavaInstall,
  LoaderId,
  LoaderVersion,
  LocalResource,
  LocalResourceKind,
  MinecraftVersion,
  Platform,
  ProgressEvent,
  ProjectVersion,
  ResourceUpdate,
  SearchQuery,
  SearchResult,
  SystemInfo,
  VersionFilter,
  World,
  Account,
} from './types'

export type Unlisten = () => void
type Args = Record<string, unknown>

interface Backend {
  invoke: (cmd: string, args?: Args) => Promise<unknown>
  listen: (event: string, cb: (payload: unknown) => void) => Promise<Unlisten>
}

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

let backendPromise: Promise<Backend> | undefined

function loadBackend(): Promise<Backend> {
  backendPromise ??= isTauri
    ? Promise.all([import('@tauri-apps/api/core'), import('@tauri-apps/api/event')]).then(
        ([core, event]) => ({
          invoke: core.invoke,
          listen: (name, cb) => event.listen(name, (e) => cb(e.payload)),
        }),
      )
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
export const listInstances = () => call<Instance[]>('list_instances')
export const createInstance = (request: CreateInstanceRequest) =>
  call<Instance>('create_instance', { request })
export const renameInstance = (instanceId: string, name: string) =>
  call<Instance>('rename_instance', { instanceId, name })
export const deleteInstance = (instanceId: string) => call<void>('delete_instance', { instanceId })
export const copyInstance = (instanceId: string, name: string) =>
  call<Instance>('copy_instance', { instanceId, name })
export const setInstanceGroup = (instanceId: string, group: string | null) =>
  call<Instance>('set_instance_group', { instanceId, group })
export const updateInstanceSettings = (instanceId: string, settings: InstanceSettings) =>
  call<Instance>('update_instance_settings', { instanceId, settings })
export const openInstanceFolder = (instanceId: string) =>
  call<void>('open_instance_folder', { instanceId })

// ---- launch ----
export const launchInstance = (instanceId: string, accountId?: string) =>
  call<void>('launch_instance', { instanceId, accountId })
export const killInstance = (instanceId: string) => call<void>('kill_instance', { instanceId })
export const listRunStates = () => call<InstanceRunState[]>('list_run_states')

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

// ---- accounts ----
export const addOfflineAccount = (name: string) => call<Account>('add_offline_account', { name })
export const removeOfflineAccount = (accountId: string) =>
  call<void>('remove_offline_account', { accountId })
export const selectAccount = (accountId: string) => call<void>('select_account', { accountId })

// ---- content ----
export const searchContent = (query: SearchQuery) => call<SearchResult>('search_content', { query })
export const getProjectVersions = (platform: Platform, projectId: string, filter: VersionFilter) =>
  call<ProjectVersion[]>('get_project_versions', { platform, projectId, filter })
export const installVersion = (
  instanceId: string,
  version: ProjectVersion,
  kind: LocalResourceKind,
) => call<InstallOutcome>('install_version', { instanceId, version, kind })
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
  call<ResourceUpdate[]>('check_updates', { instanceId, kind })
export const listWorlds = (instanceId: string) => call<World[]>('list_worlds', { instanceId })
export const importModpackFile = (path: string, name?: string) =>
  call<Instance>('import_modpack_file', { path, name })
export const installModpackVersion = (version: ProjectVersion, name?: string) =>
  call<Instance>('install_modpack_version', { version, name })

// ---- events ----
export const onProgress = (cb: (e: ProgressEvent) => void) => subscribe('progress', cb)
export const onGameLog = (cb: (e: GameLogLine) => void) => subscribe('game-log', cb)
export const onInstanceState = (cb: (e: InstanceRunState) => void) =>
  subscribe('instance-state', cb)
