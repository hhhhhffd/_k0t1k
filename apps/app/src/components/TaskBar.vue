<script setup lang="ts">
import { AlertTriangle, Check, Loader } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'

const tasks = useTasksStore()
const instances = useInstancesStore()

const isLaunch = (id: string) => id.startsWith('launch:')
const launching = computed(() => tasks.tasks.some((task) => isLaunch(task.task)))
// Latched when a launch begins: the instance's last-played time may be written while it still runs.
const firstLaunch = ref(false)
watch(launching, (now) => {
  firstLaunch.value = now && instances.loaded && instances.instances.every((i) => !i.lastPlayed)
})

function etaText(id: string) {
  const secs = tasks.etaSecs(id)
  if (secs === null) return ''
  return secs < 90 ? t.tasks.etaSeconds(Math.max(5, Math.round(secs / 5) * 5)) : t.tasks.etaMinutes(Math.round(secs / 60))
}
</script>

<template>
  <div v-if="tasks.tasks.length" class="anim-slide relative z-[45] flex flex-col gap-2 border-t border-line bg-surface px-4 py-2" aria-live="polite">
    <div v-for="task in tasks.tasks" :key="task.task" class="flex items-center gap-3">
      <AlertTriangle v-if="task.error" :size="15" class="text-danger" />
      <Check v-else-if="task.finished" :size="15" class="text-ok" />
      <Loader v-else :size="15" class="animate-spin text-accent" />
      <span class="w-56 shrink-0 truncate font-medium">{{ task.title ?? task.task }}</span>
      <span class="w-44 shrink-0 truncate text-muted">{{ task.error ?? task.stage }}</span>
      <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-raised" role="progressbar" :aria-label="t.tasks.title" :aria-valuenow="task.done" :aria-valuemax="task.total">
        <div
          class="h-full rounded-full transition-[width]"
          :class="[task.error ? 'bg-danger' : 'bg-accent', !task.total && !task.finished && !task.error && 'animate-pulse']"
          :style="{ width: `${task.total ? Math.min(100, (task.done / task.total) * 100) : 100}%` }"
        />
      </div>
      <span class="w-12 text-right text-[12px] text-faint">{{ task.total ? `${task.done}/${task.total}` : task.done }}</span>
      <span class="w-24 shrink-0 text-right text-[12px] text-faint">{{ etaText(task.task) }}</span>
    </div>
    <p v-if="firstLaunch && launching" class="text-[12px] text-muted">{{ t.tasks.firstLaunch }}</p>
  </div>
</template>
