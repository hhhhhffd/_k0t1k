import { defineStore } from 'pinia'
import { ref } from 'vue'
import { listPlugins } from '../api/commands'
import type { PluginId, PluginInfo } from '../api/types'

export const usePluginsStore = defineStore('plugins', () => {
  const list = ref<PluginInfo[]>([])
  /** The list is re-read now and then: the update check may install a release while the UI runs. */
  const MAX_AGE_MS = 30_000
  let loadedAt = 0
  let loading: Promise<void> | undefined

  function load() {
    if (loading && Date.now() - loadedAt < MAX_AGE_MS) return loading
    loadedAt = Date.now()
    loading = listPlugins().then(
      (plugins) => void (list.value = plugins),
      (e: unknown) => {
        loadedAt = 0
        throw e
      },
    )
    return loading
  }

  async function infoOf(id: PluginId): Promise<PluginInfo> {
    await load()
    const info = list.value.find((plugin) => plugin.id === id)
    if (!info) throw new Error(`unknown plugin ${id}`)
    return info
  }

  /** Reads the list again, after the update check has installed plugins. */
  function reload() {
    loadedAt = 0
    return load()
  }

  return { list, load, reload, infoOf }
})
