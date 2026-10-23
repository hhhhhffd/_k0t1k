// In-browser mock backend, used automatically when the app runs outside Tauri.
// It implements the same commands/events as the Rust side so the UI can be developed in a browser.
import type {
  Account,
  AvailableUpdate,
  BlockedFile,
  CreateInstanceRequest,
  GameLogBatch,
  GameLogLine,
  GlobalSettings,
  Instance,
  InstanceComponent,
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
  RunState,
  Screenshot,
  SearchHit,
  SearchPage,
  SearchQuery,
  Server,
  ServerStatus,
  SessionId,
  SystemInfo,
  VersionFilter,
  World,
} from './types'
import { COMPONENT_MINECRAFT, LOADER_UIDS } from './types'

type Args = Record<string, unknown>
const listeners = new Map<string, Set<(payload: unknown) => void>>()
// JSON round-trip: unlike structuredClone it accepts Vue reactive proxies, and matches real IPC
const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v)) as T
const sleep =(ms: number) => new Promise<void>((r) => setTimeout(r, ms))
const daysAgo = (d: number) => new Date(Date.now() - d * 86_400_000).toISOString()
let idCounter = 100
const nextId = (prefix: string) => `${prefix}-${++idCounter}`

function emit(event: string, payload: unknown) {
  listeners.get(event)?.forEach((cb) => cb(payload))
}
const emitProgress = (e: ProgressEvent) => emit('progress', e)

async function fakeTask(task: string, title: string, stages: string[], stepMs = 350) {
  for (const [i, stage] of stages.entries()) {
    emitProgress({ task, title, stage, done: i, total: stages.length })
    await sleep(stepMs)
  }
  emitProgress({ task, title, stage: 'Готово', done: stages.length, total: stages.length, finished: true })
}

// ---- versions ----
const MC: [string, string, string][] = [
  ['1.21.4', 'release', '2024-12-03'], ['1.21.3', 'release', '2024-10-23'], ['1.21.1', 'release', '2024-08-08'],
  ['1.21', 'release', '2024-06-13'], ['1.20.6', 'release', '2024-04-29'], ['1.20.4', 'release', '2023-12-07'],
  ['1.20.1', 'release', '2023-06-12'], ['1.19.4', 'release', '2023-03-14'], ['1.19.2', 'release', '2022-08-05'],
  ['1.18.2', 'release', '2022-02-28'], ['1.17.1', 'release', '2021-07-06'], ['1.16.5', 'release', '2021-01-15'],
  ['1.12.2', 'release', '2017-09-18'], ['1.8.9', 'release', '2015-12-09'], ['1.7.10', 'release', '2014-06-26'],
  ['25w06a', 'snapshot', '2025-02-05'], ['25w05a', 'snapshot', '2025-01-29'], ['24w46a', 'snapshot', '2024-11-13'],
  ['b1.7.3', 'old_beta', '2011-07-08'], ['a1.2.6', 'old_alpha', '2010-12-03'],
]
const minecraftVersions: MinecraftVersion[] = MC.map(([version, type, t], i) => ({
  version, type, releaseTime: new Date(t).toISOString(), recommended: i === 0,
}))

function loaderVersions(mc: string, loader: LoaderId): LoaderVersion[] {
  const seeds: Record<LoaderId, string[]> = {
    fabric: ['0.16.10', '0.16.9', '0.16.7', '0.15.11', '0.15.7'],
    quilt: ['0.27.1', '0.27.0', '0.26.4', '0.26.0'],
    forge: [`${mc}-54.0.16`, `${mc}-54.0.12`, `${mc}-53.0.40`, `${mc}-52.0.1`],
    neoForge: ['21.4.100', '21.4.80', '21.1.172', '21.1.100'],
  }
  if (mc.length > 7 && /[a-z]/.test(mc)) return []
  return seeds[loader].map((version, i) => ({
    version, type: null, releaseTime: daysAgo(20 + i * 40), recommended: i === 0,
  }))
}

// ---- settings ----
const SETTINGS_KEY = 'k0t1k.mock.settings'
const defaultSettings: GlobalSettings = {
  java: { path: null, autoSelect: true, autoDownload: true },
  memory: { minMib: 512, maxMib: 4096 },
  jvmArgs: '-XX:+UseG1GC -XX:+ParallelRefProcEnabled',
  window: { width: 854, height: 480, maximized: false },
  commands: { preLaunch: '', postExit: '', wrapper: '' },
  env: {},
  playtime: { record: true, show: true, totalSecs: 0 },
  ui: { theme: 'k0t1k', liteMode: false, customThemes: [] },
  behavior: { onGameLaunch: 'free_memory', onClose: 'tray' },
  sharedFoldersDefault: ['resourcepacks', 'shaderpacks'],
  accounts: [{ id: 'acc-1', name: 'k0t1k_player', kind: 'offline' }],
  selectedAccountId: 'acc-1',
}
function loadSettings(): GlobalSettings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY)
    if (raw) {
      const stored = JSON.parse(raw) as Partial<GlobalSettings>
      return { ...defaultSettings, ...stored, ui: { ...defaultSettings.ui, ...stored.ui } }
    }
  } catch {
    // mock only: fall back to defaults when storage is unavailable
  }
  return clone(defaultSettings)
}
let settings = loadSettings()
function persistSettings() {
  try {
    localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings))
  } catch {
    // mock only
  }
}

