<script setup lang="ts">
import { Clock, History } from '@lucide/vue'
import type { Instance } from '../api/types'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { formatPlaytime, formatRelative, loaderOf, minecraftVersionOf } from '../utils/format'
import InstanceIcon from './InstanceIcon.vue'
import PlayButton from './PlayButton.vue'

const props = defineProps<{ instance: Instance; list?: boolean }>()
const emit = defineEmits<{ open: []; menu: [e: MouseEvent] }>()
const store = useInstancesStore()
const loader = () => loaderOf(props.instance)
</script>

<template>
  <div
    class="card group relative cursor-pointer transition-colors hover:border-accent/60"
    :class="list ? 'flex items-center gap-3 px-3 py-2' : 'flex flex-col gap-3 p-3'"
    tabindex="0"
    role="button"
    @click="emit('open')"
    @keydown.enter="emit('open')"
    @contextmenu.prevent="emit('menu', $event)"
  >
    <div class="flex min-w-0 items-center gap-3" :class="list ? 'flex-1' : ''">
      <InstanceIcon :icon="instance.icon" :size="list ? 40 : 52" />
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2">
          <span class="truncate text-[14px] font-semibold">{{ instance.name }}</span>
          <span v-if="store.stateOf(instance.id) === 'running'" class="size-2 shrink-0 rounded-full bg-ok shadow-[0_0_8px_var(--c-ok)]" :title="t.library.running" />
        </div>
        <div class="mt-1 flex flex-wrap gap-1">
          <span class="badge">{{ minecraftVersionOf(instance) }}</span>
          <span v-if="loader()" class="badge badge-accent">{{ loader()!.label }}</span>
          <span v-if="instance.managedPack" class="badge">{{ t.library.managedPack }}</span>
        </div>
      </div>
    </div>
    <div class="flex text-[12px] text-muted" :class="list ? 'w-72 shrink-0 gap-4' : 'flex-col gap-1'">
      <span class="flex items-center gap-1.5" :title="t.library.playtime"><Clock :size="13" class="text-faint" />{{ formatPlaytime(instance.playtime.totalSecs) }}</span>
      <span class="flex items-center gap-1.5" :title="t.library.played"><History :size="13" class="text-faint" />{{ formatRelative(instance.lastPlayed) }}</span>
    </div>
    <PlayButton :instance-id="instance.id" :class="list ? '' : 'w-full'" />
  </div>
</template>
