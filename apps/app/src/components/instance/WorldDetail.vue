<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { ArrowLeft, Eye, EyeOff, Globe, Plus, Search, Trash2 } from '@lucide/vue'
import { onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import * as api from '../../api/commands'
import { isTauri } from '../../api/commands'
import type { LocalResource, World } from '../../api/types'
import { t } from '../../i18n/ru'
import { useTasksStore } from '../../stores/tasks'
import { formatBytes, formatRelative, originPlatform } from '../../utils/format'
import ConfirmDialog from '../ConfirmDialog.vue'
import ToggleSwitch from '../ToggleSwitch.vue'

const props = defineProps<{ instanceId: string; world: World }>()
const emit = defineEmits<{ back: [] }>()
const router = useRouter()
const tasks = useTasksStore()

const packs = ref<LocalResource[]>([])
const loading = ref(true)
const seedShown = ref(false)
const toDelete = ref<LocalResource | null>(null)

async function load() {
  loading.value = true
  packs.value = (await tasks.guard(() => api.listDatapacks(props.instanceId, props.world.dirName))) ?? []
  loading.value = false
}
onMounted(load)
watch(() => props.world.dirName, load)

/** Runs a modifying call and reloads the list when it succeeded. */
async function change<T>(action: () => Promise<T>): Promise<T | undefined> {
  const result = await tasks.guard(action)
  if (result !== undefined) await load()
  return result
}

const toggle = (pack: LocalResource, enabled: boolean) =>
  change(async () => {
    await api.toggleDatapack(props.instanceId, props.world.dirName, pack.fileName, enabled)
    return true
  })

async function confirmDelete() {
  const pack = toDelete.value
  toDelete.value = null
  if (!pack) return
  await change(async () => {
    await api.deleteDatapack(props.instanceId, props.world.dirName, pack.fileName)
    return true
  })
}

async function addFile() {
  const path = isTauri
    ? await open({ multiple: false, filters: [{ name: t.worlds.datapackFilter, extensions: ['zip'] }] })
    : 'C:\\Downloads\\my-datapack.zip'
  if (typeof path !== 'string') return
  const fileName = await change(() => api.addDatapack(props.instanceId, props.world.dirName, path))
  if (fileName) tasks.toast(t.worlds.datapackAdded(fileName), 'ok')
}

function browse() {
  void router.push({ name: 'browse', query: { instance: props.instanceId, kind: 'dataPack', world: props.world.dirName } })
}

async function copySeed() {
  if (!props.world.seed) return
  try {
    await navigator.clipboard.writeText(props.world.seed)
    tasks.toast(t.common.copy, 'ok')
  } catch {
    tasks.toast(props.world.seed)
  }
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <button class="btn btn-ghost -ml-2 w-fit text-muted" @click="emit('back')"><ArrowLeft :size="15" />{{ t.worlds.back }}</button>

    <section class="card flex items-center gap-4 p-4">
      <img v-if="world.icon" :src="world.icon" alt="" class="pixelated size-16 shrink-0 rounded-md" />
      <span v-else class="pix flex size-16 shrink-0 items-center justify-center bg-ok/15 text-ok"><Globe :size="28" /></span>
      <div class="min-w-0 flex-1">
        <h2 class="truncate text-[17px] font-bold">{{ world.name }}</h2>
        <div class="mt-1.5 flex flex-wrap items-center gap-1.5">
          <span v-if="world.gameMode" class="badge">{{ t.worlds.gameModes[world.gameMode] }}</span>
          <span v-if="world.hardcore" class="badge border-danger/50 bg-danger/15 text-danger">{{ t.worlds.hardcore }}</span>
          <span v-if="world.versionName" class="badge">{{ t.worlds.version }} {{ world.versionName }}</span>
          <span class="text-muted">{{ t.worlds.folder }}: {{ world.dirName }} · {{ formatBytes(world.size) }} · {{ t.worlds.lastPlayed }} {{ formatRelative(world.lastPlayed) }}</span>
        </div>
        <div v-if="world.seed" class="mt-1.5 flex items-center gap-2 text-muted">
          <span>{{ t.worlds.seed }}:</span>
          <button v-if="seedShown" class="font-mono text-fg hover:text-accent" :title="t.worlds.copySeed" @click="copySeed">{{ world.seed }}</button>
          <span v-else class="font-mono">••••••••••</span>
          <button class="btn btn-ghost btn-icon" :title="seedShown ? t.worlds.hideSeed : t.worlds.showSeed" :aria-label="seedShown ? t.worlds.hideSeed : t.worlds.showSeed" @click="seedShown = !seedShown">
            <EyeOff v-if="seedShown" :size="15" /><Eye v-else :size="15" />
          </button>
        </div>
      </div>
    </section>

    <div class="flex items-center gap-2">
      <h3 class="section-title mr-auto">{{ t.worlds.datapacks }}</h3>
      <span class="text-faint">{{ packs.length }}</span>
      <button class="btn" @click="addFile"><Plus :size="15" />{{ t.worlds.addDatapack }}</button>
      <button class="btn btn-primary" @click="browse"><Search :size="15" />{{ t.worlds.findDatapacks }}</button>
    </div>
    <p class="text-[12px] text-faint">{{ t.worlds.datapacksHint }}</p>

    <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
    <p v-else-if="!packs.length" class="py-10 text-center text-faint">{{ t.worlds.datapacksEmpty }}</p>
    <ul v-else class="flex flex-col gap-1.5">
      <li v-for="p in packs" :key="p.fileName" class="card flex items-center gap-3 px-3 py-2" :class="p.enabled ? '' : 'opacity-60'">
        <ToggleSwitch :model-value="p.enabled" :label="t.common.enabled" @update:model-value="toggle(p, $event)" />
        <img v-if="p.metadata?.icon" :src="p.metadata.icon" alt="" class="pixelated size-9 rounded-md" />
        <span v-else class="pix flex size-9 shrink-0 items-center justify-center bg-accent/15 font-bold text-accent">
          {{ (p.metadata?.name ?? p.fileName).charAt(0).toUpperCase() }}
        </span>
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <span class="truncate font-semibold">{{ p.metadata?.name ?? p.fileName }}</span>
            <span v-if="p.metadata?.version" class="badge">{{ p.metadata.version }}</span>
            <span v-if="p.origin" class="badge badge-accent">{{ originPlatform(p.origin) === 'modrinth' ? 'Modrinth' : 'CurseForge' }}</span>
          </div>
          <div class="truncate text-[12px] text-faint">{{ p.metadata?.description ?? p.fileName }} · {{ formatBytes(p.size) }}</div>
        </div>
        <button class="btn btn-ghost btn-icon text-muted hover:!text-danger" :aria-label="t.common.delete" @click="toDelete = p"><Trash2 :size="15" /></button>
      </li>
    </ul>

    <ConfirmDialog v-if="toDelete" :title="t.worlds.deleteDatapack" :text="toDelete.fileName" @confirm="confirmDelete" @close="toDelete = null" />
  </div>
</template>