const javaInstalls: JavaInstall[] = [
  { path: 'C:\\Program Files\\Eclipse Adoptium\\jdk-21.0.5.11-hotspot\\bin\\javaw.exe', vendor: 'Eclipse Adoptium', version: '21.0.5', major: 21, arch: 'amd64', is64bit: true, source: 'system' },
  { path: 'C:\\Program Files\\Java\\jre1.8.0_431\\bin\\javaw.exe', vendor: 'Oracle', version: '1.8.0_431', major: 8, arch: 'amd64', is64bit: true, source: 'system' },
  { path: 'C:\\Program Files\\Microsoft\\jdk-17.0.13.11-hotspot\\bin\\javaw.exe', vendor: 'Microsoft', version: '17.0.13', major: 17, arch: 'amd64', is64bit: true, source: 'system' },
]
const managedJava: JavaInstall[] = [
  { path: '%APPDATA%\\_k0t1k\\java\\java-runtime-delta\\bin\\javaw.exe', vendor: 'Mojang', version: '21.0.3', major: 21, arch: 'amd64', is64bit: true, source: 'managed' },
  { path: '%APPDATA%\\_k0t1k\\java\\java-runtime-gamma\\bin\\javaw.exe', vendor: 'Mojang', version: '17.0.8', major: 17, arch: 'amd64', is64bit: true, source: 'managed' },
]

// ---- instances ----
function components(mc: string, loader?: { id: LoaderId; version: string }): InstanceComponent[] {
  const list: InstanceComponent[] = [{ uid: COMPONENT_MINECRAFT, version: mc, dependencyOnly: false }]
  if (loader) list.push({ uid: LOADER_UIDS[loader.id], version: loader.version, dependencyOnly: false })
  return list
}
const newInstance = (fields: Pick<Instance, 'id' | 'name' | 'icon' | 'components'> & Partial<Instance>): Instance => ({
  group: null, created: new Date().toISOString(), lastPlayed: null, playtime: { totalSecs: 0, lastSecs: 0 },
  managedPack: null, notes: '', sharedFolders: [...settings.sharedFoldersDefault], settings: {}, ...fields,
})
const instances: Instance[] = [
  newInstance({ id: 'inst-1', name: 'Vanilla 1.21.4', icon: 'grass', components: components('1.21.4'), created: daysAgo(90), lastPlayed: daysAgo(1),
    playtime: { totalSecs: 86_400 * 1.5, lastSecs: 5400 } }),
  newInstance({ id: 'inst-2', name: 'Fabric Optimized', icon: 'zap', group: 'Выживание', components: components('1.21.1', { id: 'fabric', version: '0.16.10' }),
    created: daysAgo(60), lastPlayed: daysAgo(0.1), playtime: { totalSecs: 3600 * 74, lastSecs: 7200 }, notes: 'Sodium + Lithium + Iris',
    settings: { memory: { minMib: 1024, maxMib: 6144 } } }),
  newInstance({ id: 'inst-3', name: 'Create: Above and Beyond', icon: 'gem', group: 'Модпаки', components: components('1.20.1', { id: 'forge', version: '1.20.1-47.3.0' }),
    created: daysAgo(30), lastPlayed: daysAgo(6), playtime: { totalSecs: 3600 * 12, lastSecs: 1800 }, sharedFolders: [],
    managedPack: { platform: 'modrinth', projectId: 'create-aab', versionId: 'v-aab-1', name: 'Create: Above and Beyond', versionName: '1.4.0' } }),
  newInstance({ id: 'inst-4', name: 'PvP 1.8.9', icon: 'sword', group: 'Выживание', components: components('1.8.9', { id: 'forge', version: '1.8.9-11.15.1.2318' }),
    created: daysAgo(200), playtime: { totalSecs: 3600 * 140, lastSecs: 900 }, lastPlayed: daysAgo(30) }),
  newInstance({ id: 'inst-5', name: 'Снапшоты', icon: 'ghost', components: components('25w06a'), created: daysAgo(5) }),
]
const findInstance = (id: unknown): Instance => {
  const inst = instances.find((i) => i.id === id)
  if (!inst) throw new Error(`Инстанс не найден: ${String(id)}`)
  return inst
}

