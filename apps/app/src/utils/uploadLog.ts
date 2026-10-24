import * as api from '../api/commands'
import type { SessionId } from '../api/types'
import { t } from '../i18n/ru'
import { useTasksStore } from '../stores/tasks'

/** Uploads the log of a session to mclo.gs, copies the link to the clipboard and reports it. Returns the link, or `undefined` on failure. */
export async function uploadSessionLog(sessionId: SessionId): Promise<string | undefined> {
  const tasks = useTasksStore()
  const uploaded = await tasks.guard(() => api.uploadGameLog(sessionId))
  if (!uploaded) return undefined
  try {
    await navigator.clipboard.writeText(uploaded.url)
    tasks.toast(t.instance.uploaded, 'ok')
  } catch {
    // the link stays visible next to the button; only the copy failed
    tasks.toast(uploaded.url, 'ok')
  }
  return uploaded.url
}
