<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { computed, reactive, ref } from 'vue'
import { isTauri } from '../../api/commands'
import { type Instance, type InstanceSettings, SHARED_FOLDERS, type SharedFolder } from '../../api/types'
import { t } from '../../i18n/ru'
import { useInstancesStore } from '../../stores/instances'
import { useSettingsStore } from '../../stores/settings'
import { useTasksStore } from '../../stores/tasks'
import { clone } from '../../utils/format'
import CommandFields from '../fields/CommandFields.vue'
import MemoryFields from '../fields/MemoryFields.vue'
import OverrideBlock from '../fields/OverrideBlock.vue'
import WindowFields from '../fields/WindowFields.vue'
import InstanceIcon, { INSTANCE_ICONS } from '../InstanceIcon.vue'
import ToggleSwitch from '../ToggleSwitch.vue'

const props = defineProps<{ instance: Instance }>()
const store = useInstancesStore()
const settings = useSettingsStore()
const tasks = useTasksStore()

const global = computed(() => settings.settings)
const draft = reactive({
  icon: props.instance.icon,
  notes: props.instance.notes,
  joinServer: props.instance.joinServer ?? '',
  sharedFolders: [...props.instance.sharedFolders],
  settings: clone(props.instance.settings) as InstanceSettings,
})
const isShared = (folder: SharedFolder) => draft.sharedFolders.includes(folder)
function setShared(folder: SharedFolder, on: boolean) {
  draft.sharedFolders = SHARED_FOLDERS.filter((f) => (f === folder ? on : isShared(f)))
  markDirty()
}
const dirty = ref(false)
const markDirty = () => (dirty.value = true)

async function pickJava() {
  if (!isTauri || !draft.settings.java) return
  const picked = await open({ multiple: false, directory: false, title: t.iset.pickJavaTitle, filters: [{ name: 'Java', extensions: ['exe'] }] })
  if (typeof picked === 'string') {
    draft.settings.java.path = picked
    markDirty()
  }
}

const isCustomIcon = computed(() => draft.icon.startsWith('custom:'))

