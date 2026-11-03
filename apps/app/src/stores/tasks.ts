import { defineStore } from 'pinia'
import { computed, reactive, ref } from 'vue'
import { onProgress } from '../api/commands'
import type { BlockedFile, ProgressEvent } from '../api/types'
import { errorMessage } from '../utils/format'

export interface Toast {
  id: number
  kind: 'info' | 'error' | 'ok'
  text: string
  /** A button inside the toast, e.g. "Convert"; clicking it dismisses the toast. */
  action?: { label: string; run: () => void }
}

export const useTasksStore = defineStore('tasks', () => {
  const tasks = reactive(new Map<string, ProgressEvent>())
  const toasts = ref<Toast[]>([])
  /** Files the user has to download by hand; shown in a dialog until dismissed. */
  const blocked = ref<BlockedFile[]>([])
  /** The instance the blocked files belong to; the dialog places the files it finds there. */
  const blockedInstanceId = ref('')
  // Errors already shown through a progress event, so the rejected command does not toast them again.
  const reported = new Set<string>()
  let toastId = 0
  // Per task, the smoothed rate of `done` per millisecond, for the time-left estimate.
  const pace = new Map<string, { startedAt: number; at: number; done: number; total: number; rate: number }>()
  /** No estimate before this long (the first rate is too noisy) or when it would be absurdly long. */
  const ETA_AFTER_MS = 4000
  const ETA_MAX_SECS = 6 * 3600

  const list = computed(() => [...tasks.values()])

  function toast(text: string, kind: Toast['kind'] = 'info', action?: Toast['action']) {
    const id = ++toastId
    toasts.value.push({ id, kind, text, action })
    setTimeout(() => dismiss(id), action ? 15_000 : kind === 'error' ? 8000 : 4000)
  }
  function dismiss(id: number) {
    toasts.value = toasts.value.filter((t) => t.id !== id)
  }
  /** Run a backend call; report failures as a toast instead of leaving the promise rejected. */
  async function guard<T>(action: () => Promise<T>): Promise<T | undefined> {
    try {
      return await action()
    } catch (e) {
      const message = errorMessage(e)
      if (!reported.has(message)) toast(message, 'error')
      return undefined
    }
  }

  function trackPace(e: ProgressEvent) {
    const now = Date.now()
    const p = pace.get(e.task)
    // A new phase (the counter went back or the total changed) starts a fresh estimate.
    if (!p || e.done < p.done || e.total !== p.total) {
      pace.set(e.task, { startedAt: now, at: now, done: e.done, total: e.total, rate: 0 })
      return
    }
    const elapsed = now - p.at
    if (elapsed < 500 || e.done === p.done) return
    const instant = (e.done - p.done) / elapsed
    p.rate = p.rate ? p.rate * 0.7 + instant * 0.3 : instant
    p.at = now
    p.done = e.done
  }
  /** Seconds left for a running task, or null while there is no trustworthy estimate. */
  function etaSecs(task: string): number | null {
    const e = tasks.get(task)
    const p = pace.get(task)
    if (!e || !p || e.finished || e.error || e.total <= 0 || !p.rate || Date.now() - p.startedAt < ETA_AFTER_MS) return null
    const secs = Math.max(0, e.total - e.done) / p.rate / 1000
    return secs <= ETA_MAX_SECS ? secs : null
  }

  function handle(e: ProgressEvent) {
    tasks.set(e.task, e)
    if (e.finished || e.error) pace.delete(e.task)
    else trackPace(e)
    if (e.error) {
      // Stored the way `guard` compares it, so the same error is not toasted twice.
      const message = errorMessage(e.error)
      reported.add(message)
      setTimeout(() => reported.delete(message), 10_000)
      toast(`${e.title ?? e.task}: ${message}`, 'error')
    }
    if (e.finished || e.error) setTimeout(() => tasks.delete(e.task), e.error ? 6000 : 2500)
  }

  function showBlocked(files: BlockedFile[], instanceId: string) {
    if (!files.length) return
    blocked.value = files
    blockedInstanceId.value = instanceId
  }

  let started = false
  async function init() {
    if (started) return
    started = true
    await onProgress(handle)
  }

  return { tasks: list, toasts, blocked, blockedInstanceId, toast, dismiss, guard, showBlocked, etaSecs, init }
})
