import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import * as api from '../api/commands'
import type { CatPack, CustomTheme, IconSet } from '../api/types'
import { useSettingsStore } from './settings'
import { useTasksStore } from './tasks'

/** What the user dropped into the launcher's icons, catpacks, iconsets and themes folders. */
export const useCustomizationStore = defineStore('customization', () => {
  const settings = useSettingsStore()
  const tasks = useTasksStore()
  const catPacks = ref<CatPack[]>([])
  const iconSets = ref<IconSet[]>([])
  const themeFiles = ref<CustomTheme[]>([])
  const iconLibrary = ref<string[]>([])

  /** Files of the chosen icon set by key; empty for the built-in icons. */
  const iconFiles = computed(() => iconSets.value.find((s) => s.id === settings.settings?.ui.iconSet)?.files ?? {})
  /** The cat to draw: the built-in mascot, a pack, or nothing. */
  const cat = computed(() => {
    const id = settings.settings?.ui.catPack
    if (!id) return null
    if (id === 'mascot') return 'mascot' as const
    return catPacks.value.find((p) => p.id === id) ?? null
  })

  async function refresh() {
    const [cats, sets, themes, icons] = await Promise.all([
      tasks.guard(api.listCatPacks),
      tasks.guard(api.listIconSets),
      tasks.guard(api.listThemeFiles),
      tasks.guard(api.listIconLibrary),
    ])
    catPacks.value = cats ?? []
    iconSets.value = sets ?? []
    themeFiles.value = themes?.themes ?? []
    for (const bad of themes?.errors ?? []) tasks.toast(`${bad.file}: ${bad.error}`, 'error')
    iconLibrary.value = icons ?? []
    settings.setThemeFiles(themeFiles.value)
  }

  return { catPacks, iconSets, themeFiles, iconLibrary, iconFiles, cat, refresh }
})