// ---- content catalog ----
type Seed = [slug: string, title: string, author: string, description: string, downloads: number, categories: string[]]
type CatalogKind = 'mod' | 'modpack' | 'resourcePack' | 'shaderPack' | 'dataPack'
const catalog: Record<CatalogKind, Seed[]> = {
  mod: [
    ['sodium', 'Sodium', 'jellysquid3', 'Современный движок рендеринга, сильно повышающий FPS.', 98_000_000, ['optimization']],
    ['lithium', 'Lithium', 'jellysquid3', 'Оптимизация игровой логики, тиков и ИИ мобов.', 61_000_000, ['optimization']],
    ['iris', 'Iris Shaders', 'coderbot', 'Поддержка шейдеров, совместимая с OptiFine.', 54_000_000, ['decoration', 'optimization']],
    ['fabric-api', 'Fabric API', 'modmuss50', 'Базовая библиотека хуков для модов Fabric.', 140_000_000, ['library']],
    ['jei', 'Just Enough Items', 'mezz', 'Просмотр рецептов и предметов.', 120_000_000, ['utility']],
    ['create', 'Create', 'simibubi', 'Механизмы, шестерни и автоматизация.', 45_000_000, ['technology']],
    ['xaeros-minimap', "Xaero's Minimap", 'xaero96', 'Миникарта с вейпоинтами.', 38_000_000, ['utility']],
    ['modmenu', 'Mod Menu', 'Prospector', 'Список установленных модов прямо в игре.', 70_000_000, ['utility']],
    ['ferrite-core', 'FerriteCore', 'malte0811', 'Снижает потребление памяти.', 52_000_000, ['optimization']],
    ['journeymap', 'JourneyMap', 'techbrew', 'Карта мира в реальном времени.', 33_000_000, ['utility']],
    ['appleskin', 'AppleSkin', 'squeek502', 'Показывает сытость и насыщение.', 29_000_000, ['utility']],
    ['sophisticated-backpacks', 'Sophisticated Backpacks', 'P3pp3rF1y', 'Рюкзаки с улучшениями.', 21_000_000, ['storage']],
  ],
  modpack: [
    ['create-aab', 'Create: Above and Beyond', 'Zeroth', 'Модпак вокруг Create с квестами и прогрессией.', 3_200_000, ['adventure']],
    ['fabulously-optimized', 'Fabulously Optimized', 'robotkoer', 'Набор оптимизаций для Fabric.', 9_500_000, ['optimization']],
    ['all-the-mods-9', 'All the Mods 9', 'ATMTeam', 'Большой модпак «всё включено».', 7_800_000, ['kitchen-sink']],
    ['better-mc', 'Better MC', 'LunaPixelStudios', 'Ванильное расширение с новыми биомами.', 4_100_000, ['adventure']],
    ['rlcraft', 'RLCraft', 'Shivaxi', 'Хардкорное выживание в жестком мире.', 6_000_000, ['hardcore']],
  ],
  resourcePack: [
    ['faithful-32x', 'Faithful 32x', 'Faithful Team', 'Классический стиль, вдвое выше разрешение.', 18_000_000, ['vanilla-like']],
    ['fresh-animations', 'Fresh Animations', 'FreshLX', 'Новые анимации мобов.', 12_000_000, ['entities']],
    ['bare-bones', 'Bare Bones', 'Cleanstone', 'Минималистичная палитра.', 9_000_000, ['simplistic']],
    ['stay-true', 'Stay True', 'Stay True Team', 'Ванильный стиль с мелкими улучшениями.', 5_000_000, ['vanilla-like']],
    ['xray-ultimate', 'Dokucraft', 'Docm77', 'Средневековая тема.', 3_000_000, ['themed']],
  ],
  shaderPack: [
    ['complementary-reimagined', 'Complementary Shaders - Reimagined', 'EminGT', 'Красивые и быстрые шейдеры.', 31_000_000, ['quality']],
    ['bsl', 'BSL Shaders', 'CaptTatsu', 'Реалистичное освещение и вода.', 27_000_000, ['realistic']],
    ['photon', 'Photon', 'sixthsurge', 'Современные шейдеры для Iris.', 3_000_000, ['quality']],
    ['miniature', 'Miniature Shader', 'Atomix', 'Лёгкий пиксельный стиль.', 1_800_000, ['low-end']],
  ],
  dataPack: [
    ['terralith', 'Terralith', 'Stardust Labs', 'Сотни новых биомов и ландшафтов для ванильной генерации.', 8_700_000, ['worldgen']],
    ['vanilla-tweaks', 'Vanilla Tweaks', 'Vanilla Tweaks', 'Небольшие улучшения игрового процесса.', 6_400_000, ['utility']],
    ['incendium', 'Incendium', 'Stardust Labs', 'Переработанный Незер с новыми структурами.', 3_900_000, ['worldgen', 'adventure']],
    ['armor-statues', 'Armor Statues', 'Rapha', 'Позы для стоек для брони.', 2_000_000, ['utility']],
  ],
}

function hitFor(platform: Platform, s: Seed): SearchHit {
  return {
    platform, projectId: `${platform === 'modrinth' ? 'mr' : 'cf'}-${s[0]}`, slug: s[0], title: s[1], author: s[2],
    description: s[3], iconUrl: null, downloads: platform === 'modrinth' ? s[4] : Math.round(s[4] * 0.8), categories: s[5],
  }
}

function searchContent(q: SearchQuery): SearchPage {
  const needle = q.query.trim().toLowerCase()
  const all = catalog[q.kind as CatalogKind]
    .map((s) => hitFor(q.platform, s))
    .filter((h) => !needle || h.title.toLowerCase().includes(needle) || h.description.toLowerCase().includes(needle))
  return { hits: all.slice(q.offset, q.offset + q.limit), offset: q.offset, totalHits: all.length }
}

function projectVersions(platform: Platform, projectId: string, filter: VersionFilter): ProjectVersion[] {
  const slug = projectId.replace(/^(mr|cf)-/, '')
  const versions: ProjectVersion[] = [
    ['1.4.2', ['1.21.4', '1.21.3'], 'release'], ['1.4.1', ['1.21.1'], 'release'], ['1.4.0-beta.2', ['1.21'], 'beta'],
    ['1.3.9', ['1.20.1', '1.20.4'], 'release'], ['1.3.0-alpha.1', ['1.19.4'], 'alpha'],
  ].map(([num, gv, releaseType], i) => {
    // CurseForge: the third version has third-party downloads disabled by the author
    const blocked = platform === 'curseForge' && i === 2
    const filename = `${slug}-${num as string}.jar`
    return {
      platform, id: `${projectId}-v${i}`, projectId, name: `${slug} ${num as string}`, versionNumber: num as string,
      releaseType: releaseType as ProjectVersion['releaseType'], gameVersions: gv as string[], loaders: (i === 1 ? ['forge'] : ['fabric', 'quilt']) as LoaderId[], published: daysAgo(10 + i * 45),
      dependencies: [],
      files: [{
        url: blocked ? null : `https://cdn.example/${filename}`,
        downloadPage: blocked ? `https://www.curseforge.com/minecraft/mc-mods/${slug}/files/${5_000_000 + i}` : null,
        filename, size: 1_200_000 + i * 90_000, hashes: { sha1: null, sha512: null }, primary: true,
      }],
    }
  })
  return versions.filter(
    (v) => (!filter.gameVersion || v.gameVersions.includes(filter.gameVersion)) && (!filter.loader || v.loaders.includes(filter.loader)),
  )
}

