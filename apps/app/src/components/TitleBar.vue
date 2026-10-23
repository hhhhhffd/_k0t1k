<script setup lang="ts">
import type { Window } from '@tauri-apps/api/window'
import { Minus, Plus, Shrink, X } from '@lucide/vue'
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { isTauri } from '../api/commands'
import { t } from '../i18n/ru'
import { useTasksStore } from '../stores/tasks'

const tasks = useTasksStore()
const maximized = ref(false)
// null outside Tauri (browser mock): the buttons are then inert instead of throwing
let win: Window | null = null
let unlisten: (() => void) | undefined

onMounted(async () => {
  if (!isTauri) return
  const { getCurrentWindow } = await import('@tauri-apps/api/window')
  const current = getCurrentWindow()
  win = current
  maximized.value = await current.isMaximized()
  unlisten = await current.onResized(async () => {
    maximized.value = await current.isMaximized()
  })
})
onBeforeUnmount(() => unlisten?.())

const run = (action: (w: Window) => Promise<void>) => win && tasks.guard(() => action(win as Window))
const close = () => run((w) => w.close())
const minimize = () => run((w) => w.minimize())
const toggleMaximize = () => run((w) => w.toggleMaximize())
</script>

<template>
  <!-- data-tauri-drag-region also makes a double click toggle maximize -->
  <header data-tauri-drag-region class="relative flex h-9 shrink-0 items-center border-b border-line bg-surface px-3.5 select-none">
    <div class="lights flex items-center gap-2" role="group" :aria-label="t.titlebar.window">
      <button class="light bg-[#ff5f57]" :aria-label="t.common.close" @click="close"><X :size="8" :stroke-width="3.5" /></button>
      <button class="light bg-[#febc2e]" :aria-label="t.titlebar.minimize" @click="minimize"><Minus :size="8" :stroke-width="3.5" /></button>
      <button class="light bg-[#28c840]" :aria-label="maximized ? t.titlebar.restore : t.titlebar.maximize" @click="toggleMaximize">
        <Shrink v-if="maximized" :size="8" :stroke-width="3" />
        <Plus v-else :size="8" :stroke-width="3.5" />
      </button>
    </div>
    <span data-tauri-drag-region class="pointer-events-none absolute inset-x-0 text-center font-mono text-[12px] font-semibold tracking-tight text-muted">_k0t1k</span>
  </header>
</template>

<style scoped>
.light {
  display: grid;
  place-items: center;
  width: 12px;
  height: 12px;
  border-radius: 50%;
  border: 1px solid rgb(0 0 0 / 0.18);
  color: rgb(0 0 0 / 0.65);
  cursor: pointer;
}
.light > :deep(svg) {
  opacity: 0;
}
.lights:hover .light > :deep(svg),
.lights:focus-within .light > :deep(svg) {
  opacity: 1;
}
.light:active {
  filter: brightness(0.85);
}
</style>
