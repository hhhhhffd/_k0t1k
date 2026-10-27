import { ref, watch } from 'vue'

export type ViewMode = 'list' | 'tiles'

/** The list/tiles choice of one view, remembered between launches. */
export function useViewMode(name: string, fallback: ViewMode = 'list') {
  const key = `k0t1k.view.${name}`
  const mode = ref<ViewMode>(fallback)
  try {
    const stored = localStorage.getItem(key)
    if (stored === 'list' || stored === 'tiles') mode.value = stored
  } catch {
    // the choice is a convenience only
  }
  watch(mode, (value) => {
    try {
      localStorage.setItem(key, value)
    } catch {
      // the choice is a convenience only
    }
  })
  return mode
}
