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

  function handle(e: ProgressEvent) {
    tasks.set(e.task, e)
    if (e.error) {
      reported.add(e.error)
      setTimeout(() => reported.delete(e.error!), 10_000)
      toast(`${e.title ?? e.task}: ${e.error}`, 'error')
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

  return { tasks: list, toasts, blocked, blockedInstanceId, toast, dismiss, guard, showBlocked, init }
})
