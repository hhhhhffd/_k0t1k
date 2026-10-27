<script setup lang="ts">
import { X } from '@lucide/vue'
import { onBeforeUnmount, onMounted } from 'vue'
import { t } from '../i18n'

/** Widths grow with the window; `large` fills most of it. */
withDefaults(defineProps<{ title: string; size?: 'sm' | 'md' | 'lg'; large?: boolean }>(), { size: 'md' })
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
        class="card anim-pop flex max-h-full max-w-full flex-col overflow-hidden shadow-[var(--shadow-pop)]"
        :class="large ? 'dialog-large' : `dialog-${size}`"
        role="dialog"
        aria-modal="true"
        :aria-label="title"
      >
        <header class="flex items-center justify-between border-b border-line px-4 py-3">
          <h2 class="text-[15px] font-semibold">{{ title }}</h2>
          <button class="btn btn-ghost btn-icon" :aria-label="t.common.closeLabel" @click="emit('close')"><X :size="16" /></button>
        </header>
        <div class="min-h-0 flex-1" :class="large ? 'flex flex-col' : 'overflow-y-auto p-4'"><slot /></div>
        <footer v-if="$slots.footer" class="flex flex-wrap justify-end gap-2 border-t border-line px-4 py-3"><slot name="footer" /></footer>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.dialog-sm {
  width: clamp(400px, 32vw, 680px);
}
.dialog-md {
  width: clamp(540px, 46vw, 1040px);
}
.dialog-lg {
  width: clamp(760px, 70vw, 1500px);
}
.dialog-large {
  width: min(1700px, 94vw);
  height: 90vh;
}
</style>
