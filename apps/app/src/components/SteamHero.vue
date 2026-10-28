<script lang="ts">
import { versionArtwork } from '../api/commands'

// One lookup per version and app run; a failed one is forgotten so the next visit tries again.
const artworkCache = new Map<string, Promise<string | null>>()
function artworkOf(version: string): Promise<string | null> {
  let pending = artworkCache.get(version)
  if (!pending) {
    pending = versionArtwork(version).catch((e: unknown) => {
      console.warn(`no artwork for Minecraft ${version}`, e)
      artworkCache.delete(version)
      return null
    })
    artworkCache.set(version, pending)
  }
  return pending
}
</script>

<script setup lang="ts">
import { Clock, FolderOpen, History, Layers, RefreshCw, Settings, Upload } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import type { Instance } from '../api/types'
import { t } from '../i18n'
import { formatPlaytime, formatRelative, loaderOf, minecraftVersionOf } from '../utils/format'
import InstanceIcon from './InstanceIcon.vue'
import PlayButton from './PlayButton.vue'
import UiIcon from './UiIcon.vue'

/** The top of an instance page in the Steam-like mode: the official art of its version and the play bar. */
const props = defineProps<{ instance: Instance }>()
const emit = defineEmits<{ export: [e: MouseEvent]; update: []; folder: []; settings: [] }>()

const version = computed(() => minecraftVersionOf(props.instance))
const loader = computed(() => loaderOf(props.instance))
const artwork = ref<string | null>(null)
const loaded = ref(false)
watch(
  version,
  async (v) => {
    artwork.value = null
    loaded.value = false
    const url = await artworkOf(v)
    if (v === version.value) artwork.value = url
  },
  { immediate: true },
)
</script>

<template>
  <div class="hero relative h-72 shrink-0 overflow-hidden">
    <img v-if="artwork" :src="artwork" alt="" class="absolute inset-0 size-full object-cover transition-opacity duration-500" :class="loaded ? 'opacity-100' : 'opacity-0'" @load="loaded = true" @error="artwork = null" />
    <div v-else class="hero-fallback absolute inset-0 flex items-center justify-end pr-16">
      <InstanceIcon :icon="instance.icon" :instance-id="instance.id" :size="140" class="opacity-40" />
    </div>
    <div class="hero-shade absolute inset-0" />
    <span v-if="instance.group" class="absolute top-4 right-4 rounded-sm bg-black/55 px-2 py-0.5 text-[11px] font-semibold tracking-wide text-white/85 uppercase">{{ instance.group }}</span>
    <h1 class="hero-title absolute right-6 bottom-6 left-6 line-clamp-2 text-[42px] leading-none font-black tracking-tight text-white">{{ instance.name }}</h1>
  </div>

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
      <button class="btn btn-icon h-9 w-9" :title="t.instance.tabs.settings" :aria-label="t.instance.tabs.settings" @click="emit('settings')"><UiIcon name="settings" :icon="Settings" :size="17" class="text-icon" /></button>
    </div>
  </div>
</template>

<style scoped>
.hero-fallback {
  background:
    radial-gradient(circle at 75% 40%, color-mix(in srgb, var(--c-accent) 45%, transparent), transparent 60%),
    linear-gradient(120deg, var(--c-surface), color-mix(in srgb, var(--c-accent) 25%, var(--c-base)));
}
/* fades the art into the page, like Steam's library header */
.hero-shade {
  background: linear-gradient(to bottom, transparent 45%, color-mix(in srgb, var(--c-base) 85%, transparent) 100%);
}
.hero-title {
  text-shadow: 0 2px 18px rgb(0 0 0 / 0.75);
}
.stat-label {
  font-size: 12.5px;
  font-weight: 700;
  letter-spacing: 0.04em;
  text-transform: uppercase;
}
/* Steam's green play button, from the theme's own "ok" colour */
.steam-play.btn-primary {
  background: linear-gradient(90deg, var(--c-ok), color-mix(in srgb, var(--c-ok) 75%, white));
  color: #fff;
  text-transform: uppercase;
  letter-spacing: 0.04em;
  min-width: 11rem;
}
.steam-play.btn-primary:hover:not(:disabled) {
  filter: brightness(1.08);
}
</style>
