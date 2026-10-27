import { onBeforeUnmount, onMounted, ref, type Ref } from 'vue'

/** Elements that keep their own click behaviour: a drag never starts on them. */
const INTERACTIVE = 'button, a, input, textarea, select, label, [role="switch"], [role="dialog"], [role="menu"], [data-no-marquee]'
/** Distance from the container edge where dragging starts scrolling it, and the top scroll speed in px per frame. */
const EDGE = 40
const MAX_SPEED = 20
/** Movement below this many pixels is still a click. */
const DRAG_THRESHOLD = 4

export interface MarqueeBox {
  left: number
  top: number
  width: number
  height: number
}

function scrollParent(el: HTMLElement): HTMLElement | null {
  for (let p = el.parentElement; p; p = p.parentElement) {
    const overflow = getComputedStyle(p).overflowY
    if (overflow === 'auto' || overflow === 'scroll') return p
  }
  return null
}

/** Keys are not taken from fields where they edit text. */
const isTextField = (el: EventTarget | null) =>
  el instanceof HTMLElement && (el.isContentEditable || el.closest('input, textarea, select') !== null)

/**
 * Explorer-like selection for a list or grid: drag a rectangle over items marked `data-select-key`,
 * Ctrl/Shift+drag adds to the selection, a click on empty space clears it.
 * `anchor` is any element of the tab; the drag may start anywhere in the scrollable area around it
 * (margins, empty space), except on buttons, inputs, links and `data-no-marquee` blocks.
 * `onKey` receives key presses (Ctrl+A, Delete, Esc) while the component is mounted and no text field or dialog has focus.
 * Click selection of single items stays with the caller.
 */
export function useMarquee(selected: Ref<Set<string>>, anchor: Ref<HTMLElement | null>, onKey?: (e: KeyboardEvent) => void) {
  const box = ref<MarqueeBox | null>(null)
  let cleanup: (() => void) | null = null

  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0 || cleanup || !anchor.value) return
    const target = event.target as HTMLElement
    if (!(target instanceof HTMLElement) || target.closest(INTERACTIVE)) return
    const scroller = scrollParent(anchor.value)
    const root = scroller ?? document.body
    if (!root.contains(target)) return
    // A press on the scrollbar belongs to the scrollbar.
    if (scroller && (event.clientX >= scroller.getBoundingClientRect().left + scroller.clientWidth || event.clientY >= scroller.getBoundingClientRect().top + scroller.clientHeight)) return

    const additive = event.ctrlKey || event.metaKey || event.shiftKey
    const base = additive ? new Set(selected.value) : new Set<string>()
    const bounds = () => (scroller ? scroller.getBoundingClientRect() : { left: 0, top: 0, right: innerWidth, bottom: innerHeight })
    const scrollLeft = () => scroller?.scrollLeft ?? scrollX
    const scrollTop = () => scroller?.scrollTop ?? scrollY

    // The start point is kept in content coordinates so it follows the content while the container scrolls.
    const b0 = bounds()
    const origin = { x: event.clientX - b0.left + scrollLeft(), y: event.clientY - b0.top + scrollTop() }
    const pointer = { x: event.clientX, y: event.clientY }
    const press = { x: event.clientX, y: event.clientY }
    let dragging = false
    let frame = 0

    function update() {
      const b = bounds()
      const px = Math.min(Math.max(pointer.x, b.left), b.right)
      const py = Math.min(Math.max(pointer.y, b.top), b.bottom)
      const sx = origin.x - scrollLeft() + b.left
      const sy = origin.y - scrollTop() + b.top
      const left = Math.min(sx, px)
      const top = Math.min(sy, py)
      const right = Math.max(sx, px)
      const bottom = Math.max(sy, py)
      box.value = { left, top, width: right - left, height: bottom - top }
      const next = new Set(base)
      for (const el of root.querySelectorAll<HTMLElement>('[data-select-key]')) {
        const r = el.getBoundingClientRect()
        if (r.right > left && r.left < right && r.bottom > top && r.top < bottom) next.add(el.dataset.selectKey!)
      }
      selected.value = next
    }

    function tick() {
      const b = bounds()
      const speed = (distance: number) => Math.ceil(MAX_SPEED * Math.min(1, (EDGE - distance) / EDGE))
      let dy = 0
      if (pointer.y < b.top + EDGE) dy = -speed(pointer.y - b.top)
      else if (pointer.y > b.bottom - EDGE) dy = speed(b.bottom - pointer.y)
      if (dy) {
        if (scroller) scroller.scrollTop += dy
        else scrollBy(0, dy)
        update()
      }
      frame = requestAnimationFrame(tick)
    }

    function onMove(e: PointerEvent) {
      pointer.x = e.clientX
      pointer.y = e.clientY
      if (!dragging) {
        if (Math.hypot(pointer.x - press.x, pointer.y - press.y) < DRAG_THRESHOLD) return
        dragging = true
        document.body.style.userSelect = 'none'
        getSelection()?.removeAllRanges()
        frame = requestAnimationFrame(tick)
      }
      update()
    }

    function finish() {
      cancelAnimationFrame(frame)
      window.removeEventListener('pointermove', onMove)
      window.removeEventListener('pointerup', onUp)
      window.removeEventListener('pointercancel', finish)
      document.body.style.userSelect = ''
      box.value = null
      cleanup = null
    }

    function onUp() {
      const wasDragging = dragging
      finish()
      if (wasDragging) {
        // The click that ends a drag must not reach the item under the cursor and reset the selection.
        const swallow = (e: Event) => e.stopPropagation()
        window.addEventListener('click', swallow, { capture: true, once: true })
        setTimeout(() => window.removeEventListener('click', swallow, { capture: true }), 0)
      } else if (!additive && !target.closest('[data-select-key]')) {
        selected.value = new Set()
      }
    }

    window.addEventListener('pointermove', onMove)
    window.addEventListener('pointerup', onUp)
    window.addEventListener('pointercancel', finish)
    cleanup = finish
  }

  function onWindowKey(e: KeyboardEvent) {
    if (!onKey || !anchor.value || isTextField(e.target) || document.querySelector('[role="dialog"], [role="menu"]')) return
    // `code`, not `key`: on the Russian layout the A key reports 'ф'.
    if (e.code === 'KeyA' && (e.ctrlKey || e.metaKey)) onKey(e)
    else if (!e.ctrlKey && !e.metaKey && !e.altKey && (e.key === 'Delete' || e.key === 'Escape')) onKey(e)
  }

  onMounted(() => {
    window.addEventListener('pointerdown', onPointerDown)
    window.addEventListener('keydown', onWindowKey)
  })
  onBeforeUnmount(() => {
    window.removeEventListener('pointerdown', onPointerDown)
    window.removeEventListener('keydown', onWindowKey)
    cleanup?.()
  })

  return { box }
}
