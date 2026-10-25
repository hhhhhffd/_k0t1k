import { t } from '../i18n'
import type { CustomTheme, ThemeId } from '../api/types'

/** Labels are getters so they follow the language when read while rendering. */
export const THEMES: { id: ThemeId; label: string }[] = (['k0t1k', 'light', 'amoled', 'mint'] as const).map((id) => ({
  id,
  get label() {
    return t.themeNames[id]
  },
}))

/** The colors a custom theme edits: CSS variable name (without `--`) and its label. */
export const THEME_COLORS = (['c-base', 'c-surface', 'c-fg', 'c-muted', 'c-accent', 'c-line', 'c-danger', 'c-ok'] as const).map((key) => ({
  key,
  get label() {
    return t.themeColors[key]
  },
}))

/** Same rule as the backend applies to every stored value. */
const COLOR_VALUE = /^[#a-zA-Z0-9(),.%\s-]{1,64}$/
const KEYS = new Set<string>(THEME_COLORS.map((c) => c.key))

export const isBuiltinTheme = (id: string): id is ThemeId => THEMES.some((t) => t.id === id)
export const isColorValue = (v: string) => COLOR_VALUE.test(v)

let canvasCtx: CanvasRenderingContext2D | null | undefined
/** Any CSS color as `#rrggbb` (what `<input type="color">` needs); invalid input gives black. */
export function toHex(color: string): string {
  canvasCtx ??= document.createElement('canvas').getContext('2d')
  if (!canvasCtx) return '#000000'
  canvasCtx.fillStyle = '#000000'
  canvasCtx.fillStyle = color
  return canvasCtx.fillStyle.startsWith('#') ? canvasCtx.fillStyle : '#000000'
}

function luminance(hex: string): number {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
  return 0.2126 * r! + 0.7152 * g! + 0.0722 * b!
}

/** Colors of a built-in theme, read from the stylesheet so they never go out of sync with it. */
export function builtinColors(id: ThemeId): Record<string, string> {
  const probe = document.createElement('div')
  probe.dataset.theme = id
  probe.style.display = 'none'
  document.body.appendChild(probe)
  const style = getComputedStyle(probe)
  const colors = Object.fromEntries(THEME_COLORS.map(({ key }) => [key, toHex(style.getPropertyValue(`--${key}`).trim())]))
  probe.remove()
  return colors
}

/**
 * CSS variables (with the `--` prefix) of a custom theme. Surface steps, faint text and the text on accent are
 * derived from the edited colors unless the theme overrides them itself.
 */
export function themeVars(colors: Record<string, string>): Record<string, string> {
  const vars: Record<string, string> = {
    '--c-raised': 'color-mix(in srgb, var(--c-surface) 94%, var(--c-fg))',
    '--c-hover': 'color-mix(in srgb, var(--c-surface) 88%, var(--c-fg))',
    '--c-faint': 'color-mix(in srgb, var(--c-muted) 55%, var(--c-base))',
  }
  if (colors['c-accent']) vars['--c-accent-fg'] = luminance(toHex(colors['c-accent'])) > 0.55 ? '#0a0a0a' : '#ffffff'
  if (colors['c-base']) vars['color-scheme'] = luminance(toHex(colors['c-base'])) > 0.5 ? 'light' : 'dark'
  for (const [key, value] of Object.entries(colors)) vars[`--${key}`] = value
  return vars
}

/** Parses pasted JSON into a theme (without id); throws an Error with a user-readable message. */
export function parseThemeJson(text: string): Omit<CustomTheme, 'id'> {
  let data: unknown
  try {
    data = JSON.parse(text)
  } catch {
    throw new Error(t.themeErrors.notJson)
  }
  const obj = data as { name?: unknown; base?: unknown; colors?: unknown } | null
  if (!obj || typeof obj !== 'object' || typeof obj.name !== 'string' || !obj.name.trim()) throw new Error(t.themeErrors.noName)
  if (!obj.colors || typeof obj.colors !== 'object') throw new Error(t.themeErrors.noColors)
  const colors: Record<string, string> = {}
  for (const [key, value] of Object.entries(obj.colors)) {
    if (!KEYS.has(key)) continue
    if (typeof value !== 'string' || !COLOR_VALUE.test(value)) throw new Error(t.themeErrors.badColor(key))
    colors[key] = value
  }
  if (!Object.keys(colors).length) throw new Error(t.themeErrors.noColors)
  const base = typeof obj.base === 'string' && isBuiltinTheme(obj.base) ? obj.base : 'k0t1k'
  return { name: obj.name.trim().slice(0, 32), base, colors }
}
