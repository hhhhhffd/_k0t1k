<script setup lang="ts">
import { AlertTriangle, Check, Loader } from '@lucide/vue'
import { t } from '../i18n/ru'
import { useTasksStore } from '../stores/tasks'

const tasks = useTasksStore()
</script>

<template>
  <div v-if="tasks.tasks.length" class="anim-slide flex flex-col gap-2 border-t border-line bg-surface px-4 py-2" aria-live="polite">
    <div v-for="task in tasks.tasks" :key="task.task" class="flex items-center gap-3">
      <AlertTriangle v-if="task.error" :size="15" class="text-danger" />
      <Check v-else-if="task.finished" :size="15" class="text-ok" />
      <Loader v-else :size="15" class="animate-spin text-accent" />
      <span class="w-56 shrink-0 truncate font-medium">{{ task.title ?? task.task }}</span>
      <span class="w-44 shrink-0 truncate text-muted">{{ task.error ?? task.stage }}</span>
      <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-raised" role="progressbar" :aria-label="t.tasks.title" :aria-valuenow="task.done" :aria-valuemax="task.total">
        <div
          class="h-full rounded-full transition-[width]"
          :class="task.error ? 'bg-danger' : 'bg-accent'"
          :style="{ width: `${task.total ? Math.min(100, (task.done / task.total) * 100) : 100}%` }"
        />
      </div>
      <span class="w-12 text-right text-[12px] text-faint">{{ task.done }}/{{ task.total }}</span>
    </div>
  </div>
</template>
