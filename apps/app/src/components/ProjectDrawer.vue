<script setup lang="ts">
import { ArrowUpRight, Download, ExternalLink, ShieldAlert, TriangleAlert, X } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { getProjectVersions, installVersion } from '../api/commands'
import type { LoaderId, ProjectVersion, ResourceKind, SearchHit, VersionFilter } from '../api/types'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { openExternal } from '../utils/external'
import { formatBytes, formatCount, formatDate, LOADER_LABELS, loaderOf, minecraftVersionOf } from '../utils/format'
import ProjectIcon from './ProjectIcon.vue'
import SelectMenu from './SelectMenu.vue'
import ToggleSwitch from './ToggleSwitch.vue'

const props = defineProps<{ hit: SearchHit; kind: ResourceKind; instanceId: string; gameVersion?: string; loader?: LoaderId }>()
const emit = defineEmits<{ close: [] }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const versions = ref<ProjectVersion[]>([])
const loading = ref(true)
const target = ref(props.instanceId || store.instances[0]?.id || '')
const packName = ref(props.hit.title)
const busy = ref<string | null>(null)
const installedIds = ref(new Set<string>())

const isPack = computed(() => props.kind === 'modpack')
const showAll = ref(false)
const targetOptions = computed(() => store.instances.map((i) => ({ value: i.id, label: i.name })))

/** What a version must match to be installable into the target instance (a modpack creates its own instance). */
const requirement = computed<VersionFilter>(() => {
  if (isPack.value) return { gameVersion: props.gameVersion, loader: props.loader }
  const inst = store.byId(target.value)
  if (!inst) return {}
  // resource packs and shaders do not depend on a loader
  return { gameVersion: minecraftVersionOf(inst), loader: props.kind === 'mod' ? loaderOf(inst)?.id : undefined }
})

function mismatch(v: ProjectVersion): string | null {
  const { gameVersion, loader } = requirement.value
  const reasons: string[] = []
  if (gameVersion && !v.gameVersions.includes(gameVersion)) reasons.push(gameVersion)
  if (loader && !v.loaders.includes(loader)) reasons.push(LOADER_LABELS[loader])
  return reasons.length ? reasons.join(', ') : null
}

let loadSeq = 0
async function loadVersions() {
  const seq = ++loadSeq
  loading.value = true
  const filter = showAll.value ? {} : requirement.value
  const result = (await tasks.guard(() => getProjectVersions(props.hit.platform, props.hit.projectId, filter))) ?? []
  if (seq !== loadSeq) return // the target or the toggle changed while loading
  versions.value = result
  loading.value = false
}

onMounted(() => {
  window.addEventListener('keydown', onKey)
  void loadVersions()
})
// the modpack filter is fixed for the drawer's lifetime, so only the target and the toggle matter
watch([target, showAll], () => void loadVersions())
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}

async function install(v: ProjectVersion) {
  busy.value = v.id
  if (isPack.value) {
    const inst = await tasks.guard(() => store.installModpack(v, packName.value.trim() || undefined))
    if (inst) tasks.toast(t.browse.installedOk(inst.name), 'ok')
  } else if (props.kind !== 'modpack' && props.kind !== 'world') {
    const kind = props.kind
    const report = await tasks.guard(() => installVersion(target.value, v, kind))
    if (report) {
      tasks.showBlocked(report.blocked, target.value)
      const installed = report.installed.find((i) => !i.isDependency)
      if (installed) {
        installedIds.value = new Set(installedIds.value).add(v.id)
        tasks.toast(t.browse.installedOk(installed.fileName), 'ok')
      }
    }
  }
  busy.value = null
}

const releaseLabel = { release: t.browse.release, beta: t.browse.beta, alpha: t.browse.alpha }
const releaseClass = { release: 'badge-accent', beta: '', alpha: '' }
</script>

