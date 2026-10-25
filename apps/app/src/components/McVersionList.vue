<script setup lang="ts">
import { RefreshCw } from '@lucide/vue'
import { computed, onMounted, reactive, ref } from 'vue'
import { listMinecraftVersions } from '../api/commands'
import type { MinecraftVersion } from '../api/types'
import { t } from '../i18n'
import { useTasksStore } from '../stores/tasks'
import CheckBox from './CheckBox.vue'

/** Empty until the list is loaded; then the recommended version is picked unless one was given. */
const model = defineModel<string>({ required: true })
const tasks = useTasksStore()

const TYPES = ['release', 'snapshot', 'old_beta', 'old_alpha', 'experiment'] as const
type VersionType = (typeof TYPES)[number]
const versions = ref<MinecraftVersion[]>([])
const loading = ref(false)
const shown = reactive<Record<VersionType, boolean>>({ release: true, snapshot: false, old_beta: false, old_alpha: false, experiment: false })
const search = ref('')

// Prism meta has a few more snapshot-like types; they are filtered with snapshots.
const typeOf = (v: MinecraftVersion): VersionType => (TYPES.find((type) => type === v.type) ?? 'snapshot')
const visible = computed(() => {
  const needle = search.value.trim().toLowerCase()
  return versions.value.filter((v) => shown[typeOf(v)] && v.version.toLowerCase().includes(needle))
})

async function load() {
  loading.value = true
  const list = await tasks.guard(listMinecraftVersions)
  loading.value = false
  if (!list) return
  versions.value = list
  const current = list.find((v) => v.version === model.value)
  if (current) shown[typeOf(current)] = true
  else model.value = list.find((v) => v.recommended)?.version ?? list[0]?.version ?? ''
}
onMounted(load)
</script>

<template>
  <div class="flex min-h-0 flex-1 gap-3">
    <div class="flex min-h-0 min-w-0 flex-1 flex-col gap-2">
      <div class="flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-line bg-base">
        <div class="grid shrink-0 grid-cols-[1fr_110px_100px] border-b border-line px-3 py-1.5 text-[12px] font-semibold text-faint">
          <span>{{ t.create.mcVersion }}</span><span>{{ t.create.released }}</span><span>{{ t.create.type }}</span>
        </div>
        <ul class="min-h-0 flex-1 overflow-y-auto p-1" role="listbox" :aria-label="t.create.mcVersion">
          <li v-for="v in visible" :key="v.version" role="option" :aria-selected="model === v.version">
            <button
              class="grid h-7 w-full grid-cols-[1fr_110px_100px] items-center rounded-md px-2 text-left"
              :class="model === v.version ? 'bg-accent/20 text-accent' : 'hover:bg-hover'"
              @click="model = v.version"
            >
              <span class="truncate font-medium">{{ v.version }}</span>
              <span class="text-[12px] text-muted">{{ new Date(v.releaseTime).toLocaleDateString('ru-RU') }}</span>
              <span class="text-[12px] text-faint">{{ t.create.types[typeOf(v)] }}</span>
            </button>
          </li>
          <li v-if="loading" class="p-3 text-faint">{{ t.common.loading }}</li>
          <li v-else-if="!visible.length" class="p-3 text-faint">{{ t.common.nothingFound }}</li>
        </ul>
      </div>
      <input v-model="search" class="input shrink-0" :placeholder="t.common.search" :aria-label="t.common.search" />
    </div>
    <div class="flex w-44 shrink-0 flex-col gap-1">
      <span class="label mb-1">{{ t.create.show }}</span>
      <CheckBox v-for="type in TYPES" :key="type" v-model="shown[type]" :label="t.create.typeFilters[type]" />
      <button class="btn mt-auto" :disabled="loading" @click="load"><RefreshCw :size="14" />{{ t.create.reload }}</button>
    </div>
  </div>
</template>
