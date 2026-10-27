import { onBeforeUnmount, ref, type Ref } from 'vue'

/** Elements that keep their own click behaviour: a drag never starts on them. */
const INTERACTIVE = 'button, a, input, textarea, select, label, [role="switch"], [data-no-marquee]'
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

/**
 * Explorer-like selection for a list or grid: drag a rectangle over items marked `data-select-key`,
 * Ctrl/Shift+drag adds to the selection, a click on empty space clears it.
 * Bind `onPointerDown` to the (focusable) wrapper; click selection of single items stays with the caller.
 */
export function useMarquee(selected: Ref<Set<string>>) {
  const box = ref<MarqueeBox | null>(null)
  let cleanup: (() => void) | null = null

  function onPointerDown(event: PointerEvent) {
    if (event.button !== 0 || cleanup) return
    const root = event.currentTarget as HTMLElement
    const target = event.target as HTMLElement
    if (target.closest(INTERACTIVE)) return
    root.focus({ preventScroll: true })

    const additive = event.ctrlKey || event.metaKey || event.shiftKey
    const base = additive ? new Set(selected.value) : new Set<string>()
    const scroller = scrollParent(root)
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

  onBeforeUnmount(() => cleanup?.())

  return { box, onPointerDown }
}
