// What the options tab knows about options.txt. The file differs between versions (1.8 stores
// `fancyGraphics:true`, 1.16 `graphicsMode:1`, 1.21.11 `graphicsPreset:"fancy"`), so a field is shown
// and a preset value written only when the instance's file already has the key in the same shape.
import type { GameOption } from '../api/types'
import { t } from '../i18n'

type Dict = typeof t.gopt
type FieldKey = keyof Dict['fields']
export type Section = keyof Dict['sections']
type Choice = { value: string; label: () => string }

export type Field =
  | { key: FieldKey; section: Section; kind: 'bool' }
  | { key: FieldKey; section: Section; kind: 'choice'; choices: Choice[] }
  | {
      key: FieldKey
      section: Section
      kind: 'range'
      /** Range and step of the shown value; `toShown`/`fromShown` convert from and to the stored number. */
      min: number
      max: number
      step: number
      toShown?: (raw: number) => number
      fromShown?: (shown: number) => number
      format: (shown: number) => string
    }

const choice = (value: string, label: () => string): Choice => ({ value, label })
const percent = (shown: number) => `${Math.round(shown)}%`
const unit = { toShown: (raw: number) => raw * 100, fromShown: (shown: number) => shown / 100, format: percent }
const off = (format: (n: number) => string) => (n: number) => (n === 0 ? t.gopt.off : format(n))
const graphics = () => [
  choice('fast', () => t.gopt.choices.fast),
  choice('fancy', () => t.gopt.choices.fancy),
  choice('fabulous', () => t.gopt.choices.fabulous),
]

