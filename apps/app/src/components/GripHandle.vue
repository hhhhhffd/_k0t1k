<script setup lang="ts">
import { computed, onBeforeUnmount, ref } from 'vue'
import { t } from '../i18n/ru'

/**
 * Dotted handle that moves a bar to the other edge of the window: `y` docks it to the top or bottom,
 * `x` to the left or right. Pointer events, not HTML drag and drop, which WebView2 hands to Tauri's file drop.
 */
const props = defineProps<{ axis: 'x' | 'y' }>()
const emit = defineEmits<{ dock: [side: 'start' | 'end'] }>()

const target = ref<'start' | 'end' | null>(null)
const sideAt = (e: PointerEvent) =>
  (props.axis === 'y' ? e.clientY < window.innerHeight / 2 : e.clientX < window.innerWidth / 2) ? 'start' : 'end'

function move(e: PointerEvent) {
  target.value = sideAt(e)
}
function up(e: PointerEvent) {
  stop()
  emit('dock', sideAt(e))
}
function stop() {
  target.value = null
  window.removeEventListener('pointermove', move)
  window.removeEventListener('pointerup', up)
}
function down(e: PointerEvent) {
  if (e.button !== 0) return
  e.preventDefault()
  target.value = sideAt(e)
  window.addEventListener('pointermove', move)
  window.addEventListener('pointerup', up)
}
onBeforeUnmount(stop)

// the strip of the window the bar will snap to
const zone = computed(() => {
  const start = target.value === 'start'
  // 40px keeps clear of the title bar
  return props.axis === 'y'
    ? { left: 0, right: 0, height: '56px', ...(start ? { top: '40px' } : { bottom: 0 }) }
    : { top: '40px', bottom: 0, width: '220px', ...(start ? { left: 0 } : { right: 0 }) }
})
</script>

<template>
  <div
    class="grip shrink-0 cursor-grab touch-none active:cursor-grabbing"
    :class="axis === 'y' ? 'h-6 w-2' : 'h-2 w-8'"
    :title="t.layout.move"
    role="separator"
    @pointerdown="down"
  >
    <Teleport to="body">
      <div v-if="target" class="pointer-events-none fixed z-[70] rounded-lg border-2 border-dashed border-accent bg-accent/15" :style="zone" />
    </Teleport>
  </div>
</template>

<style scoped>
/* Qt-style toolbar grip: a small field of dots */
.grip {
  background-image: radial-gradient(var(--c-faint) 1px, transparent 1.3px);
  background-size: 4px 4px;
  opacity: 0.8;
}
.grip:hover {
  opacity: 1;
  background-image: radial-gradient(var(--c-accent) 1px, transparent 1.3px);
}
</style>
