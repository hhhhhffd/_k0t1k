<script setup lang="ts">
import type { Window } from '@tauri-apps/api/window'
import { Minus, Plus, Shrink, X } from '@lucide/vue'
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { isTauri } from '../api/commands'
import { t } from '../i18n'
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
  <header data-tauri-drag-region class="relative flex h-10 shrink-0 items-center border-b border-line bg-surface px-1.5 select-none">
    <div class="lights flex items-center" role="group" :aria-label="t.titlebar.window">
      <button class="light" style="--light: #ff5f57" :aria-label="t.common.close" @click="close"><X :size="10" :stroke-width="3.5" /></button>
      <button class="light" style="--light: #febc2e" :aria-label="t.titlebar.minimize" @click="minimize"><Minus :size="10" :stroke-width="3.5" /></button>
      <button class="light" style="--light: #28c840" :aria-label="maximized ? t.titlebar.restore : t.titlebar.maximize" @click="toggleMaximize">
        <Shrink v-if="maximized" :size="10" :stroke-width="3" />
        <Plus v-else :size="10" :stroke-width="3.5" />
      </button>
    </div>
    <span data-tauri-drag-region class="pointer-events-none absolute inset-x-0 text-center font-mono text-[12px] font-semibold tracking-tight text-muted">_k0t1k</span>
  </header>
</template>

<style scoped>
/* The button is a 28px hit target; the visible macOS-style dot is drawn by ::before. */
.light {
  position: relative;
  display: grid;
  place-items: center;
  width: 28px;
  height: 28px;
  border-radius: 8px;
  color: rgb(0 0 0 / 0.65);
  cursor: pointer;
}
.light::before {
  content: '';
  position: absolute;
  inset: 6px;
  border-radius: 50%;
  background: var(--light);
  border: 1px solid rgb(0 0 0 / 0.18);
}
.light > :deep(svg) {
  position: relative;
  opacity: 0;
}
.lights:hover .light > :deep(svg),
.lights:focus-within .light > :deep(svg) {
  opacity: 1;
}
.light:hover {
  background: color-mix(in srgb, var(--c-fg) 8%, transparent);
}
.light:active::before {
  filter: brightness(0.85);
}
</style>
