<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import type { Instance, InstanceSettings } from '../../api/types'
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
  shareResourcepacks: props.instance.shareResourcepacks,
  settings: clone(props.instance.settings) as InstanceSettings,
})
const dirty = ref(false)
const markDirty = () => (dirty.value = true)

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
const memoryView = computed(() => draft.settings.memory ?? global.value?.memory ?? { minMib: 512, maxMib: 4096 })
const windowView = computed(() => draft.settings.window ?? global.value?.window ?? { width: 854, height: 480, maximized: false })
const commandsView = computed(() => draft.settings.commands ?? global.value?.commands ?? { preLaunch: '', postExit: '', wrapper: '' })

async function save() {
  const saved = await tasks.guard(() =>
    store.updateSettings(props.instance.id, {
      icon: draft.icon,
      notes: draft.notes,
      shareResourcepacks: draft.shareResourcepacks,
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
      <label class="flex items-center gap-3">
        <ToggleSwitch v-model="draft.shareResourcepacks" :label="t.iset.shareResourcepacks" @update:model-value="markDirty" />
        <span>
          <span class="block font-medium">{{ t.iset.shareResourcepacks }}</span>
          <span class="text-[12px] text-faint">{{ t.iset.shareHint }}</span>
        </span>
      </label>
    </section>

    <OverrideBlock v-model="javaOn" :title="t.iset.java" :override-label="t.iset.overrideJava">
      <template #default="{ disabled }">
        <div class="flex flex-col gap-3">
          <input
            class="input font-mono"
            :disabled="disabled"
            :value="javaView.path ?? ''"
            :placeholder="t.settings.javaPath"
            @input="draft.settings.java && (draft.settings.java.path = ($event.target as HTMLInputElement).value || null)"
          />
          <label class="flex items-center gap-2">
            <ToggleSwitch
              :model-value="javaView.autoSelect"
              :disabled="disabled"
              :label="t.settings.autoSelect"
              @update:model-value="draft.settings.java && ((draft.settings.java.autoSelect = $event), markDirty())"
            />
            <span>{{ t.settings.autoSelect }}</span>
          </label>
        </div>
      </template>
    </OverrideBlock>

    <OverrideBlock v-model="memoryOn" :title="t.iset.memory" :override-label="t.iset.overrideMemory">
      <template #default="{ disabled }">
        <MemoryFields
          :model-value="memoryView"
          :disabled="disabled"
          @update:model-value="((draft.settings.memory = $event), markDirty())"
        />
      </template>
    </OverrideBlock>

    <OverrideBlock v-model="jvmOn" :title="t.iset.jvmArgs" :override-label="t.iset.overrideJvm">
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

    <OverrideBlock v-model="windowOn" :title="t.iset.window" :override-label="t.iset.overrideWindow">
      <template #default="{ disabled }"><WindowFields :model-value="windowView" :disabled="disabled" /></template>
    </OverrideBlock>

    <OverrideBlock v-model="commandsOn" :title="t.iset.commands" :override-label="t.iset.overrideCommands">
      <template #default="{ disabled }"><CommandFields :model-value="commandsView" :disabled="disabled" /></template>
    </OverrideBlock>

    <div class="sticky bottom-4 flex justify-end">
      <button class="btn btn-primary btn-lg shadow-[var(--shadow-pop)]" :disabled="!dirty" @click="save">{{ t.common.save }}</button>
    </div>
  </div>
</template>
