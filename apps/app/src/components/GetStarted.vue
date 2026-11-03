<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { Download, FileArchive, FolderOpen, Loader, Plus, Search, TriangleAlert } from '@lucide/vue'
import { computed, onMounted, ref } from 'vue'
import { discoverLauncherInstances, findPrismInstances, getProjectVersions, isTauri, searchContent } from '../api/commands'
import type { LoaderId, PrismInstanceInfo, SearchHit } from '../api/types'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { formatCount, LOADER_LABELS } from '../utils/format'
import CreateInstanceDialog from './CreateInstanceDialog.vue'
import ModalDialog from './ModalDialog.vue'
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

const searching = ref(false)
// More than one install: each row shows which one it came from.
const showLauncherDir = computed(() => new Set(prism.value.map((i) => i.launcherDir)).size > 1)

async function discover() {
  searching.value = true
  const found = await tasks.guard(() => discoverLauncherInstances())
  searching.value = false
  if (!found) return
  if (!found.length) tasks.toast(t.start.discoverNone, 'error')
  const known = new Set(prism.value.map((i) => i.path))
  prism.value = [...prism.value, ...found.filter((i) => !known.has(i.path))]
}

/** The mismatch question being asked; resolves to the chosen loader, `null` for "as declared", or `undefined` when cancelled. */
const mismatch = ref<{ info: PrismInstanceInfo; answer: (loader: LoaderId | null | undefined) => void } | null>(null)

/** Resolves to `{ loader }` (undefined = as declared in the launcher) or `null` when the user cancels. */
async function chooseLoader(info: PrismInstanceInfo): Promise<{ loader?: LoaderId } | null> {
  if (!info.modLoader) return {}
  const answer = await new Promise<LoaderId | null | undefined>((answer) => (mismatch.value = { info, answer }))
  mismatch.value = null
  return answer === undefined ? null : { loader: answer ?? undefined }
}

async function runImport(info: PrismInstanceInfo, loader?: LoaderId) {
  imported.value = new Set(imported.value).add(info.path)
  emit('chosen')
  const done = await tasks.guard(() => store.importPrism(info.path, info.name, loader).then(() => true))
  if (!done) {
    const next = new Set(imported.value)
    next.delete(info.path)
    imported.value = next
  }
}
async function importOne(info: PrismInstanceInfo) {
  const choice = await chooseLoader(info)
  if (choice) await runImport(info, choice.loader)
}
async function importAll() {
  // Ask everything first: the first started import makes the parent leave this view.
  const chosen: { info: PrismInstanceInfo; loader?: LoaderId }[] = []
  for (const info of prism.value) {
    if (imported.value.has(info.path)) continue
    const choice = await chooseLoader(info)
    if (choice) chosen.push({ info, loader: choice.loader })
  }
  for (const { info, loader } of chosen) void runImport(info, loader)
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
        <div class="min-w-0">
          <h3 class="font-mono text-[13px] font-bold">{{ t.start.prism }}</h3>
          <p class="truncate text-[12px] text-faint">{{ t.start.prismHint }}</p>
        </div>
        <button v-if="prism.length > 1" class="btn shrink-0" @click="importAll">{{ t.start.importAll }}</button>
      </div>
      <p v-if="prismLoading" class="py-1 text-faint">{{ t.common.loading }}</p>
      <ul v-else-if="prism.length" class="flex max-h-56 flex-col overflow-y-auto">
        <li v-for="info in prism" :key="info.path" class="flex items-center gap-3 rounded-md p-1.5">
          <span class="min-w-0 flex-1">
            <span class="block truncate font-semibold">{{ info.name }}</span>
            <span class="block truncate text-[12px] text-faint">Minecraft {{ info.minecraftVersion }}<template v-if="info.loader"> · {{ info.loader }}</template> · {{ info.launcher }}</span>
            <span v-if="showLauncherDir && info.launcherDir" class="block truncate text-[12px] text-faint" :title="info.path">{{ info.launcherDir }}</span>
            <span v-if="info.modLoader && info.declaredLoader" class="mt-0.5 flex items-start gap-1 text-[12px] text-warn">
              <TriangleAlert :size="12" class="mt-0.5 shrink-0" />{{ t.start.mismatchBadge(LOADER_LABELS[info.modLoader], LOADER_LABELS[info.declaredLoader], info.launcher) }}
            </span>
          </span>
          <button class="btn" :disabled="imported.has(info.path)" @click="importOne(info)">{{ imported.has(info.path) ? t.start.importing : t.start.import }}</button>
        </li>
      </ul>
      <p v-else class="text-faint">{{ t.start.prismEmpty }}</p>
      <div class="flex flex-wrap items-center gap-2">
        <button class="btn" :disabled="searching" @click="discover">
          <Loader v-if="searching" :size="15" class="animate-spin" /><Search v-else :size="15" />{{ searching ? t.start.discovering : t.start.discover }}
        </button>
        <button class="btn" :disabled="searching" @click="pickPrismFolder"><FolderOpen :size="15" />{{ t.start.chooseFolder }}</button>
      </div>
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

    <ModalDialog v-if="mismatch" :title="t.start.mismatchTitle" size="sm" @close="mismatch.answer(undefined)">
      <p class="text-muted">{{ mismatch.info.name }}: {{ mismatch.info.modLoader && mismatch.info.declaredLoader ? t.start.mismatchBadge(LOADER_LABELS[mismatch.info.modLoader], LOADER_LABELS[mismatch.info.declaredLoader], mismatch.info.launcher) : '' }}</p>
      <template #footer>
        <button class="btn" @click="mismatch.answer(undefined)">{{ t.common.cancel }}</button>
        <button v-if="mismatch.info.declaredLoader" class="btn" @click="mismatch.answer(null)">{{ t.start.mismatchKeep(LOADER_LABELS[mismatch.info.declaredLoader], mismatch.info.launcher) }}</button>
        <button v-if="mismatch.info.modLoader" class="btn btn-primary" @click="mismatch.answer(mismatch.info.modLoader)">{{ t.start.mismatchUseMods(LOADER_LABELS[mismatch.info.modLoader]) }}</button>
      </template>
    </ModalDialog>

    <CreateInstanceDialog v-if="createPage" :initial-page="createPage" @close="createClosed" />
  </div>
</template>