<template>
  <Teleport to="body">
    <div class="anim-fade fixed inset-0 z-40 bg-black/50" @mousedown="emit('close')" />
    <aside class="anim-slide fixed top-0 right-0 bottom-0 z-40 flex w-[480px] max-w-full flex-col border-l border-line bg-surface shadow-[var(--shadow-pop)]" role="dialog" :aria-label="hit.title">
      <header class="flex items-start gap-3 border-b border-line p-4">
        <ProjectIcon :hit="hit" :size="56" />
        <div class="min-w-0 flex-1">
          <h2 class="truncate text-[17px] font-bold">{{ hit.title }}</h2>
          <p class="text-muted">{{ t.browse.by }} {{ hit.author }} · {{ formatCount(hit.downloads) }} {{ t.browse.downloads }}</p>
          <div class="mt-1.5 flex flex-wrap gap-1">
            <span class="badge badge-accent">{{ hit.platform === 'modrinth' ? 'Modrinth' : 'CurseForge' }}</span>
            <span v-for="c in hit.categories" :key="c" class="badge">{{ c }}</span>
          </div>
        </div>
        <button class="btn btn-ghost btn-icon" :aria-label="t.common.close" @click="emit('close')"><X :size="16" /></button>
      </header>

      <div class="flex flex-col gap-3 border-b border-line p-4">
        <p class="text-muted">{{ hit.description }}</p>
        <label v-if="isPack" class="flex flex-col gap-1">
          <span class="label">{{ t.browse.packName }}</span>
          <input v-model="packName" class="input" />
        </label>
        <div v-else class="flex flex-col gap-1">
          <span class="label">{{ t.browse.target }}</span>
          <SelectMenu v-model="target" :options="targetOptions" :disabled="!store.instances.length" :aria-label="t.browse.target" />
          <span v-if="!store.instances.length" class="text-warn">{{ t.browse.noInstance }}</span>
        </div>
      </div>

      <div class="min-h-0 flex-1 overflow-y-auto p-4">
        <div class="mb-2 flex items-center gap-3">
          <h3 class="section-title mr-auto">{{ t.browse.versions }}</h3>
          <label v-if="requirement.gameVersion || requirement.loader" class="flex items-center gap-2 text-muted">
            <ToggleSwitch v-model="showAll" :label="t.browse.showAllVersions" />
            {{ t.browse.showAllVersions }}
          </label>
        </div>
        <p v-if="!showAll && (requirement.gameVersion || requirement.loader)" class="mb-2 text-[12px] text-faint">
          {{ t.browse.filteredFor([requirement.gameVersion, requirement.loader ? LOADER_LABELS[requirement.loader] : ''].filter(Boolean).join(' · ')) }}
        </p>
        <p v-if="loading" class="text-faint">{{ t.common.loading }}</p>
        <p v-else-if="!versions.length" class="text-faint">{{ t.browse.noVersions }}</p>
        <ul v-else class="flex flex-col gap-2">
          <li v-for="v in versions" :key="v.id" class="card bg-raised p-3">
            <div class="flex items-center gap-2">
              <span class="min-w-0 flex-1 truncate font-semibold">{{ v.name }}</span>
              <span v-if="mismatch(v)" class="badge border-danger/50 bg-danger/15 text-danger" :title="t.browse.incompatibleHint">
                <TriangleAlert :size="12" />{{ t.browse.incompatible(mismatch(v) as string) }}
              </span>
              <span class="badge" :class="releaseClass[v.releaseType]">{{ releaseLabel[v.releaseType] }}</span>
            </div>
            <div class="mt-1 flex flex-wrap gap-1">
              <span v-for="g in v.gameVersions" :key="g" class="badge">{{ g }}</span>
              <span v-for="l in v.loaders" :key="l" class="badge">{{ l }}</span>
            </div>
            <div class="mt-2 flex items-center gap-2">
              <span class="min-w-0 flex-1 truncate text-[12px] text-faint">
                {{ v.files[0]?.filename }} · {{ formatBytes(v.files[0]?.size ?? 0) }} · {{ formatDate(v.published) }}
              </span>
              <template v-if="v.files.some((f) => f.url === null)">
                <button class="btn h-7" @click="tasks.guard(() => openExternal(v.files.find((f) => f.url === null)?.downloadPage ?? ''))">
                  <ExternalLink :size="14" />{{ t.browse.blockedLink }}
                </button>
              </template>
              <button
                v-else
                class="btn btn-primary h-7"
                :disabled="busy !== null || (!isPack && !target)"
                @click="install(v)"
              >
                <Download :size="14" />{{ installedIds.has(v.id) ? t.browse.installed : isPack ? t.browse.createFromPack : t.browse.install }}
              </button>
            </div>
            <p v-if="v.files.some((f) => f.url === null)" class="mt-2 flex items-start gap-1.5 rounded-md bg-warn/10 p-2 text-[12px] text-warn">
              <ShieldAlert :size="14" class="mt-px shrink-0" />
              <span>{{ t.browse.blocked }}. {{ t.browse.blockedHint }}</span>
            </p>
          </li>
        </ul>
      </div>
      <footer class="border-t border-line p-3 text-[12px] text-faint"><ArrowUpRight :size="12" class="inline" /> {{ hit.slug }}</footer>
    </aside>
  </Teleport>
</template>
