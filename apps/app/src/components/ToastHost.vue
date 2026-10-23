<script setup lang="ts">
import { useTasksStore } from '../stores/tasks'

const tasks = useTasksStore()
const accent = { info: 'border-accent', ok: 'border-ok', error: 'border-danger' } as const
</script>

<template>
  <div class="pointer-events-none fixed right-4 bottom-4 z-[70] flex w-80 flex-col gap-2" aria-live="assertive">
    <button
      v-for="toast in tasks.toasts"
      :key="toast.id"
      class="card anim-slide pointer-events-auto border-l-4 px-3 py-2 text-left shadow-[var(--shadow-pop)]"
      :class="accent[toast.kind]"
      @click="tasks.dismiss(toast.id)"
    >
      {{ toast.text }}
    </button>
  </div>
</template>
