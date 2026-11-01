<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { Instance } from '../api/types'
import { minecraftVersionOf } from '../utils/format'
import InstanceIcon from './InstanceIcon.vue'
import PanoramaView from './PanoramaView.vue'

/** The top of an instance page in the Steam-like mode: its version's title-screen panorama with the name over it. */
const props = defineProps<{ instance: Instance }>()

const version = computed(() => minecraftVersionOf(props.instance))
/** Versions without a panorama (or offline) get a gradient instead. */
const failed = ref(false)
watch(version, () => (failed.value = false))
</script>

<template>
  <div class="hero relative h-[62vh] min-h-80 shrink-0 overflow-hidden">
    <PanoramaView v-if="!failed" :version="version" @failed="failed = true" />
    <div v-else class="hero-fallback absolute inset-0 flex items-center justify-end pr-16">
      <InstanceIcon :icon="instance.icon" :instance-id="instance.id" :size="140" class="opacity-40" />
    </div>
    <div class="hero-shade absolute inset-0" />
    <span v-if="instance.group" class="absolute top-4 right-4 rounded-sm bg-black/55 px-2 py-0.5 text-[11px] font-semibold tracking-wide text-white/85 uppercase">{{ instance.group }}</span>
    <h1 class="hero-title absolute right-6 bottom-6 left-6 line-clamp-2 text-[52px] leading-none font-black tracking-tight text-white">{{ instance.name }}</h1>
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
</style>