// ---- local resources ----
const localResources = new Map<string, LocalResource[]>()
function resourcesFor(instanceId: string, kind: LocalResourceKind): LocalResource[] {
  const key = `${instanceId}:${kind}`
  let list = localResources.get(key)
  if (!list) {
    const seeds = (catalog[kind as CatalogKind] ?? []).slice(0, instanceId === 'inst-2' || instanceId === 'inst-3' ? 7 : 2)
    list = seeds.map((s, i) => {
      const fileName = `${s[0]}-1.${i}.${i + 2}.${kind === 'mod' ? 'jar' : 'zip'}`
      return {
        fileName, enabled: i !== 3, size: 600_000 + i * 410_000,
        metadata: { id: s[0], name: s[1], version: `1.${i}.${i + 2}`, authors: [s[2]], description: s[3], icon: null, loader: null },
        origin: i % 3 === 2 ? null : { name: s[1], filename: fileName, update: { modrinth: { 'mod-id': `mr-${s[0]}`, version: `mr-${s[0]}-v${i}` } } },
      }
    })
    localResources.set(key, list)
  }
  return list
}

// ---- worlds, screenshots, servers ----
const worldsByInstance = new Map<string, World[]>()
function worldsFor(instanceId: string): World[] {
  let list = worldsByInstance.get(instanceId)
  if (!list) {
    list = instanceId === 'inst-5' ? [] : [
      { dirName: 'New World', name: 'Новый мир', gameMode: 'survival', hardcore: false, versionName: '1.21.1', lastPlayed: daysAgo(1), seed: '-4172144997902289642', icon: null, size: 84_000_000 },
      { dirName: 'Creative Build', name: 'Creative Build', gameMode: 'creative', hardcore: false, versionName: '1.20.1', lastPlayed: daysAgo(14), seed: '8675309', icon: null, size: 210_000_000 },
      { dirName: 'Hardcore', name: 'Hardcore', gameMode: 'survival', hardcore: true, versionName: '1.19.4', lastPlayed: daysAgo(40), seed: null, icon: null, size: 52_000_000 },
      { dirName: 'broken-save', name: 'broken-save', gameMode: null, hardcore: null, versionName: null, lastPlayed: daysAgo(90), seed: null, icon: null, size: 1_200_000 },
    ]
    worldsByInstance.set(instanceId, list)
  }
  return list
}
const findWorld = (instanceId: string, dirName: unknown) => {
  const world = worldsFor(instanceId).find((w) => w.dirName === dirName)
  if (!world) throw new Error(`world ${String(dirName)} does not exist`)
  return world
}
const datapacks = new Map<string, LocalResource[]>()
function datapacksFor(instanceId: string, world: string): LocalResource[] {
  const key = `${instanceId}:${world}`
  let list = datapacks.get(key)
  if (!list) {
    list = catalog.dataPack.slice(0, 2).map((seed, i) => {
      const fileName = `${seed[0]}-1.${i}.zip`
      return {
        fileName, enabled: true, size: 300_000 + i * 2_000_000,
        metadata: { id: seed[0], name: seed[1], version: `1.${i}`, authors: [seed[2]], description: seed[3], icon: null, loader: null },
        origin: i === 0 ? { name: seed[1], filename: fileName, update: { modrinth: { 'mod-id': `mr-${seed[0]}`, version: `mr-${seed[0]}-v${i}` } } } : null,
      }
    })
    datapacks.set(key, list)
  }
  return list
}
const uniqueName = (taken: string[], name: string) => {
  let candidate = name
  for (let n = 1; taken.includes(candidate); n++) candidate = `${name} (${n})`
  return candidate
}
const fileNameOf = (path: unknown, fallback: string) => String(path).split(/[\\/]/).pop() || fallback
/** The real backend refuses to edit saves and servers.dat under a running game. */
const notRunning = (instanceId: unknown) => {
  if ([...sessions.values()].some((s) => s.instanceId === instanceId && s.state !== 'stopped')) throw new Error('the instance is running; stop the game first')
}

