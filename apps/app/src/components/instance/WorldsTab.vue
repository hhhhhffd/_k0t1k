<script setup lang="ts">
import { Globe } from '@lucide/vue'
import { onMounted, ref } from 'vue'
import { listWorlds } from '../../api/commands'
import type { World } from '../../api/types'
import { t } from '../../i18n/ru'
import { useTasksStore } from '../../stores/tasks'
import { formatBytes, formatRelative } from '../../utils/format'

const props = defineProps<{ instanceId: string }>()
const tasks = useTasksStore()
const worlds = ref<World[]>([])
const loading = ref(true)

onMounted(async () => {
  worlds.value = (await tasks.guard(() => listWorlds(props.instanceId))) ?? []
  loading.value = false
})
</script>

<template>
  <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
  <p v-else-if="!worlds.length" class="py-10 text-center text-faint">{{ t.instance.worldsEmpty }}</p>
  <ul v-else class="flex flex-col gap-1.5">
    <li v-for="w in worlds" :key="w.dirName" class="card flex items-center gap-3 px-3 py-2.5">
      <span class="pix flex size-9 items-center justify-center bg-ok/15 text-ok"><Globe :size="17" /></span>
      <div class="min-w-0 flex-1">
        <div class="truncate font-semibold">{{ w.name }}</div>
        <div class="truncate text-[12px] text-faint">{{ w.dirName }}</div>
      </div>
      <span class="badge">{{ t.instance.gameMode[w.gameMode] ?? w.gameMode }}</span>
      <span class="w-24 text-right text-muted">{{ formatBytes(w.size) }}</span>
      <span class="w-28 text-right text-muted">{{ formatRelative(w.lastPlayed) }}</span>
    </li>
  </ul>
</template>
