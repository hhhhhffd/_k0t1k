import { defineStore } from 'pinia'
import { computed, reactive, ref } from 'vue'
import * as api from '../api/commands'
import type { GameLogLine, Instance, ModpackImport, RunState } from '../api/types'
import { useTasksStore } from './tasks'

const MAX_LOG_LINES = 5000

export const useInstancesStore = defineStore('instances', () => {
  const tasks = useTasksStore()
  const instances = ref<Instance[]>([])
  const loaded = ref(false)
  const runStates = reactive(new Map<string, RunState>())
  const logs = reactive(new Map<string, GameLogLine[]>())

  const groups = computed(() => [...new Set(instances.value.map((i) => i.group).filter((g): g is string => !!g))].sort())

  const byId = (id: string) => instances.value.find((i) => i.id === id)
  const stateOf = (id: string): RunState => runStates.get(id) ?? 'stopped'
  const logOf = (id: string): GameLogLine[] => logs.get(id) ?? []

  async function refresh() {
    const listing = await api.listInstances()
    instances.value = listing.instances
    for (const broken of listing.errors) tasks.toast(`${broken.id}: ${broken.error}`, 'error')
    loaded.value = true
  }

  /** Insert or replace an instance returned by the backend. */
  function upsert(inst: Instance) {
    const idx = instances.value.findIndex((i) => i.id === inst.id)
    if (idx >= 0) instances.value[idx] = inst
    else instances.value.push(inst)
    return inst
  }

  const create = async (...args: Parameters<typeof api.createInstance>) => upsert(await api.createInstance(...args))
  const rename = async (id: string, name: string) => upsert(await api.renameInstance(id, name))
  const copy = async (id: string, name: string) => upsert(await api.copyInstance(id, name))
  const setGroup = async (id: string, group: string | null) => upsert(await api.setInstanceGroup(id, group))
  const updateSettings = async (...args: Parameters<typeof api.updateInstanceSettings>) =>
    upsert(await api.updateInstanceSettings(...args))
  async function remove(id: string) {
    await api.deleteInstance(id)
    instances.value = instances.value.filter((i) => i.id !== id)
  }
  function upsertImport(result: ModpackImport) {
    tasks.showBlocked(result.blocked)
    return upsert(result.instance)
  }
  const importModpack = async (path: string) => upsertImport(await api.importModpackFile(path))
  const installModpack = async (...args: Parameters<typeof api.installModpackVersion>) =>
    upsertImport(await api.installModpackVersion(...args))

  const launch = (id: string, accountId?: string) => api.launchInstance(id, accountId)
  const kill = (id: string) => api.killInstance(id)
  function clearLog(id: string) {
    logs.set(id, [])
  }

  let started = false
  async function init() {
    if (started) return
    started = true
    await Promise.all([
      api.onInstanceState((e) => {
        runStates.set(e.instanceId, e.state)
        // playtime and lastPlayed change when a session ends
        if (e.state === 'stopped') void refresh()
      }),
      api.onGameLog((e) => {
        const lines = logs.get(e.instanceId) ?? []
        lines.push(e)
        if (lines.length > MAX_LOG_LINES) lines.splice(0, lines.length - MAX_LOG_LINES)
        logs.set(e.instanceId, lines)
      }),
    ])
    for (const s of await api.listRunStates()) runStates.set(s.instanceId, s.state)
    await refresh()
  }

  return {
    instances, loaded, groups, byId, stateOf, logOf, refresh, upsert, create, rename, copy, setGroup,
    updateSettings, remove, importModpack, installModpack, launch, kill, clearLog, init,
  }
})
