<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { FileUp } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { isTauri } from '../api/commands'
import type { LoaderId } from '../api/types'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useLayoutStore } from '../stores/layout'
import { useTasksStore } from '../stores/tasks'
import { LOADER_LABELS } from '../utils/format'
import InstanceIcon, { INSTANCE_ICONS } from './InstanceIcon.vue'
import LoaderPicker from './LoaderPicker.vue'
import McVersionList from './McVersionList.vue'
import ModalDialog from './ModalDialog.vue'

const emit = defineEmits<{ close: [] }>()
const store = useInstancesStore()
const tasks = useTasksStore()
const layout = useLayoutStore()

const name = ref('')
const nameTouched = ref(false)
const group = ref('')
const icon = ref('grass')
const mcVersion = ref('')
const loader = ref<LoaderId | null>(null)
const loaderVersion = ref('')

const canCreate = computed(
  () => name.value.trim() !== '' && mcVersion.value !== '' && (!loader.value || loaderVersion.value !== ''),
)

// default name follows the chosen version until the user types their own
watch([mcVersion, loader], () => {
  if (!nameTouched.value && mcVersion.value) {
    name.value = loader.value ? `${LOADER_LABELS[loader.value]} ${mcVersion.value}` : mcVersion.value
  }
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
  void tasks.guard(() => store.create(request)).then((created) => {
    if (created) layout.selectedId = created.id
  })
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
        <LoaderPicker v-model:loader="loader" v-model:loader-version="loaderVersion" :mc-version="mcVersion" />
      </div>

      <McVersionList v-model="mcVersion" />
    </div>

    <template #footer>
      <button class="btn mr-auto" @click="importPack"><FileUp :size="15" />{{ t.create.importButton }}</button>
      <button class="btn" @click="emit('close')">{{ t.common.cancel }}</button>
      <button class="btn btn-primary" :disabled="!canCreate" @click="create">{{ t.common.create }}</button>
    </template>
  </ModalDialog>
</template>
