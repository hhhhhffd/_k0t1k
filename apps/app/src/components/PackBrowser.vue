<script lang="ts">
import type { Instance } from '../api/types'

export type PackPlatform = 'modrinth' | 'curseForge' | 'ftb'

/** What the new-instance dialog installs when OK is pressed on a pack page. */
export interface PackChoice {
  title: string
  install: (name: string) => Promise<Instance | undefined>
}
</script>

<script setup lang="ts">
import { Download, Search } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { ftbPackVersions, ftbSearchPacks, getProjectVersions, searchContent } from '../api/commands'
import type { FtbPack, FtbVersion, ProjectVersion, SearchHit } from '../api/types'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { formatCount, LOADER_LABELS } from '../utils/format'
import ProjectIcon from './ProjectIcon.vue'
import SelectMenu from './SelectMenu.vue'

const props = defineProps<{ platform: PackPlatform }>()
const choice = defineModel<PackChoice | null>({ required: true })
const store = useInstancesStore()
const tasks = useTasksStore()

/** Modrinth/CurseForge hits and FTB packs in one shape. */
interface PackItem {
  id: string
  title: string
  author: string
  description: string
  iconUrl: string | null
  downloads: number
  hit?: SearchHit
}
interface PackVersion {
  id: string
  label: string
  hint: string
  install: (name: string) => Promise<Instance | undefined>
}

const PAGE = 30
const query = ref('')
const items = ref<PackItem[]>([])
const total = ref(0)
const loading = ref(false)
const selected = ref<PackItem | null>(null)
const versions = ref<PackVersion[]>([])
const versionId = ref('')
const loadingVersions = ref(false)

const fromHit = (hit: SearchHit): PackItem => ({
  id: hit.projectId, title: hit.title, author: hit.author, description: hit.description, iconUrl: hit.iconUrl, downloads: hit.downloads, hit,
})
const fromFtb = (p: FtbPack): PackItem => ({
  id: String(p.id), title: p.name, author: p.authors.join(', '), description: p.summary, iconUrl: p.iconUrl, downloads: p.installs,
})
const releaseLabel: Record<string, string> = { release: t.browse.release, beta: t.browse.beta, alpha: t.browse.alpha }

function versionOfProject(v: ProjectVersion): PackVersion {
  return {
    id: v.id,
    label: v.versionNumber || v.name,
    hint: [releaseLabel[v.releaseType], v.gameVersions[0], v.loaders.map((l) => LOADER_LABELS[l]).join('/')].filter(Boolean).join(' · '),
    install: (name) => tasks.guard(() => store.installModpack(v, name)),
  }
}
function versionOfFtb(packId: number, v: FtbVersion): PackVersion {
  return {
    id: String(v.id),
    label: v.name,
    hint: [releaseLabel[v.type] ?? v.type, v.minecraftVersion, v.loader ? LOADER_LABELS[v.loader.id] : ''].filter(Boolean).join(' · '),
    install: (name) => tasks.guard(() => store.installFtbPack(packId, v.id, name)),
  }
}

let searchSeq = 0
async function run(append = false) {
  const seq = ++searchSeq
  loading.value = true
  let next: PackItem[] | undefined
  if (props.platform === 'ftb') {
    next = (await tasks.guard(() => ftbSearchPacks(query.value.trim())))?.map(fromFtb)
    if (next) total.value = next.length
  } else {
    const platform = props.platform
    const page = await tasks.guard(() =>
      searchContent({ platform, kind: 'modpack', query: query.value, offset: append ? items.value.length : 0, limit: PAGE }),
    )
    next = page?.hits.map(fromHit)
    if (page) total.value = page.totalHits
  }
  if (seq !== searchSeq) return // a newer search superseded this one
  loading.value = false
  if (next) items.value = append ? [...items.value, ...next] : next
}
let debounce: ReturnType<typeof setTimeout> | undefined
watch(query, () => {
  clearTimeout(debounce)
  debounce = setTimeout(() => void run(), 300)
})
onMounted(() => void run())

