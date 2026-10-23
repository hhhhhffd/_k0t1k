<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { FileUp } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { isTauri, listLoaderVersions, listMinecraftVersions } from '../api/commands'
import type { LoaderId, LoaderVersion, MinecraftVersion } from '../api/types'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { LOADER_LABELS } from '../utils/format'
import InstanceIcon, { INSTANCE_ICONS } from './InstanceIcon.vue'
import ModalDialog from './ModalDialog.vue'
import SelectMenu from './SelectMenu.vue'

const emit = defineEmits<{ close: [] }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const name = ref('')
const nameTouched = ref(false)
const group = ref('')
const icon = ref('grass')
const versions = ref<MinecraftVersion[]>([])
const versionFilter = ref<'release' | 'snapshot' | 'old'>('release')
const versionSearch = ref('')
const mcVersion = ref('')
const loader = ref<LoaderId | null>(null)
const loaderVersions = ref<LoaderVersion[]>([])
const loaderVersion = ref('')
const loadingLoader = ref(false)

const loaders: LoaderId[] = ['fabric', 'quilt', 'forge', 'neoForge']

const visibleVersions = computed(() =>
  versions.value.filter((v) => {
    const matchesType =
      versionFilter.value === 'old' ? (v.type ?? '').startsWith('old') : v.type === versionFilter.value
    return matchesType && v.version.toLowerCase().includes(versionSearch.value.trim().toLowerCase())
  }),
)
const loaderVersionOptions = computed(() =>
  loaderVersions.value.map((v) => ({ value: v.version, label: v.version, hint: v.recommended ? t.create.recommended : undefined })),
)
const canCreate = computed(
  () => name.value.trim() !== '' && mcVersion.value !== '' && (!loader.value || loaderVersion.value !== ''),
)

onMounted(async () => {
  const list = await tasks.guard(listMinecraftVersions)
  if (!list) return
  versions.value = list
  mcVersion.value = list.find((v) => v.recommended)?.version ?? list[0]?.version ?? ''
})

// default name follows the chosen version until the user types their own
watch([mcVersion, loader], () => {
  if (!nameTouched.value && mcVersion.value) {
    name.value = loader.value ? `${LOADER_LABELS[loader.value]} ${mcVersion.value}` : mcVersion.value
  }
})

watch([mcVersion, loader], async ([mc, l]) => {
  loaderVersions.value = []
  loaderVersion.value = ''
  if (!mc || !l) return
  loadingLoader.value = true
  const list = await tasks.guard(() => listLoaderVersions(mc, l))
  // ignore the response if the selection changed while loading
  if (mc === mcVersion.value && l === loader.value && list) {
    loaderVersions.value = list
    loaderVersion.value = list.find((v) => v.recommended)?.version ?? list[0]?.version ?? ''
  }
  loadingLoader.value = false
})

function create() {
  if (!canCreate.value) return
  const request = {
    name: name.value.trim(),
    group: group.value.trim() || undefined,
    icon: icon.value,
    minecraftVersion: mcVersion.value,
    loader: loader.value ? { id: loader.value, version: loaderVersion.value } : undefined,
  }
  emit('close')
  void tasks.guard(() => store.create(request))
}

async function importPack() {
  // Outside Tauri there is no native dialog; the mock accepts a fake path.
  const path = isTauri
    ? await open({ multiple: false, filters: [{ name: t.create.importFilter, extensions: ['mrpack', 'zip'] }] })
    : 'C:\\Downloads\\Example Pack.mrpack'
  if (typeof path !== 'string') return
  emit('close')
  void tasks.guard(() => store.importModpack(path))
}
</script>

