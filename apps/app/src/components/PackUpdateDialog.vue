<script setup lang="ts">
import { ArrowUpCircle, Download } from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { getProjectVersions } from '../api/commands'
import type { Instance, Platform, ProjectVersion } from '../api/types'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { formatDate } from '../utils/format'
import ModalDialog from './ModalDialog.vue'

const props = defineProps<{ instance: Instance }>()
const emit = defineEmits<{ close: [] }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const pack = computed(() => props.instance.managedPack!)
const versions = ref<ProjectVersion[]>([])
const loading = ref(true)

onMounted(async () => {
  const platform: Platform = pack.value.platform === 'modrinth' ? 'modrinth' : 'curseForge'
  const found = await tasks.guard(() => getProjectVersions(platform, pack.value.projectId, {}))
  versions.value = (found ?? []).sort((a, b) => b.published.localeCompare(a.published))
  loading.value = false
})

const currentPublished = computed(() => versions.value.find((v) => v.id === pack.value.versionId)?.published)
const isNewer = (v: ProjectVersion) => !!currentPublished.value && v.published > currentPublished.value
const hasNewer = computed(() => versions.value.some(isNewer))

async function apply(v: ProjectVersion) {
  emit('close')
  const updated = await tasks.guard(() => store.updateModpack(props.instance.id, v.id))
  if (updated) tasks.toast(t.pack.done(v.versionNumber), 'ok')
}
</script>

<template>
  <ModalDialog :title="t.pack.title" :width="620" @close="emit('close')">
    <p class="mb-1 text-muted">
      {{ pack.name }} · {{ t.pack.current }} <span class="font-semibold text-fg">{{ pack.versionName }}</span>
    </p>
    <p class="mb-3 text-[12px] text-faint">{{ t.pack.note }}</p>
    <p v-if="loading" class="py-8 text-center text-faint">{{ t.pack.loading }}</p>
    <template v-else>
      <p v-if="!versions.length" class="py-8 text-center text-faint">{{ t.pack.noVersions }}</p>
      <p v-else-if="!hasNewer" class="mb-2 text-ok">{{ t.pack.upToDate }}</p>
      <ul class="flex flex-col gap-1.5">
        <li v-for="v in versions" :key="v.id" class="card flex items-center gap-3 px-3 py-2">
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="truncate font-semibold">{{ v.versionNumber }}</span>
              <span v-if="v.releaseType !== 'release'" class="badge">{{ v.releaseType }}</span>
              <span v-if="isNewer(v)" class="badge badge-accent">{{ t.pack.newer }}</span>
              <span v-if="v.id === pack.versionId" class="badge">{{ t.pack.installed }}</span>
            </div>
            <div class="truncate text-[12px] text-faint">{{ v.gameVersions.slice(0, 3).join(', ') }} · {{ formatDate(v.published) }}</div>
          </div>
          <button v-if="v.id !== pack.versionId" class="btn h-7" :class="isNewer(v) ? 'btn-primary' : ''" @click="apply(v)">
            <component :is="isNewer(v) ? ArrowUpCircle : Download" :size="14" />{{ isNewer(v) ? t.pack.update : t.pack.install }}
          </button>
        </li>
      </ul>
    </template>
    <template #footer>
      <button class="btn" @click="emit('close')">{{ t.common.close }}</button>
    </template>
  </ModalDialog>
</template>
