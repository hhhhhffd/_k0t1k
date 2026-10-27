<script setup lang="ts">
import { X } from '@lucide/vue'
import { t } from '../i18n'
import { useTasksStore } from '../stores/tasks'

const tasks = useTasksStore()
const accent = { info: 'border-accent', ok: 'border-ok', error: 'border-danger' } as const
</script>

<template>
  <div class="pointer-events-none fixed right-4 bottom-4 z-[70] flex w-80 flex-col gap-2" aria-live="assertive">
    <div v-for="toast in tasks.toasts" :key="toast.id" class="card anim-slide pointer-events-auto flex flex-col gap-2 border-l-4 py-2 pr-2 pl-3 text-left shadow-[var(--shadow-pop)]" :class="accent[toast.kind]">
      <div class="flex items-start gap-2">
        <p class="min-w-0 flex-1 break-words">{{ toast.text }}</p>
        <button class="-mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-md text-faint transition-colors hover:bg-hover hover:text-fg" :title="t.common.close" :aria-label="t.common.close" @click="tasks.dismiss(toast.id)">
          <X :size="14" />
        </button>
      </div>
      <button
        v-if="toast.action"
        class="btn btn-primary self-start"
        @click="toast.action.run(); tasks.dismiss(toast.id)"
      >
        {{ toast.action.label }}
      </button>
    </div>
  </div>
</template>
