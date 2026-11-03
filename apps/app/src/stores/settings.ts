import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api/commands'
import { isTauri } from '../api/commands'
import type { CustomTheme, GlobalSettings, ThemeId } from '../api/types'
import { setLanguage } from '../i18n'
import { isBuiltinTheme, resolvedOverlayTheme, themeVars } from '../utils/theme'

const THEME_CACHE = 'k0t1k.ui'

interface ResolvedTheme {
  base: ThemeId
  colors: Record<string, string>
}

let appliedVars: string[] = []

function resolveTheme(id: string, custom: CustomTheme[]): ResolvedTheme {
  if (isBuiltinTheme(id)) return { base: id, colors: {} }
  const found = custom.find((c) => c.id === id)
  // a deleted or unknown custom theme falls back to the default one
  return found ? { base: found.base, colors: found.colors } : { base: 'k0t1k', colors: {} }
}

function applyResolved({ base, colors }: ResolvedTheme, lite: boolean) {
  const root = document.documentElement
  root.dataset.theme = base
  root.dataset.lite = String(lite)
  for (const name of appliedVars) root.style.removeProperty(name)
  const vars = Object.keys(colors).length ? themeVars(colors) : {}
  for (const [name, value] of Object.entries(vars)) root.style.setProperty(name, value)
  appliedVars = Object.keys(vars)
}

let overlayThemeTimer: number | undefined

/** Passes the applied theme to the overlay. Debounced: the theme editor applies on every colour drag. */
function publishOverlayTheme(id: string, name: string) {
  if (!isTauri) return
  window.clearTimeout(overlayThemeTimer)
  overlayThemeTimer = window.setTimeout(() => {
    api.setOverlayTheme(resolvedOverlayTheme(id, name)).catch((e) => console.warn('cannot write the overlay theme', e))
  }, 300)
}

function applyUi(theme: string, lite: boolean, custom: CustomTheme[]) {
  const resolved = resolveTheme(theme, custom)
  applyResolved(resolved, lite)
  publishOverlayTheme(theme, custom.find((c) => c.id === theme)?.name ?? theme)
  try {
    localStorage.setItem(THEME_CACHE, JSON.stringify({ ...resolved, lite }))
  } catch {
    // the cache only avoids a theme flash on startup; settings.json stays the source of truth
  }
}

/** Zoom that suits the screen: a 1440p or larger screen at 100 % Windows scaling gets a bigger interface. */
export function autoScale(): number {
  const width = window.screen.width
  return width >= 3400 ? 150 : width >= 2500 ? 125 : 100
}

async function applyScale(percent: number | null | undefined) {
  if (!isTauri) return
  try {
    const { getCurrentWebview } = await import('@tauri-apps/api/webview')
    await getCurrentWebview().setZoom((percent ?? autoScale()) / 100)
  } catch (e) {
    console.warn('cannot set the interface scale', e)
  }
}

/** Apply the last known theme (custom ones included) before the first paint, before settings finish loading. */
export function applyCachedUi() {
  try {
    const raw = localStorage.getItem(THEME_CACHE)
    if (raw) {
      const { base, colors, lite } = JSON.parse(raw) as ResolvedTheme & { lite: boolean }
      applyResolved({ base, colors }, lite)
    }
  } catch {
    // no cache: defaults from CSS apply
  }
}

export const useSettingsStore = defineStore('settings', () => {
  const settings = ref<GlobalSettings | null>(null)
  /** Themes from the launcher's `themes` folder; chosen like custom themes, edited only as files. */
  const themeFiles = ref<CustomTheme[]>([])
  const allThemes = () => [...(settings.value?.ui.customThemes ?? []), ...themeFiles.value]

  async function load() {
    const loaded = await api.getSettings()
    settings.value = loaded
    setLanguage(loaded.ui.language)
    applyUi(loaded.ui.theme, loaded.ui.liteMode, allThemes())
    void applyScale(loaded.ui.scale)
  }

  /** Persist the current (already mutated) settings. */
  async function save() {
    if (settings.value) await api.saveSettings(settings.value)
  }

  async function setUi(patch: Partial<GlobalSettings['ui']>) {
    if (!settings.value) return
    settings.value.ui = { ...settings.value.ui, ...patch }
    if (patch.language) setLanguage(patch.language)
    if ('scale' in patch) void applyScale(patch.scale)
    reapply()
    await save()
  }

  /** Re-apply the current theme after its (custom) colors were edited; persisting is up to the caller. */
  function reapply() {
    if (settings.value) applyUi(settings.value.ui.theme, settings.value.ui.liteMode, allThemes())
  }
  function setThemeFiles(themes: CustomTheme[]) {
    themeFiles.value = themes
    reapply()
  }

  async function addAccount(name: string) {
    const acc = await api.addOfflineAccount(name)
    await load()
    return acc
  }
  async function removeAccount(id: string) {
    await api.removeAccount(id)
    await load()
  }
  async function selectAccount(id: string) {
    await api.selectAccount(id)
    await load()
  }

  /** Remembers a finished first-run step; the backend owns the flags, so the local copy is updated too. */
  async function markOnboarding(step: 'welcome' | 'checklist' | 'instanceTour') {
    await api.markOnboarding(step)
    if (!settings.value) return
    const flag = { welcome: 'welcomeDone', checklist: 'checklistHidden', instanceTour: 'instanceTourDone' } as const
    settings.value.onboarding[flag[step]] = true
  }

  return { settings, themeFiles, load, setThemeFiles, save, setUi, reapply, addAccount, removeAccount, selectAccount, markOnboarding }
})
