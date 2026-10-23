import { defineStore } from 'pinia'
import { computed, reactive, ref } from 'vue'
import { onProgress } from '../api/commands'
import type { ProgressEvent } from '../api/types'
import { errorMessage } from '../utils/format'

export interface Toast {
  id: number
  kind: 'info' | 'error' | 'ok'
  text: string
}

export const useTasksStore = defineStore('tasks', () => {
  const tasks = reactive(new Map<string, ProgressEvent>())
  const toasts = ref<Toast[]>([])
  let toastId = 0

  const list = computed(() => [...tasks.values()])

  function toast(text: string, kind: Toast['kind'] = 'info') {
    const id = ++toastId
    toasts.value.push({ id, kind, text })
    setTimeout(() => dismiss(id), kind === 'error' ? 8000 : 4000)
  }
  function dismiss(id: number) {
    toasts.value = toasts.value.filter((t) => t.id !== id)
  }
  /** Run a backend call; report failures as a toast instead of leaving the promise rejected. */
  async function guard<T>(action: () => Promise<T>): Promise<T | undefined> {
    try {
      return await action()
    } catch (e) {
      toast(errorMessage(e), 'error')
      return undefined
    }
  }

  function handle(e: ProgressEvent) {
    tasks.set(e.task, e)
    if (e.error) toast(`${e.title ?? e.task}: ${e.error}`, 'error')
    if (e.finished || e.error) setTimeout(() => tasks.delete(e.task), e.error ? 6000 : 2500)
  }

  let started = false
  async function init() {
    if (started) return
    started = true
    await onProgress(handle)
  }

  return { tasks: list, toasts, toast, dismiss, guard, init }
})
