import { defineStore } from 'pinia'
import { computed, reactive, ref, shallowReactive } from 'vue'
import * as api from '../api/commands'
import type { GameLogBatch, GameLogLine, Instance, ModpackImport, RunState } from '../api/types'
import { useTasksStore } from './tasks'

export const MAX_LOG_LINES = 5000
const LOG_FLUSH_MS = 100

interface LogBuffer {
  /** `seq` is a stable key, so trimming the head of the buffer does not re-render the rest of the list. */
  lines: GameLogLine[]
  /** Lines pushed out by the cap since the last clear. */
  dropped: number
}
const EMPTY_LOG: LogBuffer = { lines: [], dropped: 0 }

export const useInstancesStore = defineStore('instances', () => {
  const tasks = useTasksStore()
  const instances = ref<Instance[]>([])
  const loaded = ref(false)
  const runStates = reactive(new Map<string, RunState>())
  // Buffers are replaced (never mutated) once per flush, so a flood of lines costs one reactive update per flush.
  const logs = shallowReactive(new Map<string, LogBuffer>())
  const pendingLogs = new Map<string, LogBuffer>()
  // newest line seen per instance: a line can arrive both live and in the backfill of a fresh window
  const lastSeq = new Map<string, number>()
  let flushTimer: ReturnType<typeof setTimeout> | undefined

  const groups = computed(() => [...new Set(instances.value.map((i) => i.group).filter((g): g is string => !!g))].sort())

  const byId = (id: string) => instances.value.find((i) => i.id === id)
  const stateOf = (id: string): RunState => runStates.get(id) ?? 'stopped'
  const logOf = (id: string): LogBuffer => logs.get(id) ?? EMPTY_LOG

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
    pendingLogs.delete(id)
    logs.set(id, { lines: [], dropped: 0 })
  }

  function flushLogs() {
    flushTimer = undefined
    for (const [id, pending] of pendingLogs) {
      const old = logs.get(id) ?? EMPTY_LOG
      const merged = old.lines.concat(pending.lines)
      const overflow = Math.max(0, merged.length - MAX_LOG_LINES)
      logs.set(id, { lines: overflow ? merged.slice(overflow) : merged, dropped: old.dropped + pending.dropped + overflow })
    }
    pendingLogs.clear()
  }

  function pushLogs(batch: GameLogBatch) {
    const pending = pendingLogs.get(batch.instanceId) ?? { lines: [], dropped: 0 }
    const seen = lastSeq.get(batch.instanceId) ?? -1
    for (const l of batch.lines) if (l.seq > seen) pending.lines.push(l)
    lastSeq.set(batch.instanceId, Math.max(seen, batch.lines[batch.lines.length - 1]?.seq ?? seen))
    // while the window is hidden nothing flushes, so the pending part is capped as well
    if (pending.lines.length > MAX_LOG_LINES) {
      pending.dropped += pending.lines.length - MAX_LOG_LINES
      pending.lines.splice(0, pending.lines.length - MAX_LOG_LINES)
    }
    pendingLogs.set(batch.instanceId, pending)
    flushTimer ??= setTimeout(() => requestAnimationFrame(flushLogs), LOG_FLUSH_MS)
  }

  /** Fills the log of a window opened while the game was already running, with what the backend kept. */
  async function backfillLogs(id: string) {
    const kept = await api.getGameLogs(id)
    // live lines that arrived while the request was in flight are newer than the kept ones that overlap them
    flushLogs()
    const current = logs.get(id) ?? EMPTY_LOG
    const oldest = current.lines[0]?.seq ?? Infinity
    const merged = kept.filter((l) => l.seq < oldest).concat(current.lines)
    const overflow = Math.max(0, merged.length - MAX_LOG_LINES)
    lastSeq.set(id, Math.max(lastSeq.get(id) ?? -1, merged[merged.length - 1]?.seq ?? -1))
    if (merged.length) logs.set(id, { lines: overflow ? merged.slice(overflow) : merged, dropped: current.dropped + overflow })
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
      api.onGameLog(pushLogs),
    ])
    for (const s of await api.listRunStates()) runStates.set(s.instanceId, s.state)
    await refresh()
    await Promise.all(instances.value.map((i) => backfillLogs(i.id)))
  }

  return {
    instances, loaded, groups, byId, stateOf, logOf, refresh, upsert, create, rename, copy, setGroup,
    updateSettings, remove, importModpack, installModpack, launch, kill, clearLog, init,
  }
})
