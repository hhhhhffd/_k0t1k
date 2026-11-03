<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { Download, FileArchive, FolderOpen, Plus } from '@lucide/vue'
import { onMounted, ref } from 'vue'
import { findPrismInstances, getProjectVersions, isTauri, searchContent } from '../api/commands'
import type { PrismInstanceInfo, SearchHit } from '../api/types'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { formatCount } from '../utils/format'
import CreateInstanceDialog from './CreateInstanceDialog.vue'
import ProjectIcon from './ProjectIcon.vue'

/** Everything a newcomer can do to get a first instance. `chosen` fires once one of the ways was taken. */
const emit = defineEmits<{ chosen: [] }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const createPage = ref<'custom' | 'import' | null>(null)

function createClosed() {
  createPage.value = null
  // a cancelled dialog is not a choice
  if (store.instances.length || store.pending.length) emit('chosen')
}

const POPULAR = 6
const popular = ref<SearchHit[]>([])
const popularState = ref<'loading' | 'ready' | 'failed'>('loading')
const installing = ref(new Set<string>())

async function loadPopular() {
  popularState.value = 'loading'
  try {
    popular.value = (await searchContent({ platform: 'modrinth', kind: 'modpack', query: '', offset: 0, limit: POPULAR })).hits
    popularState.value = 'ready'
  } catch (e) {
    console.warn('cannot load popular modpacks', e)
    popularState.value = 'failed'
  }
}

async function install(hit: SearchHit) {
  installing.value = new Set(installing.value).add(hit.projectId)
  const versions = await tasks.guard(() => getProjectVersions(hit.platform, hit.projectId, {}))
  if (versions?.[0]) {
    emit('chosen')
    await tasks.guard(() => store.installModpack(versions[0], hit.title))
  } else if (versions) {
    tasks.toast(t.browse.noVersions, 'error')
  }
  const next = new Set(installing.value)
  next.delete(hit.projectId)
  installing.value = next
}

const prism = ref<PrismInstanceInfo[]>([])
const prismLoading = ref(true)
const imported = ref(new Set<string>())

async function loadPrism(folder?: string) {
  prismLoading.value = true
  const found = await tasks.guard(() => findPrismInstances(folder))
  prismLoading.value = false
  if (!found) return
  if (folder && !found.length) tasks.toast(t.start.prismNone, 'error')
  prism.value = found
}

async function pickPrismFolder() {
  // Outside Tauri there is no native dialog; the mock accepts a fake path.
  const picked = isTauri ? await open({ directory: true, multiple: false, title: t.start.prismPickTitle }) : 'C:\\Prism\\instances'
  if (typeof picked === 'string') await loadPrism(picked)
}

async function importOne(info: PrismInstanceInfo) {
  imported.value = new Set(imported.value).add(info.path)
  emit('chosen')
  const done = await tasks.guard(() => store.importPrism(info.path, info.name).then(() => true))
  if (!done) {
    const next = new Set(imported.value)
    next.delete(info.path)
    imported.value = next
  }
}
function importAll() {
  for (const info of prism.value) if (!imported.value.has(info.path)) void importOne(info)
}

onMounted(() => {
  void loadPopular()
  void loadPrism()
})
</script>

<template>
  <div class="@container w-full max-w-4xl text-left">
    <div class="grid grid-cols-1 items-start gap-4 @min-[38rem]:grid-cols-2">
    <div class="flex min-w-0 flex-col gap-4">
    <button class="card flex items-center gap-4 border-accent/60 p-4 text-left transition-colors hover:bg-hover" @click="createPage = 'custom'">
      <span class="pix flex size-12 shrink-0 items-center justify-center bg-accent text-white"><Plus :size="26" /></span>
      <span class="min-w-0">
        <span class="block text-[16px] font-bold">{{ t.start.create }}</span>
        <span class="block text-muted">{{ t.start.createHint }}</span>
      </span>
    </button>

    <section class="card flex flex-col gap-2 p-3">
      <div class="flex items-center justify-between gap-2">
        <h3 class="font-mono text-[13px] font-bold">{{ t.start.prism }}</h3>
        <button v-if="prism.length > 1" class="btn" @click="importAll">{{ t.start.importAll }}</button>
      </div>
      <p v-if="prismLoading" class="py-1 text-faint">{{ t.common.loading }}</p>
      <ul v-else-if="prism.length" class="flex max-h-44 flex-col overflow-y-auto">
        <li v-for="info in prism" :key="info.path" class="flex items-center gap-3 rounded-md p-1.5">
          <span class="min-w-0 flex-1">
            <span class="block truncate font-semibold">{{ info.name }}</span>
            <span class="block truncate text-[12px] text-faint">Minecraft {{ info.minecraftVersion }}<template v-if="info.loader"> · {{ info.loader }}</template> · {{ info.launcher }}</span>
          </span>
          <button class="btn" :disabled="imported.has(info.path)" @click="importOne(info)">{{ imported.has(info.path) ? t.start.importing : t.start.import }}</button>
        </li>
      </ul>
      <p v-else class="text-faint">{{ t.start.prismEmpty }}</p>
      <div><button class="btn" @click="pickPrismFolder"><FolderOpen :size="15" />{{ t.start.chooseFolder }}</button></div>
    </section>

    <section class="card flex flex-wrap items-center gap-3 p-3">
      <FileArchive :size="22" class="shrink-0 text-icon" />
      <p class="min-w-[12rem] flex-1 text-muted">{{ t.start.files }}</p>
      <button class="btn shrink-0" @click="createPage = 'import'">{{ t.start.importFile }}</button>
    </section>

    </div>
    <section class="card flex flex-col gap-2 p-3">
      <h3 class="font-mono text-[13px] font-bold">{{ t.start.popular }}</h3>
      <p v-if="popularState === 'loading'" class="py-3 text-center text-faint">{{ t.common.loading }}</p>
      <div v-else-if="popularState === 'failed'" class="flex items-center justify-between gap-3 py-1">
        <span class="text-faint">{{ t.start.popularFailed }}</span>
        <button class="btn" @click="loadPopular">{{ t.start.retry }}</button>
      </div>
      <ul v-else class="flex flex-col">
        <li v-for="hit in popular" :key="hit.projectId" class="flex items-center gap-3 rounded-md p-1.5">
          <ProjectIcon :hit="hit" :size="36" />
          <span class="min-w-0 flex-1">
            <span class="block truncate font-semibold">{{ hit.title }}</span>
            <span class="flex items-center gap-1 text-[12px] text-faint"><Download :size="11" />{{ formatCount(hit.downloads) }}</span>
          </span>
          <button class="btn" :disabled="installing.has(hit.projectId)" @click="install(hit)">{{ installing.has(hit.projectId) ? t.common.loading : t.start.install }}</button>
        </li>
      </ul>
    </section>

    </div>

    <CreateInstanceDialog v-if="createPage" :initial-page="createPage" @close="createClosed" />
  </div>
</template>
