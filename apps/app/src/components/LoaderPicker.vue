<script setup lang="ts">
import { RefreshCw, Star } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { listLoaderVersions } from '../api/commands'
import type { LoaderId, LoaderVersion } from '../api/types'
import { t } from '../i18n/ru'
import { useTasksStore } from '../stores/tasks'
import { LOADER_LABELS } from '../utils/format'

const props = defineProps<{ mcVersion: string }>()
const loader = defineModel<LoaderId | null>('loader', { required: true })
/** Empty while no version fits; a given version is kept when the list for the Minecraft version has it. */
const loaderVersion = defineModel<string>('loaderVersion', { required: true })
const tasks = useTasksStore()

const loaders: LoaderId[] = ['neoForge', 'forge', 'fabric', 'quilt']
const loaderVersions = ref<LoaderVersion[]>([])
const loading = ref(false)
const search = ref('')

const visible = computed(() => {
  const needle = search.value.trim().toLowerCase()
  return loaderVersions.value.filter((v) => v.version.toLowerCase().includes(needle))
})

async function load() {
  const [mc, l] = [props.mcVersion, loader.value]
  const wanted = loaderVersion.value
  loaderVersions.value = []
  loaderVersion.value = ''
  if (!mc || !l) return
  loading.value = true
  const list = await tasks.guard(() => listLoaderVersions(mc, l))
  // ignore the response if the selection changed while loading
  if (mc !== props.mcVersion || l !== loader.value) return
  loading.value = false
  if (!list) return
  loaderVersions.value = list
  loaderVersion.value = list.find((v) => v.version === wanted)?.version ?? list.find((v) => v.recommended)?.version ?? list[0]?.version ?? ''
}
watch([() => props.mcVersion, loader], load, { immediate: true })
</script>

<template>
  <div class="flex min-h-0 flex-1 gap-3">
    <div class="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
      <div class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-line bg-base">
        <div v-if="!loader" class="flex flex-1 items-center justify-center p-6 text-[15px] font-semibold text-faint">{{ t.create.noLoader }}</div>
        <template v-else>
          <div class="grid shrink-0 grid-cols-[1fr_110px] border-b border-line px-3 py-1.5 text-[12px] font-semibold text-faint">
            <span>{{ t.create.loaderVersion }}</span><span>{{ t.create.released }}</span>
          </div>
          <ul class="min-h-0 flex-1 overflow-y-auto p-1" role="listbox" :aria-label="t.create.loaderVersion">
            <li v-for="v in visible" :key="v.version" role="option" :aria-selected="loaderVersion === v.version">
              <button
                class="grid h-7 w-full grid-cols-[1fr_110px] items-center rounded-md px-2 text-left"
                :class="loaderVersion === v.version ? 'bg-accent/20 text-accent' : 'hover:bg-hover'"
                @click="loaderVersion = v.version"
              >
                <span class="flex min-w-0 items-center gap-1.5">
                  <span class="truncate font-medium">{{ v.version }}</span>
                  <Star v-if="v.recommended" :size="12" class="shrink-0 text-warn" fill="currentColor" :aria-label="t.create.recommended" />
                </span>
                <span class="text-[12px] text-muted">{{ new Date(v.releaseTime).toLocaleDateString('ru-RU') }}</span>
              </button>
            </li>
            <li v-if="loading" class="p-3 text-faint">{{ t.common.loading }}</li>
            <li v-else-if="!loaderVersions.length" class="p-3 text-warn">{{ t.create.loaderUnavailable }}</li>
          </ul>
        </template>
      </div>
      <input v-model="search" class="input shrink-0" :disabled="!loader" :placeholder="t.common.search" :aria-label="t.common.search" />
    </div>
    <div class="flex w-44 shrink-0 flex-col gap-1" role="radiogroup" :aria-label="t.create.loader">
      <span class="label mb-1">{{ t.create.loader }}</span>
      <button v-for="l in [null, ...loaders]" :key="l ?? 'none'" class="flex items-center gap-2 py-0.5 text-left" role="radio" :aria-checked="loader === l" @click="loader = l">
        <span class="flex size-4 shrink-0 items-center justify-center rounded-full border" :class="loader === l ? 'border-accent' : 'border-line bg-base'">
          <span v-if="loader === l" class="size-2 rounded-full bg-accent" />
        </span>
        {{ l ? LOADER_LABELS[l] : t.common.none }}
      </button>
      <button class="btn mt-auto" :disabled="!loader || loading" @click="load"><RefreshCw :size="14" />{{ t.create.reload }}</button>
    </div>
  </div>
</template>
