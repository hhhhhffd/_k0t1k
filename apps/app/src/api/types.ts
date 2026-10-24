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
  /** Choose the heap and GC flags at launch; minMib and maxMib are then ignored. */
  auto: boolean
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

/** Folders that can be shared between instances; mods, config and saves never are. */
export const SHARED_FOLDERS = ['resourcepacks', 'shaderpacks', 'screenshots', 'servers'] as const
export type SharedFolder = (typeof SHARED_FOLDERS)[number]

export interface Instance {
  id: string
  name: string
  /** Built-in icon key (see InstanceIcon.vue), or `custom:<file name>` for an image in the instance folder. */
  icon: string
  group: string | null
  components: InstanceComponent[]
  created: string
  lastPlayed: string | null
  playtime: Playtime
  managedPack: ManagedPack | null
  notes: string
  settings: InstanceSettings
  /** Game folders linked to the folder shared with other instances (`shared/<folder>`). */
  sharedFolders: SharedFolder[]
  /** Server (`host` or `host:port`) the game joins on every launch. */
  joinServer?: string | null
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
  sharedFolders: SharedFolder[]
  joinServer?: string | null
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

export type AccountKind = 'offline' | 'microsoft' | 'elyby'

/** The public account list; tokens never reach the UI. */
export interface Account {
  id: string
  name: string
  kind: AccountKind
}

/** Microsoft device code the user enters at `verificationUri`. */
export interface DeviceCodeInfo {
  userCode: string
  verificationUri: string
  expiresIn: number
}

/** `twoFactorRequired`: ask for the TOTP code and call elyby_login again with it. */
export interface ElybyLoginResult {
  account: Account | null
  twoFactorRequired: boolean
}

/** What the window does while a game runs: destroy it (frees the web view's memory), hide it, or leave it. */
export type OnGameLaunch = 'free_memory' | 'hide_to_tray' | 'keep_open'
/** What the window's close button does: close to the tray or exit the launcher. */
export type OnClose = 'tray' | 'exit'

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
  behavior: { onGameLaunch: OnGameLaunch; onClose: OnClose }
  discord: { enabled: boolean }
  /** Owned by the onboarding commands: `save_settings` never overwrites it. */
  onboarding: { keepPortableAt: string | null; createdShortcuts: string[] }
  /** Folders that new instances (and imported modpacks) share. */
  sharedFoldersDefault: SharedFolder[]
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

/** Why the launcher asks where to live: its folder is Desktop/Downloads/Documents, or it runs from the temp folder (a zip). */
export interface OnboardingInfo {
  reason: 'folder' | 'temp'
  exeDir: string
  /** Folder of the recommended install. */
  defaultTarget: string
}

/** What the previous launcher process left to tell the user after an install. */
export interface RelocationReport {
  installedTo: string | null
  /** Entries of the old folder that were moved to the new one. */
  moved: string[]
  /** Things that could not be moved, deleted or created, each with the path where it is. */
  problems: string[]
}

export interface UninstallPlan {
  /** The launcher's content in these folders is deleted; other files in them stay. */
  folders: string[]
  shortcuts: string[]
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
  /** Set by the user (or by a modpack install): bulk updates skip this file. */
  'x-prismlauncher-lock-update'?: boolean
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

export type GameMode = 'survival' | 'creative' | 'adventure' | 'spectator'

/** A world of `saves/`; details come from `level.dat` and are null when the file lacks them or cannot be read. */
export interface World {
  /** Folder name under `saves/`. */
  dirName: string
  /** `LevelName`; the folder name for an unreadable world. */
  name: string
  gameMode: GameMode | null
  hardcore: boolean | null
  versionName: string | null
  lastPlayed: string | null
  /** Decimal string: a 64-bit seed does not fit a JS number. */
  seed: string | null
  /** `data:image/png;base64,...` of `icon.png`. */
  icon: string | null
  size: number
}

export interface Screenshot {
  fileName: string
  /** Absolute path; pass it to `fileUrl` to get a URL an <img> can load. */
  path: string
  size: number
  /** Milliseconds since the Unix epoch. */
  modified: number
}

/** `acceptTextures` of `servers.dat`: `prompt` means the tag is absent and the game asks. */
export type ResourcePackPolicy = 'prompt' | 'accept' | 'decline'

export interface Server {
  name: string
  address: string
  resourcePack: ResourcePackPolicy
}

export interface ServerStatus {
  /** Plain text, formatting codes removed. */
  motd: string
  version: string
  protocol: number
  online: number
  max: number
  latencyMs: number
  /** `data:image/png;base64,...` as the server sent it. */
  favicon: string | null
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

/** A Feed The Beast modpack (api.feed-the-beast.com). */
export interface FtbPack {
  id: number
  name: string
  summary: string
  authors: string[]
  iconUrl: string | null
  installs: number
  updated: string
  tags: string[]
}

export interface FtbVersion {
  id: number
  name: string
  type: string
  updated: string
  minecraftVersion: string
  loader: { id: LoaderId; version: string } | null
}

export type ExportFormat = 'instance' | 'modrinth' | 'curseForge'

export interface ExportRequest {
  format: ExportFormat
  /** Where to save the archive. */
  destination: string
  name: string
  version: string
  summary: string
  /** Game folder paths (`/` separators) left out, each with everything below it. */
  excluded: string[]
}

/** One row of the export file picker: an entry of a game folder directory. */
export interface ExportEntry {
  name: string
  /** Relative to the game folder. */
  path: string
  isDir: boolean
  /** Bytes; `null` for folders. */
  size: number | null
  /** A shared folder: exported as a copy of its content, or skipped. */
  linked: boolean
  /** Unchecked when the picker opens (logs, crash reports, caches). */
  defaultExcluded: boolean
}

/** Result of a Modrinth/CurseForge export: files listed for download and files packed as overrides. */
export interface PackExportReport {
  listed: number
  overrides: number
}

export type SkipReason = 'notAFile' | 'unsupportedType' | 'unknownType' | 'worldOrDataPack'

export interface AddFilesReport {
  added: { fileName: string; kind: ResourceKind }[]
  skipped: { path: string; reason: SkipReason }[]
}

/** `targetPath`s of blocked files that are now in the instance, and those still missing. */
export interface BlockedScan {
  found: string[]
  missing: string[]
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
  /** Numbers lines across all instances; the backend keeps the order, so a larger one is newer. */
  seq: number
  stream: LogStream
  line: string
}

/** One launch of an instance. An instance may have any number of sessions at once. */
export type SessionId = number

/** `game-log` event payload: the backend sends lines in batches. */
export interface GameLogBatch {
  sessionId: SessionId
  lines: GameLogLine[]
}

export type RunState = 'stopped' | 'preparing' | 'running' | 'stopping'

/** `instance-state` event payload: one session changed state. */
export interface InstanceRunState {
  sessionId: SessionId
  instanceId: string
  /** 1 for the first launch of the instance in this app run, 2 for the next, ... Stable for the session. */
  number: number
  state: RunState
  /** Set when the game process has exited, if the OS reported a code. */
  exitCode?: number
}

/** `list_run_states` item: a live session, or (`state: 'stopped'`) a finished one whose log is still kept. */
export interface RunSession {
  sessionId: SessionId
  instanceId: string
  number: number
  state: RunState
  /** ISO time of the launch request. */
  startedAt: string
  /** Set for a crashed session until `dismiss_crash`. */
  crash?: CrashInfo
}

/** How a session ended when the game crashed (it exited on its own with a non-zero code or by a signal). */
export interface CrashInfo {
  /** Absent when a signal ended the process. */
  exitCode: number | null
  /** Path of a crash report or JVM crash log written during the session. */
  crashReport: string | null
}

/** `game-crashed` event payload. */
export interface GameCrashed extends CrashInfo {
  sessionId: SessionId
  instanceId: string
}

/** `upload_game_log` result: the log page on mclo.gs. */
export interface UploadedLog {
  id: string
  url: string
  raw: string
}
/** Files dragged over the window (real paths, from Tauri's drag-drop events). */
export type DragDropEvent = { type: 'enter' | 'drop'; paths: string[] } | { type: 'over' } | { type: 'leave' }