const shotColors = ['#3b82f6', '#22c55e', '#f59e0b', '#ef4444', '#a855f7', '#14b8a6', '#ec4899', '#84cc16']
const screenshotsByInstance = new Map<string, Screenshot[]>()
function screenshotsFor(instanceId: string): Screenshot[] {
  let list = screenshotsByInstance.get(instanceId)
  if (!list) {
    list = instanceId === 'inst-5' ? [] : shotColors.map((color, i) => {
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="${color}"/><stop offset="1" stop-color="#0f172a"/></linearGradient></defs><rect width="640" height="360" fill="url(#g)"/><text x="24" y="52" font-family="monospace" font-size="28" fill="#fff">screenshot ${i + 1}</text></svg>`
      const stamp = new Date(Date.now() - i * 5_400_000)
      const two = (n: number) => String(n).padStart(2, '0')
      const fileName = `${stamp.getFullYear()}-${two(stamp.getMonth() + 1)}-${two(stamp.getDate())}_${two(stamp.getHours())}.${two(stamp.getMinutes())}.${two(stamp.getSeconds())}.png`
      return { fileName, path: `data:image/svg+xml;utf8,${encodeURIComponent(svg)}`, size: 1_400_000 + i * 120_000, modified: stamp.getTime() }
    })
    screenshotsByInstance.set(instanceId, list)
  }
  return list
}

const serversByInstance = new Map<string, Server[]>()
function serversFor(instanceId: string): Server[] {
  let list = serversByInstance.get(instanceId)
  if (!list) {
    list = [
      { name: 'Hypixel', address: 'mc.hypixel.net', resourcePack: 'prompt' },
      { name: 'Мой сервер', address: '192.168.1.20:25566', resourcePack: 'accept' },
      { name: 'Офлайн сервер', address: 'offline.example.com', resourcePack: 'decline' },
    ]
    serversByInstance.set(instanceId, list)
  }
  return list
}
const serverAt = (instanceId: unknown, index: unknown) => {
  const server = serversFor(instanceId as string)[index as number]
  if (!server) throw new Error(`server ${String(index)} does not exist`)
  return server
}
async function pingServer(address: string): Promise<ServerStatus> {
  await sleep(300 + (address.length % 5) * 250)
  if (address.includes('offline')) throw new Error(`${address} did not answer within 5 s`)
  const local = /^\d/.test(address)
  return {
    motd: local ? 'Приватный сервер друзей' : 'Hypixel Network [1.8-1.21]  SKYBLOCK 0.21',
    version: local ? 'Paper 1.21.1' : 'Hypixel',
    protocol: 767, online: local ? 3 : 41_837, max: local ? 20 : 200_000, latencyMs: local ? 4 : 63 + (address.length % 40), favicon: null,
  }
}

let dataDir = '%APPDATA%\\_k0t1k'

// ---- launching ----
// Every launch is a session of its own, like in the backend: any number may run per instance.
const KEPT_FINISHED_SESSIONS = 3
const sessions = new Map<SessionId, RunSession>()
const sessionLogs = new Map<SessionId, GameLogLine[]>()
const killFlags = new Set<SessionId>()
const launches = new Map<string, number>()
let nextSession = 1
const live = (sessionId: SessionId) => sessions.get(sessionId)?.state === 'running'
const setRun = (sessionId: SessionId, state: RunState, exitCode?: number) => {
  const session = sessions.get(sessionId)!
  session.state = state
  emit('instance-state', { sessionId, instanceId: session.instanceId, number: session.number, state, exitCode } satisfies InstanceRunState)
}
let logSeq = 0
const logBatch = (sessionId: SessionId, lines: Omit<GameLogLine, 'seq'>[]) => {
  const numbered = lines.map((l) => ({ ...l, seq: logSeq++ }))
  const kept = sessionLogs.get(sessionId)!
  kept.push(...numbered)
  kept.splice(0, Math.max(0, kept.length - 5000))
  emit('game-log', { sessionId, lines: numbered } satisfies GameLogBatch)
}
const log = (sessionId: SessionId, stream: GameLogLine['stream'], line: string) => logBatch(sessionId, [{ stream, line }])

/** A chatty modpack: 20k lines in batches, to exercise the log cap and batching. */
async function logFlood(sessionId: SessionId) {
  const total = 20_000
  const batch = 400
  for (let n = 0; n < total && !killFlags.has(sessionId); n += batch) {
    logBatch(sessionId, Array.from({ length: batch }, (_, i) => ({
      stream: 'stdout' as const,
      line: `[Worker-Main-${i % 8}/INFO]: Loaded recipe #${n + i} of ${total}`,
    })))
    await sleep(8)
  }
}

/** Ends a session the way the backend does: `stopped` event, then the oldest finished logs of the instance go. */
function finish(sessionId: SessionId, exitCode: number) {
  const session = sessions.get(sessionId)!
  const inst = findInstance(session.instanceId)
  inst.lastPlayed = new Date().toISOString()
  inst.playtime = { totalSecs: inst.playtime.totalSecs + 120, lastSecs: 120 }
  log(sessionId, 'launcher', `Процесс игры завершён с кодом ${exitCode}`)
  setRun(sessionId, 'stopped', exitCode)
  const finished = [...sessions.values()].filter((s) => s.instanceId === session.instanceId && s.state === 'stopped')
  for (const old of finished.slice(0, Math.max(0, finished.length - KEPT_FINISHED_SESSIONS))) {
    sessions.delete(old.sessionId)
    sessionLogs.delete(old.sessionId)
  }
}

