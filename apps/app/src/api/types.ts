// Frontend <-> backend contract types. They mirror what `k0t1k-core` serialises (camelCase JSON, which
// is also the on-disk format), so a field's `null` means a Rust `Option::None`. The few exceptions are
// thin Rust view types: `Instance` (+ `id`), `JavaInstall` (+ `source`), local resource icons and `World`.
// Timestamps are ISO-8601 strings (UTC), durations are seconds, sizes are bytes, memory is MiB.

/** Component uids follow Prism meta. */
export const COMPONENT_MINECRAFT = 'net.minecraft'
export const LOADER_UIDS = {
  fabric: 'net.fabricmc.fabric-loader',
  quilt: 'org.quiltmc.quilt-loader',
  forge: 'net.minecraftforge',
  neoForge: 'net.neoforged',
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
  /** Note: lowercase here, unlike `Platform`. */
  platform: 'modrinth' | 'curseforge'
  projectId: string
  versionId: string
  name: string
  versionName: string
}

export interface JavaSettings {
  path: string | null
  autoSelect: boolean
  autoDownload: boolean
}
export interface MemorySettings {
  minMib: number
  maxMib: number
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

/** Per-instance override groups; a missing group inherits the global value. */
export interface InstanceSettings {
  java?: JavaSettings | null
  memory?: MemorySettings | null
  jvmArgs?: string | null
  window?: WindowSettings | null
  commands?: CommandSettings | null
  env?: Record<string, string> | null
}

export interface Instance {
  id: string
  name: string
  /** Built-in icon key (see InstanceIcon.vue). */
  icon: string
  group: string | null
  components: InstanceComponent[]
  created: string
  lastPlayed: string | null
  playtime: Playtime
  managedPack: ManagedPack | null
  notes: string
  settings: InstanceSettings
  shareResourcepacks: boolean
}

/** An instance whose `instance.json` could not be read. */
export interface BrokenInstance {
  id: string
  error: string
}

export interface InstanceListing {
  instances: Instance[]
  errors: BrokenInstance[]
}

/** Fields editable through `update_instance_settings`. */
export interface InstanceSettingsUpdate {
  icon: string
  notes: string
  shareResourcepacks: boolean
  settings: InstanceSettings
}

export interface CreateInstanceRequest {
  name: string
  group?: string
  icon: string
  minecraftVersion: string
  loader?: { id: LoaderId; version: string }
}

export type ThemeId = 'k0t1k' | 'light' | 'amoled' | 'mint'

/**
 * A user-made theme: a built-in `base` plus CSS variable overrides. Color keys are variable names without the
 * leading `--` (`[a-z0-9-]`), values are CSS colors; the backend rejects values outside `^[#a-zA-Z0-9(),.%\s-]{1,64}$`.
 */
export interface CustomTheme {
  id: string
  name: string
  base: ThemeId
  colors: Record<string, string>
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
  env: Record<string, string>
  playtime: { record: boolean; show: boolean; totalSecs: number }
  /** `theme` is a built-in `ThemeId` or the id of one of `customThemes`. */
  ui: { theme: string; liteMode: boolean; customThemes: CustomTheme[] }
  accounts: Account[]
  selectedAccountId: string | null
}

export interface JavaInstall {
  path: string
  /** Display string, e.g. `21.0.7` or `1.8.0_392`. */
  version: string
  major: number
  /** Raw `os.arch`, e.g. `amd64`. */
  arch: string
  vendor: string
  is64bit: boolean
  /** `managed` = downloaded by the launcher, `system` = detected on the machine. */
  source: 'managed' | 'system'
}

export interface SystemInfo {
  totalRamMib: number | null
  os: string
  appVersion: string
  dataDir: string
  /** The app keeps its data next to the executable. */
  portable: boolean
  /** Folder of the executable. */
  exeDir: string
}

/** An entry of a Prism meta version list. `type` is `release`/`snapshot`/`old_beta`/`old_alpha` for Minecraft. */
export interface VersionEntry {
  version: string
  type: string | null
  releaseTime: string
  recommended: boolean
}
export type MinecraftVersion = VersionEntry
export type LoaderVersion = VersionEntry

// ---- content ----

export type Platform = 'modrinth' | 'curseForge'
export type ResourceKind = 'mod' | 'resourcePack' | 'shaderPack' | 'dataPack' | 'modpack' | 'world'
/** Kinds that exist as files inside an instance and can be installed into it. */
export type LocalResourceKind = Exclude<ResourceKind, 'modpack' | 'world'>

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
  slug: string
  title: string
  description: string
  author: string
  iconUrl: string | null
  downloads: number
  categories: string[]
}

