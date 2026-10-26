import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { checkForUpdate, installUpdate } from '../api/commands'
import type { UpdateStatus } from '../api/types'
import { errorMessage } from '../utils/format'
import { useInstancesStore } from './instances'
import { usePluginsStore } from './plugins'

export const useUpdateStore = defineStore('update', () => {
  const instances = useInstancesStore()
  const plugins = usePluginsStore()
  const status = ref<UpdateStatus | null>(null)
  const checking = ref(false)
  const installing = ref(false)
  /** The last failed manual check or install; a failed startup check stays silent. */
  const error = ref('')

  const release = computed(() => (status.value?.status === 'available' ? status.value.release : null))
  /** Installing would replace the exe under a running game, so it waits until every session ends. */
  const gameRunning = computed(() => instances.instances.some((i) => instances.stateOf(i.id) !== 'stopped'))

  /** Checks the launcher and the plugins (newer plugins are installed by the backend). `force` asks GitHub
   * again; the startup check reuses the result of this run. */
  async function check(force: boolean, silent = false) {
    if (checking.value) return
    checking.value = true
    if (!silent) error.value = ''
    try {
      status.value = await checkForUpdate(force)
    } catch (e) {
      // the backend has logged it; only an explicit check shows it
      if (!silent) error.value = errorMessage(e)
    } finally {
      checking.value = false
    }
    // the plugin part of the check ran even when the launcher part failed
    plugins.reload().catch((e: unknown) => console.warn('cannot reload the plugin list', e))
  }

  async function install() {
    if (!release.value) return
    installing.value = true
    error.value = ''
    try {
      await installUpdate(release.value.version)
    } catch (e) {
      error.value = errorMessage(e)
    } finally {
      installing.value = false
    }
  }

  return { status, checking, installing, error, release, gameRunning, check, install }
})
