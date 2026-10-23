<script setup lang="ts">
import { ArrowUpRight, Download, ExternalLink, ShieldAlert, X } from '@lucide/vue'
import { openUrl } from '@tauri-apps/plugin-opener'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { getProjectVersions, installVersion, isTauri } from '../api/commands'
import type { LoaderId, ProjectVersion, SearchHit } from '../api/types'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { formatBytes, formatCount, formatDate } from '../utils/format'
import ProjectIcon from './ProjectIcon.vue'

const props = defineProps<{ hit: SearchHit; instanceId: string; gameVersion?: string; loader?: LoaderId }>()
const emit = defineEmits<{ close: [] }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const versions = ref<ProjectVersion[]>([])
const loading = ref(true)
const target = ref(props.instanceId || store.instances[0]?.id || '')
const packName = ref(props.hit.title)
const busy = ref<string | null>(null)
const installedIds = ref(new Set<string>())

const isPack = computed(() => props.hit.kind === 'modpack')

onMounted(async () => {
  window.addEventListener('keydown', onKey)
  versions.value =
    (await tasks.guard(() => getProjectVersions(props.hit.platform, props.hit.projectId, { gameVersion: props.gameVersion, loader: props.loader }))) ?? []
  loading.value = false
})
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))
function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}

async function openManual(url: string) {
  if (isTauri) await tasks.guard(() => openUrl(url))
  else window.open(url, '_blank', 'noopener')
}

async function install(v: ProjectVersion) {
  busy.value = v.id
  if (isPack.value) {
    const inst = await tasks.guard(() => store.installModpack(v, packName.value.trim() || undefined))
    if (inst) tasks.toast(t.browse.installedOk(inst.name), 'ok')
  } else if (props.hit.kind !== 'modpack') {
    const kind = props.hit.kind
    const outcome = await tasks.guard(() => installVersion(target.value, v, kind))
    if (outcome?.status === 'installed') {
      installedIds.value = new Set(installedIds.value).add(v.id)
      tasks.toast(t.browse.installedOk(outcome.fileName), 'ok')
    } else if (outcome?.status === 'blocked') {
      tasks.toast(t.browse.blocked, 'error')
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
        <label v-else class="flex flex-col gap-1">
          <span class="label">{{ t.browse.target }}</span>
          <select v-model="target" class="input" :disabled="!store.instances.length">
            <option v-for="i in store.instances" :key="i.id" :value="i.id">{{ i.name }}</option>
          </select>
          <span v-if="!store.instances.length" class="text-warn">{{ t.browse.noInstance }}</span>
        </label>
      </div>

      <div class="min-h-0 flex-1 overflow-y-auto p-4">
        <h3 class="section-title mb-2">{{ t.browse.versions }}</h3>
        <p v-if="loading" class="text-faint">{{ t.common.loading }}</p>
        <p v-else-if="!versions.length" class="text-faint">{{ t.browse.noVersions }}</p>
        <ul v-else class="flex flex-col gap-2">
          <li v-for="v in versions" :key="v.id" class="card bg-raised p-3">
            <div class="flex items-center gap-2">
              <span class="min-w-0 flex-1 truncate font-semibold">{{ v.name }}</span>
              <span class="badge" :class="releaseClass[v.releaseType]">{{ releaseLabel[v.releaseType] }}</span>
            </div>
            <div class="mt-1 flex flex-wrap gap-1">
              <span v-for="g in v.gameVersions" :key="g" class="badge">{{ g }}</span>
              <span v-for="l in v.loaders" :key="l" class="badge">{{ l }}</span>
            </div>
            <div class="mt-2 flex items-center gap-2">
              <span class="min-w-0 flex-1 truncate text-[12px] text-faint">
                {{ v.files[0]?.fileName }} · {{ formatBytes(v.files[0]?.size ?? 0) }} · {{ formatDate(v.datePublished) }}
              </span>
              <template v-if="v.files.some((f) => f.blocked)">
                <button class="btn h-7" @click="openManual(v.files.find((f) => f.blocked)?.manualUrl ?? hit.slug)">
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
            <p v-if="v.files.some((f) => f.blocked)" class="mt-2 flex items-start gap-1.5 rounded-md bg-warn/10 p-2 text-[12px] text-warn">
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
