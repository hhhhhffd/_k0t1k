<script setup lang="ts">
import { ChevronLeft, ChevronRight, ExternalLink, Trash2, X } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted } from 'vue'
import * as api from '../../api/commands'
import type { Screenshot } from '../../api/types'
import { t } from '../../i18n'
import { formatBytes, formatRelative } from '../../utils/format'

const props = defineProps<{ shots: Screenshot[]; index: number }>()
const emit = defineEmits<{ close: []; 'update:index': [index: number]; open: [shot: Screenshot]; delete: [shot: Screenshot] }>()

const shot = computed(() => props.shots[props.index])
const step = (delta: number) => {
  const count = props.shots.length
  if (count) emit('update:index', (props.index + delta + count) % count)
}

function onKey(e: KeyboardEvent) {
  // a confirmation dialog on top handles its own keys
  if (document.querySelector('[role="dialog"][aria-modal="true"]')) return
  if (e.key === 'Escape') emit('close')
  else if (e.key === 'ArrowLeft') step(-1)
  else if (e.key === 'ArrowRight') step(1)
  else if (e.key === 'Delete' && shot.value) emit('delete', shot.value)
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <Teleport to="body">
    <div v-if="shot" class="anim-fade fixed inset-0 z-50 flex flex-col bg-black/95 backdrop-blur-sm" role="region" :aria-label="shot.fileName">
      <header class="flex shrink-0 items-center gap-3 px-4 py-3 text-white">
        <div class="min-w-0 flex-1">
          <p class="truncate font-semibold">{{ shot.fileName }}</p>
          <p class="text-[12px] text-white/60">
            {{ index + 1 }} / {{ shots.length }} · {{ formatBytes(shot.size) }} · {{ formatRelative(new Date(shot.modified).toISOString()) }}
          </p>
        </div>
        <button class="btn" @click="emit('open', shot)"><ExternalLink :size="15" />{{ t.shots.open }}</button>
        <button class="btn btn-danger" @click="emit('delete', shot)"><Trash2 :size="15" />{{ t.common.delete }}</button>
        <button class="btn btn-ghost btn-icon text-white" :aria-label="t.common.closeLabel" @click="emit('close')"><X :size="18" /></button>
      </header>
      <div class="relative flex min-h-0 flex-1 items-center justify-center px-16 pb-6" @mousedown.self="emit('close')">
        <img :src="api.fileUrl(shot.path)" :alt="shot.fileName" class="h-full w-full object-contain" />
        <template v-if="shots.length > 1">
          <button class="nav left-3" :aria-label="t.shots.previous" @click="step(-1)"><ChevronLeft :size="26" /></button>
          <button class="nav right-3" :aria-label="t.shots.next" @click="step(1)"><ChevronRight :size="26" /></button>
        </template>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.nav {
  position: absolute;
  top: 50%;
  display: flex;
  width: 44px;
  height: 44px;
  transform: translateY(-50%);
  align-items: center;
  justify-content: center;
  border-radius: 999px;
  background: rgb(255 255 255 / 0.1);
  color: white;
  transition: background 0.12s;
}
.nav:hover {
  background: rgb(255 255 255 / 0.22);
}
</style>