/** The picture is stored at once (like the file itself); the built-in choices below only change the draft until saved. */
async function pickIcon() {
  const picked = isTauri
    ? await open({ multiple: false, directory: false, title: t.iset.pickIconTitle, filters: [{ name: 'Image', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif'] }] })
    : 'C:\\Users\\Player\\Pictures\\icon.png'
  if (typeof picked !== 'string') return
  const saved = await tasks.guard(() => store.setIcon(props.instance.id, picked))
  if (saved) draft.icon = saved.icon
}

async function resetIcon() {
  const saved = await tasks.guard(() => store.resetIcon(props.instance.id))
  if (saved) draft.icon = saved.icon
}

/** Toggle for one override group: off -> inherit (group removed), on -> start from the global value. */
function overrideModel<K extends keyof InstanceSettings>(key: K, fromGlobal: () => InstanceSettings[K]) {
  return computed({
    get: () => draft.settings[key] != null,
    set: (on: boolean) => {
      draft.settings[key] = on ? fromGlobal() : undefined
      markDirty()
    },
  })
}
const javaOn = overrideModel('java', () => clone(global.value!.java))
const memoryOn = overrideModel('memory', () => clone(global.value!.memory))
const jvmOn = overrideModel('jvmArgs', () => global.value?.jvmArgs ?? '')
const windowOn = overrideModel('window', () => clone(global.value!.window))
const commandsOn = overrideModel('commands', () => clone(global.value!.commands))

// Views of each group: the override when set, otherwise the (read-only) global value.
const javaView = computed(() => draft.settings.java ?? global.value?.java ?? { path: null, autoSelect: true, autoDownload: true })
const memoryView = computed(() => draft.settings.memory ?? global.value?.memory ?? { minMib: 512, maxMib: 4096, auto: false })
const windowView = computed(() => draft.settings.window ?? global.value?.window ?? { width: 854, height: 480, maximized: false })
const commandsView = computed(() => draft.settings.commands ?? global.value?.commands ?? { preLaunch: '', postExit: '', wrapper: '' })

async function save() {
  const saved = await tasks.guard(() =>
    store.updateSettings(props.instance.id, {
      icon: draft.icon,
      notes: draft.notes,
      joinServer: draft.joinServer.trim() || null,
      sharedFolders: draft.sharedFolders,
      settings: clone(draft.settings),
    }),
  )
  if (saved) {
    dirty.value = false
    tasks.toast(t.iset.saved, 'ok')
  }
}
</script>

<template>
  <div v-if="global" class="flex max-w-3xl flex-col gap-3 pb-16" @input.capture="markDirty" @change.capture="markDirty">
    <section class="card flex flex-col gap-3 p-4">
      <h3 class="text-[14px] font-semibold">{{ t.iset.general }}</h3>
      <div class="flex items-center gap-2">
        <InstanceIcon :icon="draft.icon" :instance-id="instance.id" :size="48" />
        <button class="btn" @click="pickIcon">{{ t.iset.pickIcon }}</button>
        <button v-if="isCustomIcon" class="btn btn-ghost" @click="resetIcon">{{ t.iset.resetIcon }}</button>
      </div>
      <div class="flex flex-wrap gap-1.5" role="group" :aria-label="t.iset.icon">
        <button
          v-for="key in Object.keys(INSTANCE_ICONS)"
          :key="key"
          class="rounded-md p-0.5"
          :class="draft.icon === key ? 'bg-accent/30 ring-2 ring-accent' : 'hover:bg-hover'"
          :aria-label="key"
          @click="((draft.icon = key), markDirty())"
        >
          <InstanceIcon :icon="key" :size="32" />
        </button>
      </div>
      <label class="flex flex-col gap-1">
        <span class="label">{{ t.iset.notes }}</span>
        <textarea v-model="draft.notes" class="input" rows="2" />
      </label>
      <label class="flex flex-col gap-1">
        <span class="label">{{ t.iset.joinServer }}</span>
        <input v-model="draft.joinServer" class="input font-mono" placeholder="play.example.org:25565" />
        <span class="text-[12px] text-faint">{{ t.iset.joinServerHint }}</span>
      </label>
    </section>

    <section class="card flex flex-col gap-3 p-4">
      <h3 class="text-[14px] font-semibold">{{ t.iset.sharedFolders }}</h3>
      <p class="text-[12px] text-faint">{{ t.iset.sharedFoldersHint }}</p>
      <label v-for="folder in SHARED_FOLDERS" :key="folder" class="flex items-center gap-3">
        <ToggleSwitch
          :model-value="isShared(folder)"
          :label="t.sharedFolder.names[folder]"
          @update:model-value="setShared(folder, $event)"
        />
        <span class="font-medium">{{ t.sharedFolder.names[folder] }}</span>
      </label>
    </section>

    <section class="card divide-y divide-line p-4">
      <OverrideBlock v-model="javaOn" :title="t.iset.java">
        <template #default="{ disabled }">
          <div class="flex gap-2">
            <input
              class="input flex-1 font-mono"
              :value="javaView.path ?? ''"
              :disabled="disabled"
              :placeholder="t.iset.javaPathPlaceholder"
              @input="draft.settings.java && ((draft.settings.java.path = ($event.target as HTMLInputElement).value || null), markDirty())"
            />
            <button class="btn" :disabled="disabled" @click="pickJava">{{ t.iset.browse }}</button>
          </div>
        </template>
      </OverrideBlock>

      <OverrideBlock v-model="memoryOn" :title="t.iset.memory">
        <template #default="{ disabled }">
          <MemoryFields
            :model-value="memoryView"
            :disabled="disabled"
            @update:model-value="((draft.settings.memory = $event), markDirty())"
          />
        </template>
      </OverrideBlock>

      <OverrideBlock v-model="jvmOn" :title="t.iset.jvmArgs">
        <template #default="{ disabled }">
          <textarea
            class="input font-mono"
            rows="2"
            :disabled="disabled"
            :value="draft.settings.jvmArgs ?? global.jvmArgs"
            @input="draft.settings.jvmArgs = ($event.target as HTMLTextAreaElement).value"
          />
        </template>
      </OverrideBlock>

      <OverrideBlock v-model="windowOn" :title="t.iset.window">
        <template #default="{ disabled }"><WindowFields :model-value="windowView" :disabled="disabled" /></template>
      </OverrideBlock>

      <OverrideBlock v-model="commandsOn" :title="t.iset.commands">
        <template #default="{ disabled }"><CommandFields :model-value="commandsView" :disabled="disabled" /></template>
      </OverrideBlock>
    </section>

    <div class="sticky bottom-4 flex justify-end">
      <button class="btn btn-primary btn-lg shadow-[var(--shadow-pop)]" :disabled="!dirty" @click="save">{{ t.common.save }}</button>
    </div>
  </div>
</template>