<template>
  <ModalDialog :title="t.create.title" :width="760" @close="emit('close')">
    <div class="grid grid-cols-[1fr_1fr] gap-5">
      <div class="flex flex-col gap-3">
        <label class="flex flex-col gap-1">
          <span class="label">{{ t.create.name }}</span>
          <input v-model="name" class="input" autofocus @input="nameTouched = true" />
        </label>
        <label class="flex flex-col gap-1">
          <span class="label">{{ t.create.group }}</span>
          <input v-model="group" class="input" list="group-options" :placeholder="t.create.groupPlaceholder" />
          <datalist id="group-options"><option v-for="g in store.groups" :key="g" :value="g" /></datalist>
        </label>
        <div class="flex flex-col gap-1">
          <span class="label">{{ t.create.icon }}</span>
          <div class="flex flex-wrap gap-1.5">
            <button
              v-for="key in Object.keys(INSTANCE_ICONS)"
              :key="key"
              class="rounded-md p-0.5"
              :class="icon === key ? 'bg-accent/30 ring-2 ring-accent' : 'hover:bg-hover'"
              :aria-label="key"
              :aria-pressed="icon === key"
              @click="icon = key"
            >
              <InstanceIcon :icon="key" :size="34" />
            </button>
          </div>
        </div>
        <div class="flex flex-col gap-1">
          <span class="label">{{ t.create.loader }}</span>
          <div class="flex flex-wrap gap-1.5">
            <button class="btn" :class="loader === null ? 'btn-primary' : ''" @click="loader = null">{{ t.common.none }}</button>
            <button v-for="l in loaders" :key="l" class="btn" :class="loader === l ? 'btn-primary' : ''" @click="loader = l">{{ LOADER_LABELS[l] }}</button>
          </div>
        </div>
        <div v-if="loader" class="flex flex-col gap-1">
          <span class="label">{{ t.create.loaderVersion }}</span>
          <p v-if="loadingLoader" class="text-faint">{{ t.common.loading }}</p>
          <p v-else-if="!loaderVersions.length" class="text-warn">{{ t.create.loaderUnavailable }}</p>
          <SelectMenu v-else v-model="loaderVersion" searchable :options="loaderVersionOptions" :aria-label="t.create.loaderVersion" />
        </div>
      </div>

      <div class="flex min-h-0 flex-col gap-2">
        <span class="label">{{ t.create.mcVersion }}</span>
        <div class="flex gap-1.5">
          <button class="btn h-7 px-2.5" :class="versionFilter === 'release' ? 'btn-primary' : ''" @click="versionFilter = 'release'">{{ t.create.release }}</button>
          <button class="btn h-7 px-2.5" :class="versionFilter === 'snapshot' ? 'btn-primary' : ''" @click="versionFilter = 'snapshot'">{{ t.create.snapshot }}</button>
          <button class="btn h-7 px-2.5" :class="versionFilter === 'old' ? 'btn-primary' : ''" @click="versionFilter = 'old'">{{ t.create.old }}</button>
        </div>
        <input v-model="versionSearch" class="input" :placeholder="t.common.search" />
        <ul class="h-64 overflow-y-auto rounded-lg border border-line bg-base p-1" role="listbox" :aria-label="t.create.mcVersion">
          <li v-for="v in visibleVersions" :key="v.version" role="option" :aria-selected="mcVersion === v.version">
            <button
              class="flex h-8 w-full items-center justify-between rounded-md px-2.5 text-left"
              :class="mcVersion === v.version ? 'bg-accent/20 text-accent' : 'hover:bg-hover'"
              @click="mcVersion = v.version"
            >
              <span class="font-medium">{{ v.version }}</span>
              <span class="text-[11px] text-faint">{{ new Date(v.releaseTime).toLocaleDateString('ru-RU') }}</span>
            </button>
          </li>
          <li v-if="!visibleVersions.length" class="p-3 text-faint">{{ t.common.nothingFound }}</li>
        </ul>
      </div>
    </div>

    <template #footer>
      <button class="btn mr-auto" @click="importPack"><FileUp :size="15" />{{ t.create.importButton }}</button>
      <button class="btn" @click="emit('close')">{{ t.common.cancel }}</button>
      <button class="btn btn-primary" :disabled="!canCreate" @click="create">{{ t.common.create }}</button>
    </template>
  </ModalDialog>
</template>
