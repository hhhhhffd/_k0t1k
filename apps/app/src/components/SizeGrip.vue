<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { isTauri } from '../api/commands'
import { t } from '../i18n'
import { useTasksStore } from '../stores/tasks'

/** Resize grip for the undecorated window. */
const tasks = useTasksStore()
const maximized = ref(false)
let unlisten: (() => void) | undefined

onMounted(async () => {
  if (!isTauri) return
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  const current = getCurrentWindow()
  maximized.value = await current.isMaximized()
  unlisten = await current.onResized(async () => {
    maximized.value = await current.isMaximized()
  })
})
onBeforeUnmount(() => unlisten?.())

async function down(e: PointerEvent) {
  if (e.button !== 0 || !isTauri) return
  e.preventDefault()
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  await tasks.guard(() => getCurrentWindow().startResizeDragging('SouthEast'))
}
</script>

<template>
  <div v-if="!maximized" class="size-grip fixed right-0 bottom-0 z-[60] size-3.5 cursor-nwse-resize touch-none" :title="t.titlebar.resize" @pointerdown="down" />
</template>

<style scoped>
/* dots on a 4px grid, cut to the lower-right triangle */
.size-grip {
  background-image: radial-gradient(var(--c-accent) 1px, transparent 1.3px);
  background-size: 4px 4px;
  background-position: right 1px bottom 1px;
  mask-image: linear-gradient(to bottom right, transparent 50%, black 50%);
  opacity: 0.75;
}
.size-grip:hover {
  opacity: 1;
}
</style>