let versionSeq = 0
async function select(item: PackItem) {
  const seq = ++versionSeq
  selected.value = item
  versions.value = []
  versionId.value = ''
  choice.value = null
  loadingVersions.value = true
  let list: PackVersion[] | undefined
  if (props.platform === 'ftb') {
    const packId = Number(item.id)
    list = (await tasks.guard(() => ftbPackVersions(packId)))?.map((v) => versionOfFtb(packId, v))
  } else {
    const hit = item.hit!
    list = (await tasks.guard(() => getProjectVersions(hit.platform, hit.projectId, {})))?.map(versionOfProject)
  }
  if (seq !== versionSeq) return
  loadingVersions.value = false
  versions.value = list ?? []
  versionId.value = versions.value[0]?.id ?? ''
}

const versionOptions = computed(() => versions.value.map((v) => ({ value: v.id, label: v.label, hint: v.hint })))
watch(versionId, (id) => {
  const version = versions.value.find((v) => v.id === id)
  choice.value = version && selected.value ? { title: selected.value.title, install: version.install } : null
})
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col gap-3">
    <div class="relative shrink-0">
      <Search :size="15" class="pointer-events-none absolute top-2.5 left-2.5 text-faint" />
      <input v-model="query" class="input !pl-8" :placeholder="t.create.packSearch" :aria-label="t.common.search" />
    </div>
    <div class="flex min-h-0 flex-1 gap-3">
      <ul class="min-h-0 min-w-0 flex-1 overflow-y-auto rounded-lg border border-line bg-base p-1">
        <li v-for="item in items" :key="item.id">
          <button
            class="flex w-full items-center gap-3 rounded-md p-2 text-left"
            :class="selected?.id === item.id ? 'bg-accent/20' : 'hover:bg-hover'"
            @click="select(item)"
          >
            <ProjectIcon :hit="{ iconUrl: item.iconUrl, title: item.title, slug: item.id }" :size="40" />
            <span class="min-w-0 flex-1">
              <span class="block truncate font-semibold" :class="selected?.id === item.id ? 'text-accent' : ''">{{ item.title }}</span>
              <span class="block truncate text-[12px] text-muted">{{ item.description }}</span>
            </span>
            <span class="flex shrink-0 items-center gap-1 text-[12px] text-faint"><Download :size="12" />{{ formatCount(item.downloads) }}</span>
          </button>
        </li>
        <li v-if="loading" class="p-4 text-center text-faint">{{ t.common.loading }}</li>
        <li v-else-if="!items.length" class="p-4 text-center text-faint">{{ t.common.nothingFound }}</li>
        <li v-else-if="items.length < total" class="p-2 text-center">
          <button class="btn" @click="run(true)">{{ t.browse.loadMore }}</button>
        </li>
      </ul>

      <aside class="flex w-[38%] min-w-64 shrink-0 flex-col gap-3 overflow-y-auto rounded-lg border border-line bg-surface p-4">
        <template v-if="selected">
          <div class="flex items-center gap-3">
            <ProjectIcon :hit="{ iconUrl: selected.iconUrl, title: selected.title, slug: selected.id }" :size="56" />
            <div class="min-w-0">
              <h3 class="text-[16px] font-bold">{{ selected.title }}</h3>
              <p v-if="selected.author" class="text-[12px] text-muted">{{ t.browse.by }} {{ selected.author }}</p>
            </div>
          </div>
          <p class="text-muted selectable">{{ selected.description }}</p>
          <div class="mt-auto flex flex-col gap-1">
            <span class="label">{{ t.create.packVersion }}</span>
            <p v-if="loadingVersions" class="text-faint">{{ t.common.loading }}</p>
            <p v-else-if="!versions.length" class="text-warn">{{ t.browse.noVersions }}</p>
            <SelectMenu v-else v-model="versionId" searchable :options="versionOptions" :aria-label="t.create.packVersion" />
          </div>
        </template>
        <p v-else class="m-auto text-center text-faint">{{ t.create.pickPack }}</p>
      </aside>
    </div>
  </div>
</template>