export const FIELDS: Field[] = [
  { key: 'graphicsPreset', section: 'graphics', kind: 'choice', choices: [...graphics(), choice('custom', () => t.gopt.choices.custom)] },
  {
    key: 'graphicsMode',
    section: 'graphics',
    kind: 'choice',
    choices: graphics().map((c, i) => choice(String(i), c.label)),
  },
  { key: 'fancyGraphics', section: 'graphics', kind: 'bool' },
  { key: 'renderDistance', section: 'graphics', kind: 'range', min: 2, max: 32, step: 1, format: (n) => t.gopt.chunks(n) },
  { key: 'simulationDistance', section: 'graphics', kind: 'range', min: 5, max: 32, step: 1, format: (n) => t.gopt.chunks(n) },
  { key: 'maxFps', section: 'graphics', kind: 'range', min: 10, max: 260, step: 10, format: (n) => (n >= 260 ? t.gopt.unlimited : String(n)) },
  { key: 'enableVsync', section: 'graphics', kind: 'bool' },
  { key: 'fullscreen', section: 'graphics', kind: 'bool' },
  { key: 'guiScale', section: 'graphics', kind: 'range', min: 0, max: 6, step: 1, format: (n) => (n === 0 ? t.gopt.auto : String(n)) },
  // Stored as -1..1 around the default 70°.
  { key: 'fov', section: 'graphics', kind: 'range', min: 30, max: 110, step: 1, toShown: (raw) => 70 + raw * 40, fromShown: (deg) => (deg - 70) / 40, format: (n) => `${Math.round(n)}°` },
  { key: 'gamma', section: 'graphics', kind: 'range', min: 0, max: 100, step: 1, ...unit },
  {
    key: 'renderClouds',
    section: 'graphics',
    kind: 'choice',
    choices: [choice('true', () => t.gopt.choices.fancy), choice('fast', () => t.gopt.choices.fast), choice('false', () => t.gopt.off)],
  },
  {
    key: 'particles',
    section: 'graphics',
    kind: 'choice',
    choices: [choice('0', () => t.gopt.choices.all), choice('1', () => t.gopt.choices.decreased), choice('2', () => t.gopt.choices.minimal)],
  },
  { key: 'ao', section: 'graphics', kind: 'bool' },
  // Before 1.14 smooth lighting has levels instead of on/off.
  {
    key: 'ao',
    section: 'graphics',
    kind: 'choice',
    choices: [choice('0', () => t.gopt.off), choice('1', () => t.gopt.choices.minimal), choice('2', () => t.gopt.choices.max)],
  },
  { key: 'entityShadows', section: 'graphics', kind: 'bool' },
  { key: 'biomeBlendRadius', section: 'graphics', kind: 'range', min: 0, max: 7, step: 1, format: off((n) => `${2 * n + 1}×${2 * n + 1}`) },
  { key: 'mipmapLevels', section: 'graphics', kind: 'range', min: 0, max: 4, step: 1, format: off(String) },
  { key: 'entityDistanceScaling', section: 'graphics', kind: 'range', min: 50, max: 500, step: 25, ...unit },
  {
    key: 'prioritizeChunkUpdates',
    section: 'graphics',
    kind: 'choice',
    choices: [
      choice('0', () => t.gopt.choices.threaded),
      choice('1', () => t.gopt.choices.semiBlocking),
      choice('2', () => t.gopt.choices.fullyBlocking),
    ],
  },
  {
    key: 'inactivityFpsLimit',
    section: 'graphics',
    kind: 'choice',
    choices: [choice('minimized', () => t.gopt.choices.minimized), choice('afk', () => t.gopt.choices.afk)],
  },
  { key: 'bobView', section: 'graphics', kind: 'bool' },
  { key: 'fovEffectScale', section: 'graphics', kind: 'range', min: 0, max: 100, step: 1, ...unit },
  { key: 'screenEffectScale', section: 'graphics', kind: 'range', min: 0, max: 100, step: 1, ...unit },
  { key: 'damageTiltStrength', section: 'graphics', kind: 'range', min: 0, max: 100, step: 1, ...unit },

  // The game shows the stored 0..1 as 0..200%.
  { key: 'mouseSensitivity', section: 'controls', kind: 'range', min: 0, max: 200, step: 1, toShown: (raw) => raw * 200, fromShown: (shown) => shown / 200, format: percent },
  { key: 'rawMouseInput', section: 'controls', kind: 'bool' },
  { key: 'invertYMouse', section: 'controls', kind: 'bool' },
  { key: 'autoJump', section: 'controls', kind: 'bool' },
  { key: 'toggleCrouch', section: 'controls', kind: 'bool' },
  { key: 'toggleSprint', section: 'controls', kind: 'bool' },

  { key: 'soundCategory_master', section: 'sound', kind: 'range', min: 0, max: 100, step: 1, ...unit },
  { key: 'soundCategory_music', section: 'sound', kind: 'range', min: 0, max: 100, step: 1, ...unit },
  { key: 'showSubtitles', section: 'sound', kind: 'bool' },

  {
    key: 'lang',
    section: 'interface',
    kind: 'choice',
    choices: [choice('ru_ru', () => 'Русский'), choice('en_us', () => 'English (US)'), choice('uk_ua', () => 'Українська')],
  },
  { key: 'chatScale', section: 'interface', kind: 'range', min: 0, max: 100, step: 1, ...unit },
  { key: 'chatOpacity', section: 'interface', kind: 'range', min: 10, max: 100, step: 1, ...unit },
  {
    key: 'attackIndicator',
    section: 'interface',
    kind: 'choice',
    choices: [choice('0', () => t.gopt.off), choice('1', () => t.gopt.choices.crosshair), choice('2', () => t.gopt.choices.hotbar)],
  },
  {
    key: 'narrator',
    section: 'interface',
    kind: 'choice',
    choices: [
      choice('0', () => t.gopt.off),
      choice('1', () => t.gopt.choices.narratorAll),
      choice('2', () => t.gopt.choices.narratorChat),
      choice('3', () => t.gopt.choices.narratorSystem),
    ],
  },
  { key: 'pauseOnLostFocus', section: 'interface', kind: 'bool' },
  { key: 'advancedItemTooltips', section: 'interface', kind: 'bool' },
  { key: 'hideLightningFlashes', section: 'interface', kind: 'bool' },
]

export type Shape = 'bool' | 'number' | 'string'
/** The kind of a stored value: newer versions quote enum strings, so `"fast"` and `fast` are both strings. */
export function shapeOf(raw: string): Shape {
  if (raw === 'true' || raw === 'false') return 'bool'
  if (raw.trim() !== '' && Number.isFinite(Number(raw))) return 'number'
  return 'string'
}
export const unquote = (raw: string) => (raw.length >= 2 && raw.startsWith('"') && raw.endsWith('"') ? raw.slice(1, -1) : raw)
/** `bare` stored the way `existing` is: quoted when the file quotes its strings. */
export const requote = (bare: string, existing: string) => (existing.startsWith('"') ? `"${bare}"` : bare)

/** Whether `field` can edit the stored value `raw`. */
export function fits(field: Field, raw: string): boolean {
  const shape = shapeOf(raw)
  if (field.kind === 'bool') return shape === 'bool'
  if (field.kind === 'range') return shape === 'number'
  // a number-keyed choice (0, 1, 2) must not take `ao:true`, nor a string choice a number
  const numeric = field.choices.every((c) => shapeOf(c.value) === 'number')
  return numeric ? shape === 'number' : shape !== 'number'
}

