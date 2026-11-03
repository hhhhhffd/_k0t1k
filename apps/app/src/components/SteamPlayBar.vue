<script setup lang="ts">
import { Clock, FolderOpen, History, Layers, RefreshCw, Upload } from '@lucide/vue'
import { computed } from 'vue'
import type { Instance } from '../api/types'
import { t } from '../i18n'
import { formatPlaytime, formatRelative, loaderOf, minecraftVersionOf } from '../utils/format'
import PlayButton from './PlayButton.vue'
import UiIcon from './UiIcon.vue'

/** The play bar of an instance page in the Steam-like mode: play, the version, the play times and the folder tools. */
const props = defineProps<{ instance: Instance }>()
const emit = defineEmits<{ export: [e: MouseEvent]; update: []; folder: [] }>()

const version = computed(() => minecraftVersionOf(props.instance))
const loader = computed(() => loaderOf(props.instance))
</script>

<template>
  <div class="flex flex-wrap items-center gap-x-8 gap-y-3 px-6 py-4">
    <PlayButton :instance-id="instance.id" large class="steam-play" />
    <div class="flex items-center gap-2.5">
      <UiIcon name="steam-version" :icon="Layers" :size="26" class="text-icon" />
      <div class="leading-tight">
        <div class="stat-label">{{ t.steam.version }}</div>
        <div class="text-[12.5px] text-muted">Minecraft {{ version }}<template v-if="loader"> · {{ loader.label }} {{ loader.version }}</template></div>
      </div>
    </div>
    <div class="flex items-center gap-2.5">
      <UiIcon name="steam-last-played" :icon="History" :size="26" class="text-icon" />
      <div class="leading-tight">
        <div class="stat-label">{{ t.steam.lastPlayed }}</div>
        <div class="text-[12.5px] text-muted">{{ formatRelative(instance.lastPlayed) }}</div>
      </div>
    </div>
    <div class="flex items-center gap-2.5">
      <UiIcon name="steam-played" :icon="Clock" :size="26" class="text-icon" />
      <div class="leading-tight">
        <div class="stat-label">{{ t.steam.played }}</div>
        <div class="text-[12.5px] text-muted">{{ formatPlaytime(instance.playtime.totalSecs) }}</div>
      </div>
    </div>
    <div class="ml-auto flex gap-1.5">
      <button v-if="instance.managedPack" class="btn btn-icon h-9 w-9" :title="t.pack.check" :aria-label="t.pack.check" @click="emit('update')"><UiIcon name="update" :icon="RefreshCw" :size="17" class="text-icon" /></button>
      <button class="btn btn-icon h-9 w-9" :title="t.transfer.export" :aria-label="t.transfer.export" @click="emit('export', $event)"><UiIcon name="export" :icon="Upload" :size="17" class="text-icon" /></button>
      <button class="btn btn-icon h-9 w-9" :title="t.instance.instanceFolderHint" :aria-label="t.instance.instanceFolder" @click="emit('folder')"><UiIcon name="folder" :icon="FolderOpen" :size="17" class="text-icon" /></button>
    </div>
  </div>
</template>

<style scoped>
.stat-label {
  font-size: 12.5px;
  font-weight: 700;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
/* Steam's green play button, from the theme's own "ok" colour. :deep because PlayButton has several roots,
   so this component's scope attribute does not reach the button. */
:deep(.steam-play.btn-primary) {
  background: linear-gradient(90deg, var(--c-ok), color-mix(in srgb, var(--c-ok) 75%, white));
  color: #fff;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  min-width: 11rem;
}
:deep(.steam-play.btn-primary:hover:not(:disabled)) {
  filter: brightness(1.08);
}
</style>
