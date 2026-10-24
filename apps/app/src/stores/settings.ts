import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api/commands'
import type { CustomTheme, GlobalSettings, ThemeId } from '../api/types'
import { isBuiltinTheme, themeVars } from '../utils/theme'

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

function applyUi(theme: string, lite: boolean, custom: CustomTheme[]) {
  const resolved = resolveTheme(theme, custom)
  applyResolved(resolved, lite)
  try {
    localStorage.setItem(THEME_CACHE, JSON.stringify({ ...resolved, lite }))
  } catch {
    // the cache only avoids a theme flash on startup; settings.json stays the source of truth
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

  async function load() {
    const loaded = await api.getSettings()
    settings.value = loaded
    applyUi(loaded.ui.theme, loaded.ui.liteMode, loaded.ui.customThemes)
  }

  /** Persist the current (already mutated) settings. */
  async function save() {
    if (settings.value) await api.saveSettings(settings.value)
  }

  async function setUi(patch: Partial<GlobalSettings['ui']>) {
    if (!settings.value) return
    settings.value.ui = { ...settings.value.ui, ...patch }
    reapply()
    await save()
  }

  /** Re-apply the current theme after its (custom) colors were edited; persisting is up to the caller. */
  function reapply() {
    if (settings.value) applyUi(settings.value.ui.theme, settings.value.ui.liteMode, settings.value.ui.customThemes)
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

  return { settings, load, save, setUi, reapply, addAccount, removeAccount, selectAccount }
})
