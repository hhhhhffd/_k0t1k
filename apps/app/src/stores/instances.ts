import { defineStore } from 'pinia'
import { computed, reactive, ref, shallowReactive } from 'vue'
import * as api from '../api/commands'
import type { GameLogBatch, GameLogLine, Instance, InstanceRunState, ModpackImport, RunSession, RunState, SessionId } from '../api/types'
import { t } from '../i18n/ru'
import { useTasksStore } from './tasks'

export const MAX_LOG_LINES = 5000
const LOG_FLUSH_MS = 100
/** Finished sessions per instance whose log stays (the backend keeps the same number). */
const KEPT_FINISHED_SESSIONS = 3
const MULTI_LAUNCH_HINT_KEY = 'k0t1k.multiLaunchHintShown'

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
  /** Live sessions and finished ones whose log is kept; entries are replaced, never mutated. */
  const sessions = reactive(new Map<SessionId, RunSession>())
  // Buffers are replaced (never mutated) once per flush, so a flood of lines costs one reactive update per flush.
  const logs = shallowReactive(new Map<SessionId, LogBuffer>())
  const pendingLogs = new Map<SessionId, LogBuffer>()
  // newest line seen per session: a line can arrive both live and in the backfill of a fresh window
  const lastSeq = new Map<SessionId, number>()
  let flushTimer: ReturnType<typeof setTimeout> | undefined

  const groups = computed(() => [...new Set(instances.value.map((i) => i.group).filter((g): g is string => !!g))].sort())

  const byId = (id: string) => instances.value.find((i) => i.id === id)
  /** Sessions of an instance, oldest first (finished ones included while their log is kept). */
  const sessionsOf = (instanceId: string): RunSession[] =>
    [...sessions.values()].filter((s) => s.instanceId === instanceId).sort((a, b) => a.sessionId - b.sessionId)
  const liveSessionsOf = (instanceId: string) => sessionsOf(instanceId).filter((s) => s.state !== 'stopped')
  /** The instance's overall state: `running` while any session runs, else `preparing`, else `stopping`. */
  function stateOf(instanceId: string): RunState {
    const live = liveSessionsOf(instanceId)
    for (const state of ['running', 'preparing', 'stopping'] as const) if (live.some((s) => s.state === state)) return state
    return 'stopped'
  }
  const logOf = (sessionId: SessionId): LogBuffer => logs.get(sessionId) ?? EMPTY_LOG

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

  async function launch(id: string, accountId?: string) {
    const alreadyRunning = liveSessionsOf(id).length > 0
    const sessionId = await api.launchInstance(id, accountId)
    // Minecraft locks a world while it is open, so a second copy cannot open the same one.
    if (alreadyRunning && !localStorage.getItem(MULTI_LAUNCH_HINT_KEY)) {
      localStorage.setItem(MULTI_LAUNCH_HINT_KEY, '1')
      tasks.toast(t.library.multiLaunchHint)
    }
    return sessionId
  }
  /** Stops every session of the instance. */
  const kill = (id: string) => api.killInstance(id)
  const killSession = (sessionId: SessionId) => api.killSession(sessionId)
  function clearLog(sessionId: SessionId) {
    pendingLogs.delete(sessionId)
    logs.set(sessionId, { lines: [], dropped: 0 })
  }

  /** Mirrors the backend: only the newest finished sessions of an instance keep their log. */
  function pruneFinished(instanceId: string) {
    const finished = sessionsOf(instanceId).filter((s) => s.state === 'stopped')
    for (const old of finished.slice(0, Math.max(0, finished.length - KEPT_FINISHED_SESSIONS))) {
      sessions.delete(old.sessionId)
      logs.delete(old.sessionId)
      pendingLogs.delete(old.sessionId)
      lastSeq.delete(old.sessionId)
    }
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
    const pending = pendingLogs.get(batch.sessionId) ?? { lines: [], dropped: 0 }
    const seen = lastSeq.get(batch.sessionId) ?? -1
    for (const l of batch.lines) if (l.seq > seen) pending.lines.push(l)
    lastSeq.set(batch.sessionId, Math.max(seen, batch.lines[batch.lines.length - 1]?.seq ?? seen))
    // while the window is hidden nothing flushes, so the pending part is capped as well
    if (pending.lines.length > MAX_LOG_LINES) {
      pending.dropped += pending.lines.length - MAX_LOG_LINES
      pending.lines.splice(0, pending.lines.length - MAX_LOG_LINES)
    }
    pendingLogs.set(batch.sessionId, pending)
    flushTimer ??= setTimeout(() => requestAnimationFrame(flushLogs), LOG_FLUSH_MS)
  }

  /** Fills the log of a session that started before this window opened, with what the backend kept. */
  async function backfillLogs(id: SessionId) {
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
      api.onInstanceState((e: InstanceRunState) => {
        const known = sessions.get(e.sessionId)
        sessions.set(e.sessionId, {
          sessionId: e.sessionId,
          instanceId: e.instanceId,
          number: e.number,
          state: e.state,
          startedAt: known?.startedAt ?? new Date().toISOString(),
        })
        if (e.state === 'stopped') {
          pruneFinished(e.instanceId)
          // playtime and lastPlayed change when a session ends
          void refresh()
        }
      }),
      api.onGameLog(pushLogs),
    ])
    // a state event that arrived while the request was in flight is newer than the listing
    for (const s of await api.listRunStates()) if (!sessions.has(s.sessionId)) sessions.set(s.sessionId, s)
    await refresh()
    await Promise.all([...sessions.keys()].map(backfillLogs))
  }

  return {
    instances, loaded, groups, byId, stateOf, sessionsOf, liveSessionsOf, logOf, refresh, upsert, create, rename, copy, setGroup,
    updateSettings, remove, importModpack, installModpack, launch, kill, killSession, clearLog, init,
  }
})
