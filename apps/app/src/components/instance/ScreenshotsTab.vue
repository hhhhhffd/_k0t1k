<script setup lang="ts">
import { ExternalLink, FolderOpen, Trash2 } from '@lucide/vue'
import { onMounted, ref, watch } from 'vue'
import * as api from '../../api/commands'
import type { Screenshot } from '../../api/types'
import { t } from '../../i18n'
import { useTasksStore } from '../../stores/tasks'
import { formatBytes, formatRelative } from '../../utils/format'
import ConfirmDialog from '../ConfirmDialog.vue'
import ModalDialog from '../ModalDialog.vue'

const props = defineProps<{ instanceId: string; shared: boolean }>()
const tasks = useTasksStore()

const shots = ref<Screenshot[]>([])
const loading = ref(true)
const previewed = ref<Screenshot | null>(null)
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
  if (previewed.value?.fileName === shot.fileName) previewed.value = null
  await load()
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
    <ul v-else class="grid grid-cols-[repeat(auto-fill,minmax(190px,1fr))] gap-2.5">
      <li v-for="s in shots" :key="s.fileName">
        <button class="card group block w-full overflow-hidden text-left transition-colors hover:border-accent/60" @click="previewed = s">
          <img :src="api.fileUrl(s.path)" :alt="s.fileName" loading="lazy" decoding="async" class="aspect-video w-full bg-raised object-cover" />
          <span class="block truncate px-2 py-1.5 text-[12px] text-muted">{{ s.fileName }}</span>
        </button>
      </li>
    </ul>

    <ModalDialog v-if="previewed" :title="previewed.fileName" :width="960" @close="previewed = null">
      <img :src="api.fileUrl(previewed.path)" :alt="previewed.fileName" class="mx-auto max-h-[65vh] w-auto max-w-full rounded-md" />
      <p class="mt-2 text-center text-[12px] text-faint">{{ formatBytes(previewed.size) }} · {{ formatRelative(new Date(previewed.modified).toISOString()) }}</p>
      <template #footer>
        <button class="btn btn-danger" @click="deleting = previewed"><Trash2 :size="15" />{{ t.common.delete }}</button>
        <button class="btn btn-primary" @click="openShot(previewed)"><ExternalLink :size="15" />{{ t.shots.open }}</button>
      </template>
    </ModalDialog>

    <ConfirmDialog v-if="deleting" :title="t.shots.delete" :text="deleting.fileName" @confirm="confirmDelete" @close="deleting = null" />
  </div>
</template>
