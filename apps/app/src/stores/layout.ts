import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

const KEY = 'k0t1k.layout'

/** `steam`: a library styled after Steam's, with the instance pages inside it. `classic` is the legacy interface, kept as is. */
export type UiMode = 'classic' | 'steam'

interface SavedLayout {
  mode: UiMode
  toolbar: 'top' | 'bottom'
  panel: 'left' | 'right'
  selectedId: string | null
}

function read(): SavedLayout {
  const fallback: SavedLayout = { mode: 'steam', toolbar: 'top', panel: 'right', selectedId: null }
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<SavedLayout>
    return {
      mode: raw.mode === 'classic' ? 'classic' : 'steam',
      toolbar: raw.toolbar === 'bottom' ? 'bottom' : 'top',
      panel: raw.panel === 'left' ? 'left' : 'right',
      selectedId: typeof raw.selectedId === 'string' ? raw.selectedId : null,
    }
  } catch {
    return fallback
  }
}

/** The interface mode, where the movable toolbar and instance panel are docked, and the instance selected in the library. */
export const useLayoutStore = defineStore('layout', () => {
  const saved = read()
  const mode = ref(saved.mode)
  const toolbar = ref(saved.toolbar)
  const panel = ref(saved.panel)
  const selectedId = ref(saved.selectedId)
  const search = ref('')

  watch([mode, toolbar, panel, selectedId], () => {
    try {
      localStorage.setItem(KEY, JSON.stringify({ mode: mode.value, toolbar: toolbar.value, panel: panel.value, selectedId: selectedId.value }))
    } catch {
      // the layout is a convenience only
    }
  })

  return { mode, toolbar, panel, selectedId, search }
})
