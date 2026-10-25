<script setup lang="ts">
import { ChevronDown, ChevronRight, File, Folder } from '@lucide/vue'
import { reactive, ref } from 'vue'
import { listExportEntries } from '../api/commands'
import type { ExportEntry } from '../api/types'
import { t } from '../i18n'
import { useTasksStore } from '../stores/tasks'
import { formatBytes } from '../utils/format'

const props = defineProps<{
  instanceId: string
  entries: ExportEntry[]
  /** Paths left out of the export; each also leaves out everything below it. */
  excluded: Set<string>
  /** A folder above is already left out, so these rows are off and cannot be switched on alone. */
  ancestorExcluded?: boolean
}>()
const emit = defineEmits<{ toggle: [path: string] }>()
const tasks = useTasksStore()

const open = ref(new Set<string>())
const children = reactive(new Map<string, ExportEntry[]>())

async function toggleOpen(entry: ExportEntry) {
  const next = new Set(open.value)
  if (next.delete(entry.path)) {
    open.value = next
    return
  }
  if (!children.has(entry.path)) {
    const loaded = await tasks.guard(() => listExportEntries(props.instanceId, entry.path))
    if (!loaded) return
    children.set(entry.path, loaded)
  }
  next.add(entry.path)
  open.value = next
}
</script>

<template>
  <ul class="flex flex-col">
    <li v-for="entry in entries" :key="entry.path">
      <div class="flex h-7 items-center gap-1.5 rounded-md px-1 hover:bg-hover" :class="ancestorExcluded ? 'opacity-50' : ''">
        <button
          v-if="entry.isDir"
          class="btn btn-ghost btn-icon !size-5 text-faint"
          :aria-label="entry.name"
          :aria-expanded="open.has(entry.path)"
          @click="toggleOpen(entry)"
        >
          <component :is="open.has(entry.path) ? ChevronDown : ChevronRight" :size="14" />
        </button>
        <span v-else class="size-5 shrink-0" />
        <input
          type="checkbox"
          class="accent-accent"
          :aria-label="entry.name"
          :checked="!ancestorExcluded && !excluded.has(entry.path)"
          :disabled="ancestorExcluded"
          @change="emit('toggle', entry.path)"
        />
        <component :is="entry.isDir ? Folder : File" :size="14" class="shrink-0 text-faint" />
        <span class="truncate">{{ entry.name }}</span>
        <span v-if="entry.linked" class="badge badge-accent" :title="t.transfer.linkedHint">{{ t.transfer.sharedBadge }}</span>
        <span v-if="entry.size !== null" class="ml-auto shrink-0 text-[12px] text-faint">{{ formatBytes(entry.size) }}</span>
      </div>
      <div v-if="entry.isDir && open.has(entry.path)" class="ml-4 border-l border-line pl-1">
        <ExportTree
          v-if="children.get(entry.path)?.length"
          :instance-id="instanceId"
          :entries="children.get(entry.path)!"
          :excluded="excluded"
          :ancestor-excluded="ancestorExcluded || excluded.has(entry.path)"
          @toggle="emit('toggle', $event)"
        />
        <p v-else class="px-6 py-1 text-[12px] text-faint">{{ t.transfer.folderEmpty }}</p>
      </div>
    </li>
  </ul>
</template>
