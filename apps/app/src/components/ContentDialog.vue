<script setup lang="ts">
import { Download, Search } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { listMinecraftVersions, searchContent } from '../api/commands'
import type { LoaderId, MinecraftVersion, Platform, SearchHit } from '../api/types'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { formatCount, LOADER_LABELS, loaderOf, minecraftVersionOf } from '../utils/format'
import curseforgeLogo from '../assets/brands/curseforge.svg'
import modrinthLogo from '../assets/brands/modrinth.svg'
import ModalDialog from './ModalDialog.vue'
import ProjectDrawer from './ProjectDrawer.vue'
import ProjectIcon from './ProjectIcon.vue'
import SelectMenu from './SelectMenu.vue'

/** Searches Modrinth and CurseForge for content of one kind and installs it into the instance (or one of its worlds). */
const props = defineProps<{ instanceId: string; kind: 'mod' | 'resourcePack' | 'shaderPack' | 'dataPack'; world?: string }>()
const emit = defineEmits<{ close: [] }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const PAGE = 20
const instance = computed(() => store.byId(props.instanceId))
const platform = ref<Platform>('modrinth')
const query = ref('')
// filtered for the instance by default; both filters can be widened
const gameVersion = ref(instance.value ? minecraftVersionOf(instance.value) : '')
const loader = ref<LoaderId | ''>(props.kind === 'mod' && instance.value ? (loaderOf(instance.value)?.id ?? '') : '')
const mcVersions = ref<MinecraftVersion[]>([])

const hits = ref<SearchHit[]>([])
const total = ref(0)
const loading = ref(false)
const selected = ref<SearchHit | null>(null)

const versionOptions = computed(() => [
  { value: '', label: t.browse.anyVersion },
  ...mcVersions.value.map((v) => ({ value: v.version, label: v.version })),
])
const loaderOptions = [
  { value: '', label: t.browse.anyLoader },
  ...(Object.entries(LOADER_LABELS) as [LoaderId, string][]).map(([value, label]) => ({ value, label })),
]

let requestSeq = 0
async function run(append = false) {
  const seq = ++requestSeq
  loading.value = true
  const result = await tasks.guard(() =>
    searchContent({
      platform: platform.value,
      kind: props.kind,
      query: query.value,
      gameVersion: gameVersion.value || undefined,
      loader: props.kind === 'mod' && loader.value ? loader.value : undefined,
      offset: append ? hits.value.length : 0,
      limit: PAGE,
    }),
  )
  if (seq !== requestSeq) return // a newer search superseded this one
  loading.value = false
  if (!result) return
  hits.value = append ? [...hits.value, ...result.hits] : result.hits
  total.value = result.totalHits
}

let debounce: ReturnType<typeof setTimeout> | undefined
watch(query, () => {
  clearTimeout(debounce)
  debounce = setTimeout(() => void run(), 250)
})
watch([platform, gameVersion, loader], () => void run())

onMounted(async () => {
  void run()
  const list = await tasks.guard(listMinecraftVersions)
  mcVersions.value = (list ?? []).filter((v) => v.type === 'release')
})
</script>

<template>
  <ModalDialog :title="`${t.content.titles[kind]}${instance ? ` — ${instance.name}` : ''}`" large @close="emit('close')">
    <div class="flex shrink-0 flex-wrap items-center gap-2 border-b border-line p-4">
      <div class="flex gap-0.5 rounded-lg border border-line p-0.5" role="group">
        <button class="btn btn-ghost h-7" :class="platform === 'modrinth' ? '!bg-accent !text-accent-fg' : ''" @click="platform = 'modrinth'"><img :src="modrinthLogo" alt="" class="size-4" />Modrinth</button>
        <button class="btn btn-ghost h-7" :class="platform === 'curseForge' ? '!bg-accent !text-accent-fg' : ''" @click="platform = 'curseForge'"><img :src="curseforgeLogo" alt="" class="size-4" />CurseForge</button>
      </div>
      <div class="relative min-w-60 flex-1">
        <Search :size="15" class="pointer-events-none absolute top-2.5 left-2.5 text-faint" />
        <input v-model="query" class="input !pl-8" :placeholder="t.browse.searchPlaceholder" :aria-label="t.common.search" autofocus />
      </div>
      <SelectMenu v-model="gameVersion" class="w-44" searchable :options="versionOptions" :aria-label="t.browse.gameVersion" />
      <SelectMenu v-if="kind === 'mod'" v-model="loader" class="w-36" :options="loaderOptions" :aria-label="t.browse.loader" />
    </div>

    <div class="min-h-0 flex-1 overflow-y-auto p-4">
      <p class="mb-2 text-[12px] text-faint">{{ t.browse.results }}: {{ total }}</p>
      <ul class="grid grid-cols-[repeat(auto-fill,minmax(420px,1fr))] gap-2">
        <li v-for="hit in hits" :key="hit.projectId">
          <button class="card flex h-full w-full items-start gap-3 p-3 text-left transition-colors hover:border-accent/60" @click="selected = hit">
            <ProjectIcon :hit="hit" :size="52" />
            <div class="min-w-0 flex-1">
              <div class="flex items-baseline gap-2">
                <span class="truncate text-[14.5px] font-semibold">{{ hit.title }}</span>
                <span class="shrink-0 truncate text-[12px] text-muted">{{ t.browse.by }} {{ hit.author }}</span>
              </div>
              <p class="mt-0.5 line-clamp-2 text-muted">{{ hit.description }}</p>
            </div>
            <span class="flex shrink-0 items-center gap-1 text-[12px] text-faint"><Download :size="12" />{{ formatCount(hit.downloads) }}</span>
          </button>
        </li>
      </ul>
      <p v-if="loading" class="py-6 text-center text-faint">{{ t.common.loading }}</p>
      <p v-else-if="!hits.length" class="py-10 text-center text-faint">{{ t.common.nothingFound }}</p>
      <div v-if="!loading && hits.length < total" class="mt-3 flex justify-center">
        <button class="btn" @click="run(true)">{{ t.browse.loadMore }}</button>
      </div>
    </div>

    <template #footer>
      <button class="btn" @click="emit('close')">{{ t.common.close }}</button>
    </template>

    <ProjectDrawer
      v-if="selected"
      :hit="selected"
      :kind="kind"
      :instance-id="instanceId"
      :world="world"
      @close="selected = null"
    />
  </ModalDialog>
</template>
