<script setup lang="ts">
import { Search, X } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { listMinecraftVersions, searchContent } from '../api/commands'
import type { LoaderId, MinecraftVersion, Platform, ResourceKind, SearchHit } from '../api/types'
import ProjectDrawer from '../components/ProjectDrawer.vue'
import ProjectIcon from '../components/ProjectIcon.vue'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { formatCount, formatRelative, LOADER_LABELS, loaderOf, minecraftVersionOf } from '../utils/format'

const PAGE = 20
const kinds: ResourceKind[] = ['mod', 'modpack', 'resourcepack', 'shader']
const route = useRoute()
const store = useInstancesStore()
const tasks = useTasksStore()

const queryKind = route.query.kind
const platform = ref<Platform>('modrinth')
const kind = ref<ResourceKind>(kinds.includes(queryKind as ResourceKind) ? (queryKind as ResourceKind) : 'mod')
const query = ref('')
const gameVersion = ref('')
const loader = ref<LoaderId | ''>('')
const instanceId = ref(typeof route.query.instance === 'string' ? route.query.instance : '')
const mcVersions = ref<MinecraftVersion[]>([])

const hits = ref<SearchHit[]>([])
const total = ref(0)
const loading = ref(false)
const selected = ref<SearchHit | null>(null)

const instance = computed(() => store.byId(instanceId.value))
const showLoader = computed(() => kind.value === 'mod' || kind.value === 'modpack')

function applyInstanceFilters() {
  const inst = instance.value
  if (!inst) return
  gameVersion.value = minecraftVersionOf(inst)
  loader.value = kind.value === 'mod' ? (loaderOf(inst)?.id ?? '') : ''
}

let requestSeq = 0
async function run(append = false) {
  const seq = ++requestSeq
  loading.value = true
  const result = await tasks.guard(() =>
    searchContent({
      platform: platform.value,
      kind: kind.value,
      query: query.value,
      gameVersion: gameVersion.value || undefined,
      loader: showLoader.value && loader.value ? loader.value : undefined,
      offset: append ? hits.value.length : 0,
      limit: PAGE,
    }),
  )
  if (seq !== requestSeq) return // a newer search superseded this one
  loading.value = false
  if (!result) return
  hits.value = append ? [...hits.value, ...result.hits] : result.hits
  total.value = result.total
}

let debounce: ReturnType<typeof setTimeout> | undefined
watch(query, () => {
  clearTimeout(debounce)
  debounce = setTimeout(() => void run(), 250)
})
watch([platform, kind, gameVersion, loader], () => void run())
watch(kind, applyInstanceFilters)

function clearInstance() {
  instanceId.value = ''
  gameVersion.value = ''
  loader.value = ''
}

onMounted(async () => {
  const list = await tasks.guard(listMinecraftVersions)
  mcVersions.value = (list ?? []).filter((v) => v.type === 'release')
  if (!store.loaded) await store.refresh()
  applyInstanceFilters()
  await run()
})
</script>

<template>
  <div class="mx-auto flex max-w-5xl flex-col gap-4 p-6">
    <header class="flex items-center gap-3">
      <h1 class="mr-auto text-xl font-bold">{{ t.browse.title }}</h1>
      <div class="flex gap-0.5 rounded-lg border border-line p-0.5" role="group">
        <button class="btn btn-ghost h-7" :class="platform === 'modrinth' ? '!bg-accent !text-accent-fg' : ''" @click="platform = 'modrinth'">Modrinth</button>
        <button class="btn btn-ghost h-7" :class="platform === 'curseforge' ? '!bg-accent !text-accent-fg' : ''" @click="platform = 'curseforge'">CurseForge</button>
      </div>
    </header>

    <div class="flex gap-1 border-b border-line" role="tablist">
      <button
        v-for="k in kinds"
        :key="k"
        role="tab"
        :aria-selected="kind === k"
        class="-mb-px h-9 border-b-2 px-3.5 font-medium"
        :class="kind === k ? 'border-accent text-accent' : 'border-transparent text-muted hover:text-fg'"
        @click="kind = k"
      >
        {{ t.browse.kinds[k] }}
      </button>
    </div>

    <div class="flex flex-wrap items-center gap-2">
      <div class="relative min-w-60 flex-1">
        <Search :size="15" class="pointer-events-none absolute top-2.5 left-2.5 text-faint" />
        <input v-model="query" class="input pl-8" :placeholder="t.browse.searchPlaceholder" :aria-label="t.common.search" />
      </div>
      <select v-model="gameVersion" class="input w-40" :aria-label="t.browse.gameVersion">
        <option value="">{{ t.browse.anyVersion }}</option>
        <option v-for="v in mcVersions" :key="v.version" :value="v.version">{{ v.version }}</option>
      </select>
      <select v-if="showLoader" v-model="loader" class="input w-36" :aria-label="t.browse.loader">
        <option value="">{{ t.browse.anyLoader }}</option>
        <option v-for="(label, id) in LOADER_LABELS" :key="id" :value="id">{{ label }}</option>
      </select>
    </div>

    <div v-if="instance" class="flex items-center gap-2">
      <span class="badge badge-accent h-6 gap-2 pr-1">
        {{ t.browse.forInstance }}: {{ instance.name }}
        <button class="rounded p-0.5 hover:bg-accent/20" :aria-label="t.browse.resetFilter" :title="t.browse.resetFilter" @click="clearInstance"><X :size="12" /></button>
      </span>
    </div>

    <p class="text-[12px] text-faint">{{ t.browse.results }}: {{ total }}</p>
    <ul class="flex flex-col gap-2">
      <li v-for="hit in hits" :key="hit.projectId">
        <button class="card flex w-full items-start gap-3 p-3 text-left transition-colors hover:border-accent/60" @click="selected = hit">
          <ProjectIcon :hit="hit" :size="56" />
          <div class="min-w-0 flex-1">
            <div class="flex items-baseline gap-2">
              <span class="truncate text-[15px] font-semibold">{{ hit.title }}</span>
              <span class="shrink-0 text-muted">{{ t.browse.by }} {{ hit.author }}</span>
            </div>
            <p class="mt-0.5 line-clamp-2 text-muted">{{ hit.description }}</p>
            <div class="mt-1.5 flex flex-wrap gap-1">
              <span v-for="c in hit.categories" :key="c" class="badge">{{ c }}</span>
            </div>
          </div>
          <div class="shrink-0 text-right text-[12px] text-muted">
            <div class="font-semibold text-fg">{{ formatCount(hit.downloads) }}</div>
            <div>{{ t.browse.downloads }}</div>
            <div class="mt-1 text-faint">{{ formatRelative(hit.updatedAt) }}</div>
          </div>
        </button>
      </li>
    </ul>
    <p v-if="loading" class="py-6 text-center text-faint">{{ t.common.loading }}</p>
    <p v-else-if="!hits.length" class="py-10 text-center text-faint">{{ t.common.nothingFound }}</p>
    <button v-if="!loading && hits.length < total" class="btn mx-auto" @click="run(true)">{{ t.browse.loadMore }}</button>

    <ProjectDrawer
      v-if="selected"
      :hit="selected"
      :instance-id="instanceId"
      :game-version="gameVersion || undefined"
      :loader="showLoader && loader ? loader : undefined"
      @close="selected = null"
    />
  </div>
</template>