export interface SearchPage {
  hits: SearchHit[]
  offset: number
  totalHits: number
}

export interface VersionFilter {
  gameVersion?: string
  loader?: LoaderId
}

export interface VersionFile {
  /** `null` when the author disabled third-party downloads (CurseForge); see `downloadPage`. */
  url: string | null
  /** Page where the user can download a file with no `url` by hand. */
  downloadPage: string | null
  filename: string
  size: number
  hashes: { sha1: string | null; sha512: string | null }
  primary: boolean
}

export interface ProjectVersion {
  platform: Platform
  id: string
  projectId: string
  name: string
  versionNumber: string
  releaseType: 'release' | 'beta' | 'alpha'
  gameVersions: string[]
  loaders: LoaderId[]
  files: VersionFile[]
  dependencies: { projectId: string | null; versionId: string | null; kind: 'required' | 'optional' | 'embedded' | 'incompatible' }[]
  published: string
}

export interface ResourceMetadata {
  id: string
  name: string
  version: string | null
  authors: string[]
  description: string | null
  /** `data:image/png;base64,...` URL of the embedded icon. */
  icon: string | null
  loader: LoaderId | null
}

/** The packwiz `.pw.toml` entry that describes where a local file came from (only the fields the UI reads). */
export interface PwMod {
  name: string
  filename: string
  update: {
    modrinth?: { 'mod-id': string; version: string }
    curseforge?: { 'file-id': number; 'project-id': number }
  }
}

export interface LocalResource {
  fileName: string
  enabled: boolean
  size: number
  metadata: ResourceMetadata | null
  origin: PwMod | null
}

export interface AvailableUpdate {
  fileName: string
  name: string
  currentVersion: string
  latest: ProjectVersion
}

export interface World {
  dirName: string
  size: number
  lastPlayed: string | null
}

/** A file the author forbids third parties to download; the user fetches it and drops it into `targetPath`. */
export interface BlockedFile {
  platform: Platform
  projectId: string
  name: string
  fileName: string
  /** Relative to the instance's game folder, e.g. `mods/example.jar`. */
  targetPath: string
  downloadPage: string | null
  sha1: string | null
}

export interface InstallReport {
  installed: { platform: Platform; projectId: string; name: string; fileName: string; isDependency: boolean }[]
  blocked: BlockedFile[]
  /** Required dependencies without a compatible version, as `platform:project id`. */
  unresolvedDependencies: string[]
}

export interface ModpackImport {
  instance: Instance
  blocked: BlockedFile[]
}

// ---- events ----

export interface ProgressEvent {
  /** Stable task id; events with the same id update the same row. */
  task: string
  title?: string
  stage: string
  done: number
  total: number
  /** The task ended successfully. */
  finished?: boolean
  /** The task failed; this is the message. */
  error?: string
}

export type LogStream = 'stdout' | 'stderr' | 'launcher'

export interface GameLogLine {
  stream: LogStream
  line: string
}

/** `game-log` event payload: the backend sends lines in batches. */
export interface GameLogBatch {
  instanceId: string
  lines: GameLogLine[]
}

export type RunState = 'stopped' | 'preparing' | 'running' | 'stopping'

export interface InstanceRunState {
  instanceId: string
  state: RunState
  /** Set when the game process has exited, if the OS reported a code. */
  exitCode?: number
}
