<script setup lang="ts">
import { save } from '@tauri-apps/plugin-dialog'
import { computed, onMounted, ref } from 'vue'
import * as api from '../api/commands'
import type { ExportEntry, ExportFormat, Instance } from '../api/types'
import { t } from '../i18n/ru'
import { useTasksStore } from '../stores/tasks'
import ExportTree from './ExportTree.vue'
import ModalDialog from './ModalDialog.vue'

const props = defineProps<{ instance: Instance; format: ExportFormat }>()
const emit = defineEmits<{ close: [] }>()
const tasks = useTasksStore()

const formatLabel = { instance: t.transfer.exportInstance, modrinth: t.transfer.exportModrinth, curseForge: t.transfer.exportCurseForge }
const isPack = computed(() => props.format !== 'instance')

const name = ref(props.instance.name)
const version = ref('1.0.0')
const summary = ref('')
const entries = ref<ExportEntry[]>([])
const excluded = ref(new Set<string>())
const loading = ref(true)

onMounted(async () => {
  const root = await tasks.guard(() => api.listExportEntries(props.instance.id, ''))
  if (root) {
    entries.value = root
    excluded.value = new Set(root.filter((entry) => entry.defaultExcluded).map((entry) => entry.path))
  }
  loading.value = false
})

function toggle(path: string) {
  const next = new Set(excluded.value)
  if (!next.delete(path)) next.add(path)
  excluded.value = next
}

const canExport = computed(() => !loading.value && !!name.value.trim() && (!isPack.value || !!version.value.trim()))

async function run() {
  const extension = props.format === 'modrinth' ? 'mrpack' : 'zip'
  const base = (isPack.value ? `${name.value.trim()}-${version.value.trim()}` : name.value.trim()).replace(/[\\/:*?"<>|]/g, '_')
  const destination = api.isTauri
    ? await save({
        defaultPath: `${base}.${extension}`,
        filters: [{ name: isPack.value ? t.transfer.packFilter : t.transfer.instanceFilter, extensions: [extension] }],
      })
    : `C:\\Exports\\${base}.${extension}`
  if (!destination) return
  emit('close')
  const report = await tasks.guard(() =>
    api.exportInstance(props.instance.id, {
      format: props.format,
      destination,
      name: name.value.trim(),
      version: version.value.trim(),
      summary: summary.value.trim(),
      excluded: [...excluded.value],
    }),
  )
  if (report === undefined) return
  tasks.toast(report ? t.transfer.exportedPack(report.listed, report.overrides) : t.transfer.exportedZip, 'ok')
}
</script>

<template>
  <ModalDialog :title="t.transfer.exportTitle(formatLabel[format])" :width="620" @close="emit('close')">
    <div class="flex flex-col gap-3">
      <label class="flex flex-col gap-1">
        <span class="label">{{ t.transfer.packName }}</span>
        <input v-model="name" class="input" />
      </label>
      <template v-if="isPack">
        <label class="flex flex-col gap-1">
          <span class="label">{{ t.transfer.packVersion }}</span>
          <input v-model="version" class="input" />
        </label>
        <label class="flex flex-col gap-1">
          <span class="label">{{ t.transfer.packSummary }}</span>
          <input v-model="summary" class="input" />
        </label>
      </template>
      <div class="flex flex-col gap-1">
        <span class="label">{{ t.transfer.filesToInclude }}</span>
        <p class="text-[12px] text-faint">{{ t.transfer.filesHint }}</p>
        <div class="card max-h-72 overflow-y-auto p-1.5">
          <p v-if="loading" class="py-6 text-center text-faint">{{ t.common.loading }}</p>
          <ExportTree v-else :instance-id="instance.id" :entries="entries" :excluded="excluded" @toggle="toggle" />
        </div>
      </div>
    </div>
    <template #footer>
      <button class="btn" @click="emit('close')">{{ t.common.cancel }}</button>
      <button class="btn btn-primary" :disabled="!canExport" @click="run">{{ t.transfer.saveAs }}</button>
    </template>
  </ModalDialog>
</template>
