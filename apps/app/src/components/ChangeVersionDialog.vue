<script setup lang="ts">
import { Check, Download, ExternalLink } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import * as api from '../api/commands'
import type { LocalResource, LocalResourceKind, ProjectVersion, VersionFilter } from '../api/types'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { openExternal } from '../utils/external'
import { formatBytes, formatDate, loaderOf, minecraftVersionOf, originProject } from '../utils/format'
import ModalDialog from './ModalDialog.vue'
import ToggleSwitch from './ToggleSwitch.vue'

/** Installs another version of a catalog file; core replaces the old file of the project. */
const props = defineProps<{ instanceId: string; kind: LocalResourceKind; resource: LocalResource }>()
const emit = defineEmits<{ close: []; changed: [fileNames: string[]] }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const project = computed(() => (props.resource.origin ? originProject(props.resource.origin) : undefined))
const versions = ref<ProjectVersion[]>([])
const loading = ref(true)
const showAll = ref(false)
const busy = ref(false)

const requirement = computed<VersionFilter>(() => {
  const inst = store.byId(props.instanceId)
  if (!inst) return {}
  return { gameVersion: minecraftVersionOf(inst), loader: props.kind === 'mod' ? loaderOf(inst)?.id : undefined }
})

let loadSeq = 0
async function load() {
  const target = project.value
  if (!target) return
  const seq = ++loadSeq
  loading.value = true
  const found = (await tasks.guard(() => api.getProjectVersions(target.platform, target.projectId, showAll.value ? {} : requirement.value))) ?? []
  if (seq !== loadSeq) return
  versions.value = found
  loading.value = false
}
onMounted(load)
watch(showAll, load)

async function install(version: ProjectVersion) {
  busy.value = true
  const report = await tasks.guard(() => api.installVersion(props.instanceId, version, props.kind))
  busy.value = false
  if (!report) return
  tasks.showBlocked(report.blocked, props.instanceId)
  tasks.toast(t.instance.versionChanged(version.versionNumber), 'ok')
  emit('changed', report.installed.map((i) => i.fileName))
  emit('close')
}

const releaseLabel = { release: t.browse.release, beta: t.browse.beta, alpha: t.browse.alpha }
</script>

<template>
  <ModalDialog :title="t.instance.changeVersionTitle(resource.metadata?.name ?? resource.fileName)" size="md" @close="emit('close')">
    <div class="flex flex-col gap-3">
      <label class="flex items-center gap-2 text-muted">
        <ToggleSwitch v-model="showAll" :label="t.instance.showAllVersions" />
        {{ t.instance.showAllVersions }}
      </label>
      <p v-if="loading" class="py-6 text-center text-faint">{{ t.common.loading }}</p>
      <p v-else-if="!versions.length" class="py-6 text-center text-faint">{{ t.instance.noVersions }}</p>
      <ul v-else class="flex max-h-[55vh] flex-col gap-1.5 overflow-y-auto">
        <li v-for="v in versions" :key="v.id" class="card flex items-center gap-3 bg-raised px-3 py-2">
          <div class="min-w-0 flex-1">
            <div class="flex items-center gap-2">
              <span class="truncate font-semibold">{{ v.versionNumber }}</span>
              <span class="badge" :class="v.releaseType === 'release' ? 'badge-accent' : ''">{{ releaseLabel[v.releaseType] }}</span>
              <span v-if="v.id === project?.versionId" class="badge badge-ok"><Check :size="12" />{{ t.instance.installedVersion }}</span>
            </div>
            <div class="truncate text-[12px] text-faint">
              {{ v.gameVersions.join(', ') }} · {{ formatBytes(v.files[0]?.size ?? 0) }} · {{ formatDate(v.published) }}
            </div>
          </div>
          <button
            v-if="v.files.some((f) => f.url === null)"
            class="btn h-7"
            @click="tasks.guard(() => openExternal(v.files.find((f) => f.url === null)?.downloadPage ?? ''))"
          >
            <ExternalLink :size="14" />{{ t.browse.blockedLink }}
          </button>
          <button v-else class="btn btn-primary h-7" :disabled="busy || v.id === project?.versionId" @click="install(v)">
            <Download :size="14" />{{ t.instance.installVersion }}
          </button>
        </li>
      </ul>
    </div>
  </ModalDialog>
</template>
