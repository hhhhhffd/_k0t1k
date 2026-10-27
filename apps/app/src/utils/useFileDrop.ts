import { onBeforeUnmount, onMounted, ref } from 'vue'
import { onDragDrop } from '../api/commands'
import type { Unlisten } from '../api/commands'

/**
 * Calls `onDrop` with the real paths of files dropped onto the window and the drop point; `dragging` is
 * true while files are over it and `position` follows the cursor.
 */
export function useFileDrop(onDrop: (paths: string[], at: { x: number; y: number }) => void) {
  const dragging = ref(false)
  const position = ref<{ x: number; y: number } | null>(null)
  let unlisten: Unlisten | undefined
  let unmounted = false

  onMounted(async () => {
    const stop = await onDragDrop((event) => {
      if (event.type === 'leave') {
        dragging.value = false
        position.value = null
        return
      }
      position.value = { x: event.x, y: event.y }
      if (event.type === 'enter') dragging.value = true
      else if (event.type === 'drop') {
        dragging.value = false
        position.value = null
        if (event.paths.length) onDrop(event.paths, { x: event.x, y: event.y })
      }
    })
    // the view can be gone before the listener is registered
    if (unmounted) stop()
    else unlisten = stop
  })
  onBeforeUnmount(() => {
    unmounted = true
    unlisten?.()
  })
  return { dragging, position }
}

/** Last path segment, for messages. */
export const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path