function launch(instanceId: string): SessionId {
  const inst = findInstance(instanceId)
  const sessionId = nextSession++
  const number = (launches.get(instanceId) ?? 0) + 1
  launches.set(instanceId, number)
  sessions.set(sessionId, { sessionId, instanceId, number, state: 'preparing', startedAt: new Date().toISOString() })
  sessionLogs.set(sessionId, [])
  setRun(sessionId, 'preparing')
  log(sessionId, 'launcher', `Запуск ${inst.name}...`)
  void (async () => {
    await fakeTask(`launch-${sessionId}`, `Подготовка ${inst.name}`, ['Метаданные', 'Библиотеки', 'Ассеты', 'Java'], 400)
    if (killFlags.has(sessionId)) return
    setRun(sessionId, 'running')
    const tag = () => new Date().toTimeString().slice(0, 8)
    const lines: [GameLogLine['stream'], string][] = [
      ['stdout', `[${tag()}] [main/INFO]: Loading Minecraft ${inst.components[0]?.version ?? ''}`],
      ['stdout', `[${tag()}] [main/INFO]: Setting user: ${settings.accounts[0]?.name ?? 'Player'}`],
      ['stdout', `[${tag()}] [Render thread/INFO]: Backend library: LWJGL version 3.3.3`],
      ['stdout', `[${tag()}] [main/WARN]: Ambiguity between arguments [--version] and [--gameDir]`],
      ['stdout', `[${tag()}] [Worker-Main-1/INFO]: Reloading ResourceManager`],
      ['stderr', `[${tag()}] [Render thread/ERROR]: Failed to load sound event minecraft:ambient.cave`],
      ['stdout', `[${tag()}] [Server thread/DEBUG]: Preparing spawn area: 72%`],
      ['stdout', `[${tag()}] [Server thread/INFO]: Done (3.412s)! For help, type "help"`],
    ]
    for (const [stream, line] of lines) {
      if (killFlags.has(sessionId)) return
      log(sessionId, stream, line)
      await sleep(500)
    }
    if (instanceId === 'inst-3') void logFlood(sessionId)
    let n = 0
    while (!killFlags.has(sessionId) && live(sessionId)) {
      log(sessionId, 'stdout', `[${tag()}] [Server thread/INFO]: Autosave tick ${++n}`)
      await sleep(2500)
    }
  })()
  return sessionId
}

async function killSession(sessionId: SessionId) {
  const session = sessions.get(sessionId)
  if (!session || session.state === 'stopped') throw new Error('the session is not running')
  killFlags.add(sessionId)
  setRun(sessionId, 'stopping')
  await sleep(600)
  finish(sessionId, 0)
}

async function killInstance(instanceId: string) {
  const liveSessions = [...sessions.values()].filter((s) => s.instanceId === instanceId && s.state !== 'stopped')
  if (!liveSessions.length) throw new Error('the instance is not running')
  await Promise.all(liveSessions.map((s) => killSession(s.sessionId)))
}

