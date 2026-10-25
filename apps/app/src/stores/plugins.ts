import { defineStore } from 'pinia'
import { ref } from 'vue'
import { checkPluginUpdates, listPlugins } from '../api/commands'
import type { PluginId, PluginInfo } from '../api/types'
import { errorMessage } from '../utils/format'

export const usePluginsStore = defineStore('plugins', () => {
  const list = ref<PluginInfo[]>([])
  const checking = ref(false)
  /** The last failed manual check; per-plugin errors of background checks are in `list`. */
  const error = ref('')
  /** The list is re-read now and then: the startup check may install a release while the UI runs. */
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

  /** Settings > Plugins "check now". */
  async function check() {
    if (checking.value) return
    checking.value = true
    error.value = ''
    try {
      list.value = await checkPluginUpdates(true)
      loadedAt = Date.now()
      loading = Promise.resolve()
    } catch (e) {
      error.value = errorMessage(e)
    } finally {
      checking.value = false
    }
  }

  return { list, checking, error, load, infoOf, check }
})
