// Frontend <-> backend contract types. All field names are camelCase; the Rust side
// must serialize with `#[serde(rename_all = "camelCase")]`. Timestamps are ISO-8601 strings
// (UTC), durations are seconds, sizes are bytes unless the name says Mb.

/** Component uids follow Prism meta. */
export const COMPONENT_MINECRAFT = 'net.minecraft'
export const LOADER_UIDS = {
  fabric: 'net.fabricmc.fabric-loader',
  quilt: 'org.quiltmc.quilt-loader',
  forge: 'net.minecraftforge',
  neoforge: 'net.neoforged',
} as const
export type LoaderId = keyof typeof LOADER_UIDS

export interface InstanceComponent {
  uid: string
  version: string
  dependencyOnly: boolean
}

export interface Playtime {
  totalSecs: number
  lastSecs: number
}

export interface ManagedPack {
  platform: Platform
  projectId: string
  versionId: string
  name: string
  versionName: string
}

/** `null` in an override group means "inherit the global value". */
export interface JavaOverride {
  path: string | null
  autoSelect: boolean
}
export interface MemorySettings {
  minMb: number
  maxMb: number
}
export interface WindowSettings {
  width: number
  height: number
  maximized: boolean
}
export interface CommandSettings {
  preLaunch: string
  postExit: string
  wrapper: string
}

export interface InstanceOverrides {
  java: JavaOverride | null
  memory: MemorySettings | null
  jvmArgs: string | null
  window: WindowSettings | null
  commands: CommandSettings | null
}

export interface Instance {
  id: string
  name: string
  /** Built-in icon key (see InstanceIcon.vue). */
  icon: string
  group?: string
  components: InstanceComponent[]
  createdAt: string
  lastPlayed?: string
  playtime: Playtime
  managedPack?: ManagedPack
  notes: string
  shareResourcepacks: boolean
  overrides: InstanceOverrides
}

/** Fields editable through `update_instance_settings`. */
export interface InstanceSettings {
  icon: string
  notes: string
  shareResourcepacks: boolean
  overrides: InstanceOverrides
}

export interface CreateInstanceRequest {
  name: string
  group?: string
  icon: string
  minecraftVersion: string
  loader?: { id: LoaderId; version: string }
}

export type ThemeId = 'k0t1k' | 'light' | 'amoled' | 'mint'

export interface JavaSettings {
  path?: string
  autoSelect: boolean
  autoDownload: boolean
}

export interface Account {
  id: string
  name: string
  kind: 'offline'
}

export interface GlobalSettings {
  java: JavaSettings
  memory: MemorySettings
  jvmArgs: string
  window: WindowSettings
  commands: CommandSettings
  ui: { theme: ThemeId; liteMode: boolean }
  accounts: Account[]
  selectedAccountId?: string
}

export interface JavaInstall {
  path: string
  vendor: string
  version: string
  major: number
  arch: string
  /** `managed` = downloaded by the launcher, `system` = detected on the machine. */
  source: 'managed' | 'system'
}

export interface SystemInfo {
  totalRamMb: number
  os: string
  appVersion: string
  dataDir: string
}

export type VersionType = 'release' | 'snapshot' | 'old_beta' | 'old_alpha'

export interface MinecraftVersion {
  version: string
  type: VersionType
  releaseTime: string
  recommended: boolean
}

export interface LoaderVersion {
  version: string
  releaseTime: string
  recommended: boolean
}

// ---- content ----

export type Platform = 'modrinth' | 'curseforge'
export type ResourceKind = 'mod' | 'modpack' | 'resourcepack' | 'shader'
/** Kinds that exist as local files inside an instance. */
export type LocalResourceKind = Exclude<ResourceKind, 'modpack'>

export interface SearchQuery {
  platform: Platform
  kind: ResourceKind
  query: string
  gameVersion?: string
  loader?: LoaderId
  offset: number
  limit: number
}

export interface SearchHit {
  platform: Platform
  projectId: string
  kind: ResourceKind
  slug: string
  title: string
  author: string
  description: string
  iconUrl?: string
  downloads: number
  categories: string[]
  updatedAt: string
}

export interface SearchResult {
  hits: SearchHit[]
  total: number
}

export interface VersionFilter {
  gameVersion?: string
  loader?: LoaderId
}

export interface VersionFile {
  fileName: string
  size: number
  primary: boolean
  /** CurseForge: the author disabled third-party downloads for this file. */
  blocked: boolean
  /** Page where the user can download a blocked file manually. */
  manualUrl?: string
}

export interface ProjectVersion {
  id: string
  projectId: string
  platform: Platform
  name: string
  versionNumber: string
  releaseType: 'release' | 'beta' | 'alpha'
  gameVersions: string[]
  loaders: string[]
  datePublished: string
  downloads: number
  files: VersionFile[]
}

export interface ResourceMetadata {
  name: string
  version?: string
  authors?: string[]
  description?: string
  iconUrl?: string
}

export interface ResourceOrigin {
  platform: Platform
  projectId: string
  versionId: string
}

export interface LocalResource {
  fileName: string
  enabled: boolean
  size: number
  metadata?: ResourceMetadata
  origin?: ResourceOrigin
}

export interface ResourceUpdate {
  fileName: string
  newVersion: ProjectVersion
}

export interface World {
  dirName: string
  name: string
  gameMode: string
  lastPlayed?: string
  size: number
}

export type InstallOutcome =
  | { status: 'installed'; fileName: string }
  | { status: 'blocked'; fileName: string; manualUrl: string }

// ---- events ----

export interface ProgressEvent {
  /** Stable task id; events with the same id update the same row. */
  task: string
  title?: string
  stage: string
  done: number
  total: number
  finished?: boolean
  error?: string
}

export type LogStream = 'stdout' | 'stderr' | 'launcher'

export interface GameLogLine {
  instanceId: string
  stream: LogStream
  line: string
}

export type RunState = 'stopped' | 'preparing' | 'running' | 'stopping'

export interface InstanceRunState {
  instanceId: string
  state: RunState
  /** Set when the game exited unsuccessfully. */
  exitCode?: number
}
