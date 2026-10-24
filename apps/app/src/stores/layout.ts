import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

const KEY = 'k0t1k.layout'

interface SavedLayout {
  toolbar: 'top' | 'bottom'
  panel: 'left' | 'right'
  selectedId: string | null
}

function read(): SavedLayout {
  const fallback: SavedLayout = { toolbar: 'top', panel: 'right', selectedId: null }
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Partial<SavedLayout>
    return {
      toolbar: raw.toolbar === 'bottom' ? 'bottom' : 'top',
      panel: raw.panel === 'left' ? 'left' : 'right',
      selectedId: typeof raw.selectedId === 'string' ? raw.selectedId : null,
    }
  } catch {
    return fallback
  }
}

/** Where the movable toolbar and instance panel are docked, and the instance selected in the library. */
export const useLayoutStore = defineStore('layout', () => {
  const saved = read()
  const toolbar = ref(saved.toolbar)
  const panel = ref(saved.panel)
  const selectedId = ref(saved.selectedId)
  const search = ref('')

  watch([toolbar, panel, selectedId], () => {
    try {
      localStorage.setItem(KEY, JSON.stringify({ toolbar: toolbar.value, panel: panel.value, selectedId: selectedId.value }))
    } catch {
      // the layout is a convenience only
    }
  })

  return { toolbar, panel, selectedId, search }
})
