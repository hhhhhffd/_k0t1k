<script setup lang="ts">
import { ArrowUpCircle, Plus, RefreshCw, Search, Trash2 } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import * as api from '../../api/commands'
import type { AvailableUpdate, LocalResource, LocalResourceKind } from '../../api/types'
import { t } from '../../i18n/ru'
import { useTasksStore } from '../../stores/tasks'
import { formatBytes, originPlatform } from '../../utils/format'
import ConfirmDialog from '../ConfirmDialog.vue'
import ToggleSwitch from '../ToggleSwitch.vue'

const props = defineProps<{ instanceId: string; kind: LocalResourceKind }>()
const router = useRouter()
const tasks = useTasksStore()

const resources = ref<LocalResource[]>([])
const loading = ref(true)
const filter = ref('')
const updates = ref<Map<string, AvailableUpdate>>(new Map())
const checking = ref(false)
const toDelete = ref<LocalResource | null>(null)

const visible = computed(() => {
  const needle = filter.value.trim().toLowerCase()
  return resources.value.filter(
    (r) => !needle || r.fileName.toLowerCase().includes(needle) || r.metadata?.name.toLowerCase().includes(needle),
  )
})

async function load() {
  loading.value = true
  updates.value = new Map()
  resources.value = (await tasks.guard(() => api.listLocalResources(props.instanceId, props.kind))) ?? []
  loading.value = false
}
onMounted(load)
watch(() => [props.instanceId, props.kind], load)

async function toggle(r: LocalResource, enabled: boolean) {
  const ok = await tasks.guard(async () => {
    await api.toggleResource(props.instanceId, props.kind, r.fileName, enabled)
    return true
  })
  if (ok) await load()
}

async function confirmDelete() {
  const r = toDelete.value
  toDelete.value = null
  if (!r) return
  const ok = await tasks.guard(async () => {
    await api.deleteResource(props.instanceId, props.kind, r.fileName)
    return true
  })
  if (ok) await load()
}

async function checkUpdates() {
  checking.value = true
  const found = await tasks.guard(() => api.checkUpdates(props.instanceId, props.kind))
  checking.value = false
  if (!found) return
  updates.value = new Map(found.map((u) => [u.fileName, u]))
  if (!found.length) tasks.toast(t.instance.noUpdates, 'ok')
}

async function applyUpdate(r: LocalResource) {
  const update = updates.value.get(r.fileName)
  if (!update) return
  const report = await tasks.guard(() => api.installVersion(props.instanceId, update.latest, props.kind))
  if (!report) return
  tasks.showBlocked(report.blocked)
  // core replaces the old file of an already installed project, so nothing is deleted here
  tasks.toast(t.instance.updated, 'ok')
  await load()
}

function browse() {
  void router.push({ name: 'browse', query: { instance: props.instanceId, kind: props.kind } })
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex items-center gap-2">
      <div class="relative max-w-xs flex-1">
        <Search :size="15" class="pointer-events-none absolute top-2.5 left-2.5 text-faint" />
        <input v-model="filter" class="input !pl-8" :placeholder="t.instance.filterPlaceholder" :aria-label="t.instance.filterPlaceholder" />
      </div>
      <span class="mr-auto text-faint">{{ resources.length }}</span>
      <button class="btn" :disabled="checking || !resources.length" @click="checkUpdates">
        <RefreshCw :size="15" :class="checking ? 'animate-spin' : ''" />{{ checking ? t.instance.checking : t.instance.checkUpdates }}
      </button>
      <button class="btn btn-primary" @click="browse"><Plus :size="16" />{{ t.instance.addContent }}</button>
    </div>

    <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
    <p v-else-if="!visible.length" class="py-10 text-center text-faint">{{ t.instance.noResources }}</p>
    <ul v-else class="flex flex-col gap-1.5">
      <li v-for="r in visible" :key="r.fileName" class="card flex items-center gap-3 px-3 py-2" :class="r.enabled ? '' : 'opacity-60'">
        <ToggleSwitch :model-value="r.enabled" :label="t.common.enabled" @update:model-value="toggle(r, $event)" />
        <img v-if="r.metadata?.icon" :src="r.metadata.icon" alt="" class="pixelated size-9 rounded-md" />
        <span v-else class="pix flex size-9 shrink-0 items-center justify-center bg-accent/15 font-bold text-accent">
          {{ (r.metadata?.name ?? r.fileName).charAt(0).toUpperCase() }}
        </span>
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <span class="truncate font-semibold">{{ r.metadata?.name ?? r.fileName }}</span>
            <span v-if="r.metadata?.version" class="badge">{{ r.metadata.version }}</span>
            <span v-if="!r.origin" class="badge" :title="t.instance.unknownOrigin">?</span>
            <span v-else class="badge badge-accent">{{ originPlatform(r.origin) === 'modrinth' ? 'Modrinth' : 'CurseForge' }}</span>
          </div>
          <div class="truncate text-[12px] text-faint">{{ r.fileName }} · {{ formatBytes(r.size) }}<template v-if="r.metadata?.authors"> · {{ r.metadata.authors.join(', ') }}</template></div>
        </div>
        <button v-if="updates.has(r.fileName)" class="btn btn-primary h-7" @click="applyUpdate(r)">
          <ArrowUpCircle :size="14" />{{ t.instance.updateTo }} {{ updates.get(r.fileName)!.latest.versionNumber }}
        </button>
        <button class="btn btn-ghost btn-icon text-muted hover:!text-danger" :aria-label="t.common.delete" @click="toDelete = r"><Trash2 :size="15" /></button>
      </li>
    </ul>

    <ConfirmDialog
      v-if="toDelete"
      :title="t.instance.deleteResource"
      :text="toDelete.fileName"
      @confirm="confirmDelete"
      @close="toDelete = null"
    />
  </div>
</template>
