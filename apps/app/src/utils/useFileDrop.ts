import { onBeforeUnmount, onMounted, ref } from 'vue'
import { onDragDrop } from '../api/commands'
import type { Unlisten } from '../api/commands'

/** Calls `onDrop` with the real paths of files dropped onto the window; `dragging` is true while files are over it. */
export function useFileDrop(onDrop: (paths: string[]) => void) {
  const dragging = ref(false)
  let unlisten: Unlisten | undefined
  let unmounted = false

  onMounted(async () => {
    const stop = await onDragDrop((event) => {
      if (event.type === 'enter') dragging.value = true
      else if (event.type === 'leave') dragging.value = false
      else if (event.type === 'drop') {
        dragging.value = false
        if (event.paths.length) onDrop(event.paths)
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
  return { dragging }
}

/** Last path segment, for messages. */
export const baseName = (path: string) => path.split(/[\\/]/).pop() ?? path
