import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api/commands'
import type { GlobalSettings, ThemeId } from '../api/types'

const THEME_CACHE = 'k0t1k.ui'

export const THEMES: { id: ThemeId; label: string }[] = [
  { id: 'k0t1k', label: '_k0t1k' },
  { id: 'light', label: 'Светлая' },
  { id: 'amoled', label: 'AMOLED' },
  { id: 'mint', label: 'Мята' },
]

function applyUi(theme: ThemeId, lite: boolean) {
  document.documentElement.dataset.theme = theme
  document.documentElement.dataset.lite = String(lite)
  try {
    localStorage.setItem(THEME_CACHE, JSON.stringify({ theme, lite }))
  } catch {
    // the cache only avoids a theme flash on startup; settings.json stays the source of truth
  }
}

/** Apply the last known theme before the first paint, before settings finish loading. */
export function applyCachedUi() {
  try {
    const raw = localStorage.getItem(THEME_CACHE)
    if (raw) {
      const { theme, lite } = JSON.parse(raw) as { theme: ThemeId; lite: boolean }
      applyUi(theme, lite)
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
    applyUi(loaded.ui.theme, loaded.ui.liteMode)
  }

  /** Persist the current (already mutated) settings. */
  async function save() {
    if (settings.value) await api.saveSettings(settings.value)
  }

  async function setUi(patch: Partial<GlobalSettings['ui']>) {
    if (!settings.value) return
    settings.value.ui = { ...settings.value.ui, ...patch }
    applyUi(settings.value.ui.theme, settings.value.ui.liteMode)
    await save()
  }

  async function addAccount(name: string) {
    const acc = await api.addOfflineAccount(name)
    await load()
    return acc
  }
  async function removeAccount(id: string) {
    await api.removeOfflineAccount(id)
    await load()
  }
  async function selectAccount(id: string) {
    await api.selectAccount(id)
    await load()
  }

  return { settings, load, save, setUi, addAccount, removeAccount, selectAccount }
})
