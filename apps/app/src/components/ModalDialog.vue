<script setup lang="ts">
import { X } from '@lucide/vue'
import { onBeforeUnmount, onMounted } from 'vue'
import { t } from '../i18n'

/** `large` makes the dialog follow the window size instead of a fixed width. */
const props = withDefaults(defineProps<{ title: string; width?: number; large?: boolean }>(), { width: 560 })
const emit = defineEmits<{ close: [] }>()

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
</script>

<template>
  <Teleport to="body">
    <div class="anim-fade fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6 backdrop-blur-sm" @mousedown.self="emit('close')">
      <div
        class="card anim-pop flex max-h-full w-full flex-col overflow-hidden shadow-[var(--shadow-pop)]"
        :style="large ? { width: 'min(1500px, 94vw)', height: '90vh' } : { maxWidth: `${props.width}px` }"
        role="dialog"
        aria-modal="true"
        :aria-label="title"
      >
        <header class="flex items-center justify-between border-b border-line px-4 py-3">
          <h2 class="text-[15px] font-semibold">{{ title }}</h2>
          <button class="btn btn-ghost btn-icon" :aria-label="t.common.closeLabel" @click="emit('close')"><X :size="16" /></button>
        </header>
        <div class="min-h-0 flex-1" :class="large ? 'flex flex-col' : 'overflow-y-auto p-4'"><slot /></div>
        <footer v-if="$slots.footer" class="flex justify-end gap-2 border-t border-line px-4 py-3"><slot name="footer" /></footer>
      </div>
    </div>
  </Teleport>
</template>
