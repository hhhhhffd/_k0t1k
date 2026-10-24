<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { listMinecraftVersions } from '../api/commands'
import type { MinecraftVersion } from '../api/types'
import { t } from '../i18n/ru'
import { useTasksStore } from '../stores/tasks'

/** Empty until the list is loaded; then the recommended version is picked unless one was given. */
const model = defineModel<string>({ required: true })
const tasks = useTasksStore()

const versions = ref<MinecraftVersion[]>([])
const versionFilter = ref<'release' | 'snapshot' | 'old'>('release')
const versionSearch = ref('')

const visibleVersions = computed(() =>
  versions.value.filter((v) => {
    const matchesType =
      versionFilter.value === 'old' ? (v.type ?? '').startsWith('old') : v.type === versionFilter.value
    return matchesType && v.version.toLowerCase().includes(versionSearch.value.trim().toLowerCase())
  }),
)

onMounted(async () => {
  const list = await tasks.guard(listMinecraftVersions)
  if (!list) return
  versions.value = list
  const current = list.find((v) => v.version === model.value)
  if (current) versionFilter.value = (current.type ?? '').startsWith('old') ? 'old' : current.type === 'snapshot' ? 'snapshot' : 'release'
  else model.value = list.find((v) => v.recommended)?.version ?? list[0]?.version ?? ''
})
</script>

<template>
  <div class="flex min-h-0 flex-col gap-2">
    <span class="label">{{ t.create.mcVersion }}</span>
    <div class="flex gap-1.5">
      <button class="btn h-7 px-2.5" :class="versionFilter === 'release' ? 'btn-primary' : ''" @click="versionFilter = 'release'">{{ t.create.release }}</button>
      <button class="btn h-7 px-2.5" :class="versionFilter === 'snapshot' ? 'btn-primary' : ''" @click="versionFilter = 'snapshot'">{{ t.create.snapshot }}</button>
      <button class="btn h-7 px-2.5" :class="versionFilter === 'old' ? 'btn-primary' : ''" @click="versionFilter = 'old'">{{ t.create.old }}</button>
    </div>
    <input v-model="versionSearch" class="input" :placeholder="t.common.search" />
    <ul class="h-64 overflow-y-auto rounded-lg border border-line bg-base p-1" role="listbox" :aria-label="t.create.mcVersion">
      <li v-for="v in visibleVersions" :key="v.version" role="option" :aria-selected="model === v.version">
        <button
          class="flex h-8 w-full items-center justify-between rounded-md px-2.5 text-left"
          :class="model === v.version ? 'bg-accent/20 text-accent' : 'hover:bg-hover'"
          @click="model = v.version"
        >
          <span class="font-medium">{{ v.version }}</span>
          <span class="text-[11px] text-faint">{{ new Date(v.releaseTime).toLocaleDateString('ru-RU') }}</span>
        </button>
      </li>
      <li v-if="!visibleVersions.length" class="p-3 text-faint">{{ t.common.nothingFound }}</li>
    </ul>
  </div>
</template>
