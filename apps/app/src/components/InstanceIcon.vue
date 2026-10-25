<script lang="ts">
import { Axe, Gem, Ghost, Leaf, Pickaxe, Skull, Sword, Zap, Box } from '@lucide/vue'
import type { Component } from 'vue'
import mascot from '../assets/mascot.png'
import iconIndex from '../../../../icons/instances/index.json'

// The launcher's instance icons; `icons/instances/index.json` gives their keys (Prism's icon keys, so imported
// Prism instances keep theirs) and the picker order.
const files = import.meta.glob<string>('../../../../icons/instances/*.svg', { eager: true, import: 'default' })
const iconUrl = (file: string) => files[`../../../../icons/instances/${file}`]!

/** Built-in icons offered in the pickers, mascot first. */
export const INSTANCE_ICONS: Record<string, string> = {
  mascot,
  ...Object.fromEntries(iconIndex.map((entry) => [entry.id, iconUrl(entry.file)])),
}

// Icons of earlier versions of the launcher: still drawn for instances that use them, no longer offered.
const LEGACY_ICONS: Record<string, { icon: Component; hue: number }> = {
  box: { icon: Box, hue: 130 },
  sword: { icon: Sword, hue: 210 },
  pickaxe: { icon: Pickaxe, hue: 35 },
  axe: { icon: Axe, hue: 20 },
  gem: { icon: Gem, hue: 185 },
  skull: { icon: Skull, hue: 0 },
  leaf: { icon: Leaf, hue: 95 },
  zap: { icon: Zap, hue: 55 },
  ghost: { icon: Ghost, hue: 300 },
}
</script>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { fileUrl } from '../api/commands'
import { useInstancesStore } from '../stores/instances'

const CUSTOM_PREFIX = 'custom:'
const props = withDefaults(defineProps<{ icon: string; size?: number; instanceId?: string }>(), { size: 48 })
const store = useInstancesStore()
const legacy = computed(() => LEGACY_ICONS[props.icon])
const builtIn = computed(() => INSTANCE_ICONS[props.icon] ?? INSTANCE_ICONS.grass!)

// A custom icon is a file in the instance folder (its name changes with every new icon, so nothing stale is cached).
const customUrl = computed(() => {
  const dir = store.instancesDir
  if (!props.icon.startsWith(CUSTOM_PREFIX) || !props.instanceId || !dir) return null
  const sep = dir.includes('\\') ? '\\' : '/'
  return fileUrl([dir, props.instanceId, props.icon.slice(CUSTOM_PREFIX.length)].join(sep))
})
const broken = ref(false)
watch(customUrl, () => (broken.value = false))
const box = computed(() => ({ width: `${props.size}px`, height: `${props.size}px` }))
</script>

<template>
  <span v-if="customUrl && !broken" class="inline-flex shrink-0 overflow-hidden rounded-md" :style="box">
    <img :src="customUrl" alt="" class="size-full object-cover" @error="broken = true" />
  </span>
  <span
    v-else-if="legacy"
    class="pix inline-flex shrink-0 items-center justify-center"
    :style="{ ...box, background: `linear-gradient(145deg, hsl(${legacy.hue} 55% 38%), hsl(${legacy.hue} 50% 20%))` }"
  >
    <component :is="legacy.icon" :size="size * 0.52" color="#fff" :stroke-width="2" />
  </span>
  <img v-else :src="builtIn" alt="" class="shrink-0 object-contain" :class="{ pixelated: builtIn === mascot }" :style="box" />
</template>
