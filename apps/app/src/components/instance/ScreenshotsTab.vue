<script setup lang="ts">
import { FolderOpen } from '@lucide/vue'
import { onMounted, ref, watch } from 'vue'
import * as api from '../../api/commands'
import type { Screenshot } from '../../api/types'
import { t } from '../../i18n'
import { useTasksStore } from '../../stores/tasks'
import ConfirmDialog from '../ConfirmDialog.vue'
import ScreenshotViewer from './ScreenshotViewer.vue'

const props = defineProps<{ instanceId: string; shared: boolean }>()
const tasks = useTasksStore()

const shots = ref<Screenshot[]>([])
const loading = ref(true)
/** Index of the screenshot open in the viewer. */
const viewing = ref<number | null>(null)
const deleting = ref<Screenshot | null>(null)

async function load() {
  loading.value = true
  shots.value = (await tasks.guard(() => api.listScreenshots(props.instanceId))) ?? []
  loading.value = false
}
onMounted(load)
watch(() => props.instanceId, load)

const openFolder = () => tasks.guard(() => api.openScreenshotsFolder(props.instanceId))
const openShot = (shot: Screenshot) => tasks.guard(() => api.openScreenshot(props.instanceId, shot.fileName))

async function confirmDelete() {
  const shot = deleting.value
  deleting.value = null
  if (!shot) return
  const done = await tasks.guard(async () => {
    await api.deleteScreenshot(props.instanceId, shot.fileName)
    return true
  })
  if (!done) return
  await load()
  // the viewer stays open on the next screenshot, or closes with the last one gone
  if (viewing.value !== null) viewing.value = shots.value.length ? Math.min(viewing.value, shots.value.length - 1) : null
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <p v-if="shared" class="flex items-center gap-2 text-muted">
      <span class="badge badge-accent">{{ t.sharedFolder.badge }}</span>{{ t.shots.sharedNote }}
    </p>
    <div class="flex items-center gap-2">
      <span class="mr-auto text-faint">{{ t.shots.count(shots.length) }}</span>
      <button class="btn" @click="openFolder"><FolderOpen :size="15" />{{ t.shots.openFolder }}</button>
    </div>

    <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
    <p v-else-if="!shots.length" class="py-10 text-center text-faint">{{ t.shots.empty }}</p>
    <ul v-else class="grid grid-cols-[repeat(auto-fill,minmax(clamp(190px,15vw,340px),1fr))] gap-2.5">
      <li v-for="(s, i) in shots" :key="s.fileName">
        <button class="card group block w-full overflow-hidden text-left transition-colors hover:border-accent/60" @click="viewing = i">
          <img :src="api.fileUrl(s.path)" :alt="s.fileName" loading="lazy" decoding="async" class="aspect-video w-full bg-raised object-cover" />
          <span class="block truncate px-2 py-1.5 text-[12px] text-muted">{{ s.fileName }}</span>
        </button>
      </li>
    </ul>

    <ScreenshotViewer
      v-if="viewing !== null"
      :index="viewing"
      @update:index="viewing = $event"
      :shots="shots"
      @open="openShot"
      @delete="deleting = $event"
      @close="viewing = null"
    />

    <ConfirmDialog v-if="deleting" :title="t.shots.delete" :text="deleting.fileName" @confirm="confirmDelete" @close="deleting = null" />
  </div>
</template>
