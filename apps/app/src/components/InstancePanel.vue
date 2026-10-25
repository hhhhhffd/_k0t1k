<script setup lang="ts">
import type { Instance } from '../api/types'
import { t } from '../i18n'
import { useLayoutStore } from '../stores/layout'
import { loaderOf, minecraftVersionOf } from '../utils/format'
import type { MenuItem } from './ContextMenu.vue'
import GripHandle from './GripHandle.vue'
import InstanceIcon from './InstanceIcon.vue'
import UiIcon from './UiIcon.vue'

defineProps<{ instance: Instance | undefined; actions: MenuItem[] }>()
const emit = defineEmits<{ rename: []; edit: [] }>()
const layout = useLayoutStore()
</script>

<template>
  <aside class="flex w-52 shrink-0 flex-col bg-surface" :class="layout.panel === 'right' ? 'border-l border-line' : 'border-r border-line'">
    <div class="flex justify-center pt-1.5">
      <GripHandle axis="x" @dock="layout.panel = $event === 'start' ? 'left' : 'right'" />
    </div>
    <template v-if="instance">
      <div class="flex flex-col items-center gap-2 px-3 pt-3 pb-3 text-center">
        <button class="rounded-lg p-1 hover:bg-hover" :title="t.panel.changeIcon" @click="emit('edit')">
          <InstanceIcon :icon="instance.icon" :instance-id="instance.id" :size="72" />
        </button>
        <button class="max-w-full rounded-md px-1.5 py-0.5 font-semibold break-words hover:bg-hover" :title="t.library.rename" @click="emit('rename')">
          {{ instance.name }}
        </button>
        <div class="flex flex-wrap justify-center gap-1">
          <span class="badge">{{ minecraftVersionOf(instance) }}</span>
          <span v-if="loaderOf(instance)" class="badge badge-accent">{{ loaderOf(instance)!.label }}</span>
        </div>
      </div>
      <nav class="flex min-h-0 flex-1 flex-col overflow-y-auto px-1.5 pb-2">
        <template v-for="item in actions" :key="item.label">
          <div v-if="item.separatorBefore" class="mx-1.5 my-1 h-px bg-line" />
          <button class="tool h-8 w-full justify-start" :class="item.danger ? 'text-danger' : ''" :disabled="item.disabled" @click="item.action()">
            <UiIcon v-if="item.icon" :name="item.iconKey ?? ''" :icon="item.icon" :size="16" :class="item.iconClass" />
            <span class="truncate">{{ item.label }}</span>
          </button>
        </template>
      </nav>
    </template>
    <p v-else class="px-4 py-10 text-center text-[12.5px] text-faint">{{ t.panel.nothingSelected }}</p>
  </aside>
</template>