/** A stored value as people read it. */
export function display(key: string, raw: string): string {
  const field = FIELDS.find((f) => f.key === key && fits(f, raw))
  if (!field) return raw
  if (field.kind === 'bool') return raw === 'true' ? '✓' : '✗'
  if (field.kind === 'choice') return field.choices.find((c) => c.value === unquote(raw))?.label() ?? unquote(raw)
  const n = Number(raw)
  return field.format(field.toShown ? field.toShown(n) : n)
}

export type PresetId = keyof Dict['presetNames']
/** Values per key. A list holds alternatives for the shapes different versions store (the first that fits wins). */
type PresetValues = Record<string, string | string[]>
const fast = { graphicsPreset: 'fast', graphicsMode: '0', fancyGraphics: 'false' }
const fancy = { graphicsPreset: 'fancy', graphicsMode: '1', fancyGraphics: 'true' }

export const PRESETS: Record<PresetId, PresetValues> = {
  fps: {
    ...fast,
    renderDistance: '8',
    simulationDistance: '6',
    maxFps: '260',
    enableVsync: 'false',
    renderClouds: 'false',
    particles: '2',
    ao: ['false', '0'],
    entityShadows: 'false',
    biomeBlendRadius: '0',
    mipmapLevels: '0',
    entityDistanceScaling: '0.75',
    prioritizeChunkUpdates: '0',
    improvedTransparency: 'false',
  },
  balanced: {
    ...fancy,
    renderDistance: '12',
    simulationDistance: '8',
    maxFps: '144',
    enableVsync: 'false',
    renderClouds: 'fast',
    particles: '1',
    ao: ['true', '2'],
    entityShadows: 'true',
    biomeBlendRadius: '2',
    mipmapLevels: '4',
    entityDistanceScaling: '1.0',
  },
  quality: {
    ...fancy,
    renderDistance: '20',
    simulationDistance: '12',
    maxFps: '260',
    enableVsync: 'true',
    renderClouds: 'true',
    particles: '0',
    ao: ['true', '2'],
    entityShadows: 'true',
    biomeBlendRadius: '5',
    mipmapLevels: '4',
    entityDistanceScaling: '1.5',
  },
  pvp: {
    fov: '0.5',
    fovEffectScale: '0.0',
    bobView: 'false',
    damageTiltStrength: '0.0',
    screenEffectScale: '0.0',
    particles: '1',
    maxFps: '260',
    enableVsync: 'false',
    rawMouseInput: 'true',
    autoJump: 'false',
  },
}

export interface Change {
  key: string
  from: string | null
  to: string
}

/** The preset values that differ from `current`, written in the shape and quoting the file uses. */
export function presetChanges(preset: PresetValues, current: GameOption[]): Change[] {
  const byKey = new Map(current.map((o) => [o.key, o.value]))
  const changes: Change[] = []
  for (const [key, wanted] of Object.entries(preset)) {
    const existing = byKey.get(key)
    if (existing === undefined) continue
    const alternatives = Array.isArray(wanted) ? wanted : [wanted]
    // An enum keeps the file's quoting (`renderClouds:"fast"` from 1.14, `renderClouds:fast` before).
    const field = FIELDS.find((f) => f.key === key && fits(f, existing))
    const to =
      field?.kind === 'choice'
        ? alternatives.filter((v) => field.choices.some((c) => c.value === v)).map((v) => requote(v, existing))[0]
        : alternatives.find((v) => shapeOf(v) === shapeOf(existing))
    if (to === undefined) continue
    if (to !== existing && !(shapeOf(to) === 'number' && Number(to) === Number(existing))) changes.push({ key, from: existing, to })
  }
  return changes
}

/** Bookkeeping of the source game rather than settings: its data version, packs, last server, tutorial state. */
const NOT_COPIED = new Set([
  'version',
  'resourcePacks',
  'incompatibleResourcePacks',
  'lastServer',
  'tutorialStep',
  'joinedFirstServer',
  'skipMultiplayerWarning',
  'onboardAccessibility',
  'startedCleanly',
  'syncChunkWrites',
])

/** Settings of another instance to copy. Without a file of its own, the instance gets the source file whole. */
export function copyChanges(source: GameOption[], current: GameOption[] | null): Change[] {
  if (!current) return source.map((o) => ({ key: o.key, from: null, to: o.value }))
  const byKey = new Map(current.map((o) => [o.key, o.value]))
  return source.flatMap((o) => {
    const existing = byKey.get(o.key)
    if (existing === undefined || existing === o.value || NOT_COPIED.has(o.key)) return []
    // a key that changed shape between versions keeps the target's own value
    if (shapeOf(existing) !== shapeOf(o.value) || existing.startsWith('"') !== o.value.startsWith('"')) return []
    return [{ key: o.key, from: existing, to: o.value }]
  })
}
