<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { listLoaderVersions } from '../api/commands'
import type { LoaderId, LoaderVersion } from '../api/types'
import { t } from '../i18n/ru'
import { useTasksStore } from '../stores/tasks'
import { LOADER_LABELS } from '../utils/format'
import SelectMenu from './SelectMenu.vue'

const props = defineProps<{ mcVersion: string }>()
const loader = defineModel<LoaderId | null>('loader', { required: true })
/** Empty while no version fits; a given version is kept when the list for the Minecraft version has it. */
const loaderVersion = defineModel<string>('loaderVersion', { required: true })
const tasks = useTasksStore()

const loaders: LoaderId[] = ['fabric', 'quilt', 'forge', 'neoForge']
const loaderVersions = ref<LoaderVersion[]>([])
const loading = ref(false)

const options = computed(() =>
  loaderVersions.value.map((v) => ({ value: v.version, label: v.version, hint: v.recommended ? t.create.recommended : undefined })),
)

watch(
  [() => props.mcVersion, loader],
  async ([mc, l]) => {
    const wanted = loaderVersion.value
    loaderVersions.value = []
    loaderVersion.value = ''
    if (!mc || !l) return
    loading.value = true
    const list = await tasks.guard(() => listLoaderVersions(mc, l))
    // ignore the response if the selection changed while loading
    if (mc === props.mcVersion && l === loader.value && list) {
      loaderVersions.value = list
      loaderVersion.value = list.find((v) => v.version === wanted)?.version ?? list.find((v) => v.recommended)?.version ?? list[0]?.version ?? ''
    }
    loading.value = false
  },
  { immediate: true },
)
</script>

<template>
  <div class="flex flex-col gap-1">
    <span class="label">{{ t.create.loader }}</span>
    <div class="flex flex-wrap gap-1.5">
      <button class="btn" :class="loader === null ? 'btn-primary' : ''" @click="loader = null">{{ t.common.none }}</button>
      <button v-for="l in loaders" :key="l" class="btn" :class="loader === l ? 'btn-primary' : ''" @click="loader = l">{{ LOADER_LABELS[l] }}</button>
    </div>
  </div>
  <div v-if="loader" class="flex flex-col gap-1">
    <span class="label">{{ t.create.loaderVersion }}</span>
    <p v-if="loading" class="text-faint">{{ t.common.loading }}</p>
    <p v-else-if="!loaderVersions.length" class="text-warn">{{ t.create.loaderUnavailable }}</p>
    <SelectMenu v-else v-model="loaderVersion" searchable :options="options" :aria-label="t.create.loaderVersion" />
  </div>
</template>
