import { type Ref } from 'vue'

/**
 * Click, Ctrl/Shift+click, Ctrl+A and Esc handling for a multi-selectable list.
 * `order` is the keys in display order, which Shift+click needs to select a range.
 */
export function useSelection(selected: Ref<Set<string>>) {
  let anchor: string | null = null

  function click(key: string, e: MouseEvent, order: string[]) {
    const additive = e.ctrlKey || e.metaKey || e.shiftKey
    const next = new Set(additive ? selected.value : [])
    if (e.shiftKey && anchor !== null && order.includes(anchor)) {
      const [from, to] = [order.indexOf(anchor), order.indexOf(key)].sort((a, b) => a - b)
      order.slice(from, to! + 1).forEach((k) => next.add(k))
    } else if ((e.ctrlKey || e.metaKey) && next.has(key)) {
      next.delete(key)
    } else {
      next.add(key)
    }
    if (!e.shiftKey) anchor = key
    selected.value = next
  }

  /** Handles Ctrl+A and Esc; returns true when the key was used. */
  function onKey(e: KeyboardEvent, order: string[]): boolean {
    if (e.key === 'a' && (e.ctrlKey || e.metaKey)) {
      e.preventDefault()
      selected.value = new Set(order)
      return true
    }
    if (e.key === 'Escape') {
      selected.value = new Set()
      return true
    }
    return false
  }

  /** Makes `key` the start of the next Shift range. */
  const setAnchor = (key: string | null) => (anchor = key)

  return { click, onKey, setAnchor }
}
