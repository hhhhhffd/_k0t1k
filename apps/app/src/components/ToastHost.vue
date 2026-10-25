<script setup lang="ts">
import { useTasksStore } from '../stores/tasks'

const tasks = useTasksStore()
const accent = { info: 'border-accent', ok: 'border-ok', error: 'border-danger' } as const
</script>

<template>
  <div class="pointer-events-none fixed right-4 bottom-4 z-[70] flex w-80 flex-col gap-2" aria-live="assertive">
    <div v-for="toast in tasks.toasts" :key="toast.id" class="card anim-slide pointer-events-auto flex flex-col gap-2 border-l-4 px-3 py-2 text-left shadow-[var(--shadow-pop)]" :class="accent[toast.kind]">
      <button class="text-left" @click="tasks.dismiss(toast.id)">{{ toast.text }}</button>
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