// ---- command table ----
async function invoke(cmd: string, a: Args = {}): Promise<unknown> {
  await sleep(60)
  switch (cmd) {
    case 'list_instances': return clone({ instances, errors: [] })
    case 'create_instance': {
      const r = a.request as CreateInstanceRequest
      const inst = newInstance({ id: nextId('inst'), name: r.name, icon: r.icon, group: r.group || null, components: components(r.minecraftVersion, r.loader) })
      await fakeTask(`create-${inst.id}`, `Создание ${inst.name}`, ['Метаданные Minecraft', 'Метаданные загрузчика', 'Подготовка папки'], 300)
      instances.push(inst)
      return clone(inst)
    }
    case 'rename_instance': { const i = findInstance(a.instanceId); i.name = a.name as string; return clone(i) }
    case 'delete_instance':
      if ([...sessions.values()].some((s) => s.instanceId === a.instanceId && s.state !== 'stopped')) {
        throw new Error('the instance is running; stop all its sessions before deleting')
      }
      instances.splice(instances.indexOf(findInstance(a.instanceId)), 1); return undefined
    case 'copy_instance': {
      const src = findInstance(a.instanceId)
      const copy: Instance = { ...clone(src), id: nextId('inst'), name: a.name as string, created: new Date().toISOString(), lastPlayed: null, playtime: { totalSecs: 0, lastSecs: 0 } }
      instances.push(copy)
      return clone(copy)
    }
    case 'set_instance_group': { const i = findInstance(a.instanceId); i.group = (a.group as string | null) || null; return clone(i) }
    case 'update_instance_settings': {
      const i = findInstance(a.instanceId)
      const { icon, notes, sharedFolders, settings: groups } = clone(a.settings as InstanceSettingsUpdate)
      Object.assign(i, { icon, notes, sharedFolders, settings: groups })
      return clone(i)
    }
    case 'open_instance_folder': return undefined
    case 'launch_instance': return launch(a.instanceId as string)
    case 'kill_session': await killSession(a.sessionId as SessionId); return undefined
    case 'kill_instance': await killInstance(a.instanceId as string); return undefined
    case 'list_run_states': return clone([...sessions.values()])
    case 'get_game_logs': return clone(sessionLogs.get(a.sessionId as SessionId) ?? [])
    case 'list_minecraft_versions': return minecraftVersions
    case 'list_loader_versions': { await sleep(250); return loaderVersions(a.minecraftVersion as string, a.loader as LoaderId) }
    case 'detect_java': await sleep(400); return javaInstalls
    case 'list_managed_java': return managedJava
    case 'get_settings': return clone(settings)
    case 'save_settings': settings = clone(a.settings as GlobalSettings); persistSettings(); return undefined
    case 'get_system_info':
      return { totalRamMib: 16_384, os: 'Windows 11 (мок)', appVersion: '0.1.0', dataDir, portable: false, exeDir: 'C:\\Program Files\\_k0t1k' } satisfies SystemInfo
    case 'set_data_dir': dataDir = (a.path as string | null) ?? '%APPDATA%\\_k0t1k'; return true
    case 'open_data_dir': return undefined
    case 'open_shared_folder': return undefined
    case 'add_offline_account': {
      const acc: Account = { id: nextId('acc'), name: a.name as string, kind: 'offline' }
      settings.accounts.push(acc)
      settings.selectedAccountId ??= acc.id
      persistSettings()
      return acc
    }
    case 'remove_offline_account':
      settings.accounts = settings.accounts.filter((x) => x.id !== a.accountId)
      if (settings.selectedAccountId === a.accountId) settings.selectedAccountId = settings.accounts[0]?.id ?? null
      persistSettings()
      return undefined
    case 'select_account': settings.selectedAccountId = a.accountId as string; persistSettings(); return undefined
    case 'search_content': await sleep(250); return searchContent(a.query as SearchQuery)
    case 'get_project_versions': await sleep(250); return projectVersions(a.platform as Platform, a.projectId as string, a.filter as VersionFilter)
    case 'install_version': {
      const v = projectVersions(a.platform as Platform, a.projectId as string, {}).find((x) => x.id === a.versionId)
      const file = v?.files.find((f) => f.primary) ?? v?.files[0]
      if (!v || !file) throw new Error('У версии нет файлов')
      if (file.url === null) {
        const blocked: BlockedFile = {
          platform: v.platform, projectId: v.projectId, name: v.name, fileName: file.filename,
          targetPath: `mods/${file.filename}`, downloadPage: file.downloadPage, sha1: null,
        }
        return { installed: [], blocked: [blocked], unresolvedDependencies: [] } satisfies InstallReport
      }
      await fakeTask(`install-${v.id}`, `Установка ${v.name}`, ['Скачивание', 'Проверка хеша'], 450)
      resourcesFor(a.instanceId as string, a.kind as LocalResourceKind).unshift({
        fileName: file.filename, enabled: true, size: file.size,
        metadata: { id: v.projectId, name: v.name, version: v.versionNumber, authors: [], description: null, icon: null, loader: null },
        origin: { name: v.name, filename: file.filename, update: { modrinth: { 'mod-id': v.projectId, version: v.id } } },
      })
      return {
        installed: [{ platform: v.platform, projectId: v.projectId, name: v.name, fileName: file.filename, isDependency: false }],
        blocked: [], unresolvedDependencies: [],
      } satisfies InstallReport
    }
    case 'list_local_resources': return clone(resourcesFor(a.instanceId as string, a.kind as LocalResourceKind))
    case 'toggle_resource': {
      const r = resourcesFor(a.instanceId as string, a.kind as LocalResourceKind).find((x) => x.fileName === a.fileName)
      if (r) r.enabled = a.enabled as boolean
      return undefined
    }
    case 'delete_resource': {
      const list = resourcesFor(a.instanceId as string, a.kind as LocalResourceKind)
      const idx = list.findIndex((x) => x.fileName === a.fileName)
      if (idx >= 0) list.splice(idx, 1)
      return undefined
    }
    case 'check_updates': {
      await sleep(700)
      return resourcesFor(a.instanceId as string, a.kind as LocalResourceKind)
        .filter((r) => r.origin?.update.modrinth)
        .slice(0, 2)
        .map((r): AvailableUpdate => ({
          fileName: r.fileName, name: r.metadata?.name ?? r.fileName, currentVersion: r.metadata?.version ?? '',
          latest: projectVersions('modrinth', r.origin!.update.modrinth!['mod-id'], {})[0]!,
        }))
    }
    case 'list_worlds': return clone(worldsFor(a.instanceId as string))
    case 'rename_world': {
      notRunning(a.instanceId)
      const world = findWorld(a.instanceId as string, a.dirName)
      world.name = (a.name as string).trim()
      if (a.renameFolder) world.dirName = uniqueName(worldsFor(a.instanceId as string).filter((w) => w !== world).map((w) => w.dirName), world.name)
      return world.dirName
    }
    case 'copy_world': {
      notRunning(a.instanceId)
      const list = worldsFor(a.instanceId as string)
      const source = findWorld(a.instanceId as string, a.dirName)
      const name = (a.name as string).trim()
      const copy: World = { ...clone(source), name, dirName: uniqueName(list.map((w) => w.dirName), name) }
      list.unshift(copy)
      return copy.dirName
    }
    case 'delete_world': {
      notRunning(a.instanceId)
      const list = worldsFor(a.instanceId as string)
      list.splice(list.indexOf(findWorld(a.instanceId as string, a.dirName)), 1)
      return undefined
    }
    case 'backup_world': {
      notRunning(a.instanceId)
      await sleep(500)
      return (a.destination as string | undefined) ?? `C:\\_k0t1k\\instances\\${String(a.instanceId)}\\backups\\${String(a.dirName)} 2026-01-01_12-00-00.zip`
    }
    case 'open_world_folder': return undefined
    case 'import_world': {
      notRunning(a.instanceId)
      const list = worldsFor(a.instanceId as string)
      const base = fileNameOf(a.path, 'World').replace(/\.zip$/i, '')
      const world: World = { dirName: uniqueName(list.map((w) => w.dirName), base), name: base, gameMode: 'survival', hardcore: false, versionName: '1.21.1', lastPlayed: new Date().toISOString(), seed: '12345', icon: null, size: 9_000_000 }
      list.unshift(world)
      return world.dirName
    }
    case 'list_datapacks': return clone(datapacksFor(a.instanceId as string, a.world as string))
    case 'toggle_datapack': {
      notRunning(a.instanceId)
      const pack = datapacksFor(a.instanceId as string, a.world as string).find((x) => x.fileName === a.fileName)
      if (pack) pack.enabled = a.enabled as boolean
      return undefined
    }
    case 'delete_datapack': {
      notRunning(a.instanceId)
      const list = datapacksFor(a.instanceId as string, a.world as string)
      const idx = list.findIndex((x) => x.fileName === a.fileName)
      if (idx >= 0) list.splice(idx, 1)
      return undefined
    }
    case 'add_datapack': {
      notRunning(a.instanceId)
      const list = datapacksFor(a.instanceId as string, a.world as string)
      const fileName = uniqueName(list.map((x) => x.fileName), fileNameOf(a.path, 'pack.zip'))
      list.unshift({ fileName, enabled: true, size: 120_000, metadata: null, origin: null })
      return fileName
    }
    case 'install_datapack': {
      notRunning(a.instanceId)
      const v = projectVersions(a.platform as Platform, a.projectId as string, {}).find((x) => x.id === a.versionId)
      if (!v) throw new Error('У версии нет файлов')
      const fileName = v.files[0]!.filename.replace(/\.jar$/, '.zip')
      await fakeTask(`datapack-${v.id}`, `Установка ${v.name}`, ['Скачивание', 'Проверка хеша'], 450)
      datapacksFor(a.instanceId as string, a.world as string).unshift({
        fileName, enabled: true, size: v.files[0]!.size,
        metadata: { id: v.projectId, name: v.name, version: v.versionNumber, authors: [], description: null, icon: null, loader: null },
        origin: { name: v.name, filename: fileName, update: { modrinth: { 'mod-id': v.projectId, version: v.id } } },
      })
      return { installed: [{ platform: v.platform, projectId: v.projectId, name: v.name, fileName, isDependency: false }], blocked: [], unresolvedDependencies: [] } satisfies InstallReport
    }
    case 'list_screenshots': return clone(screenshotsFor(a.instanceId as string))
    case 'open_screenshot':
    case 'open_screenshots_folder': return undefined
    case 'delete_screenshot': {
      const list = screenshotsFor(a.instanceId as string)
      const idx = list.findIndex((x) => x.fileName === a.fileName)
      if (idx >= 0) list.splice(idx, 1)
      return undefined
    }
    case 'list_servers': return clone(serversFor(a.instanceId as string))
    case 'add_server': notRunning(a.instanceId); serversFor(a.instanceId as string).push(clone(a.server as Server)); return undefined
    case 'update_server': notRunning(a.instanceId); Object.assign(serverAt(a.instanceId, a.index), clone(a.server as Server)); return undefined
    case 'remove_server': notRunning(a.instanceId); serversFor(a.instanceId as string).splice(a.index as number, 1); return undefined
    case 'move_server': {
      notRunning(a.instanceId)
      const list = serversFor(a.instanceId as string)
      const [moved] = list.splice(a.from as number, 1)
      if (!moved) throw new Error(`server ${String(a.from)} does not exist`)
      list.splice(a.to as number, 0, moved)
      return undefined
    }
    case 'ping_server': return pingServer(a.address as string)
    case 'import_modpack_file': {
      const path = a.path as string
      const inst = newInstance({
        id: nextId('inst'), name: (a.name as string | undefined) || (path.split(/[\\/]/).pop() ?? 'Модпак').replace(/\.(mrpack|zip)$/i, ''),
        icon: 'gem', group: 'Модпаки', components: components('1.20.1', { id: 'fabric', version: '0.16.10' }),
      })
      await fakeTask(`import-${inst.id}`, `Импорт ${inst.name}`, ['Чтение манифеста', 'Скачивание модов', 'Применение overrides'], 450)
      instances.push(inst)
      return clone({ instance: inst, blocked: [] } satisfies ModpackImport)
    }
    case 'install_modpack_version': {
      const v = projectVersions(a.platform as Platform, a.projectId as string, {})[0]!
      const inst = newInstance({
        id: nextId('inst'), name: (a.name as string | undefined) || v.name, icon: 'gem', group: 'Модпаки',
        components: components(v.gameVersions[0] ?? '1.20.1', { id: 'fabric', version: '0.16.10' }),
        managedPack: { platform: v.platform === 'modrinth' ? 'modrinth' : 'curseforge', projectId: v.projectId, versionId: v.id, name: v.name, versionName: v.versionNumber },
      })
      await fakeTask(`pack-${inst.id}`, `Установка ${v.name}`, ['Скачивание пакета', 'Скачивание модов', 'Применение overrides'], 450)
      instances.push(inst)
      const blocked: BlockedFile[] = v.platform === 'curseForge'
        ? [{ platform: v.platform, projectId: '306612', name: 'Blocked Example Mod', fileName: 'blocked-example.jar', targetPath: 'mods/blocked-example.jar',
            downloadPage: 'https://www.curseforge.com/minecraft/mc-mods/blocked-example/files/1', sha1: null }]
        : []
      return clone({ instance: inst, blocked } satisfies ModpackImport)
    }
    default: throw new Error(`mock: неизвестная команда ${cmd}`)
  }
}

export const mockBackend = {
  invoke,
  listen: async (event: string, cb: (payload: unknown) => void) => {
    let set = listeners.get(event)
    if (!set) listeners.set(event, (set = new Set()))
    set.add(cb)
    return () => void set.delete(cb)
  },
}
