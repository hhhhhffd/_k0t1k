<script lang="ts">
import { Axe, Flame, Gem, Ghost, Leaf, Pickaxe, Skull, Sword, Zap, Box } from '@lucide/vue'
import type { Component } from 'vue'

export const INSTANCE_ICONS: Record<string, { icon: Component | null; hue: number }> = {
  mascot: { icon: null, hue: 275 },
  grass: { icon: Box, hue: 130 },
  sword: { icon: Sword, hue: 210 },
  pickaxe: { icon: Pickaxe, hue: 35 },
  axe: { icon: Axe, hue: 20 },
  flame: { icon: Flame, hue: 15 },
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
import mascot from '../assets/mascot.png'
import { useInstancesStore } from '../stores/instances'

const CUSTOM_PREFIX = 'custom:'
const props = withDefaults(defineProps<{ icon: string; size?: number; instanceId?: string }>(), { size: 48 })
const store = useInstancesStore()
const spec = computed(() => INSTANCE_ICONS[props.icon] ?? INSTANCE_ICONS.grass!)

// A custom icon is a file in the instance folder (its name changes with every new icon, so nothing stale is cached).
const customUrl = computed(() => {
  const dir = store.instancesDir
  if (!props.icon.startsWith(CUSTOM_PREFIX) || !props.instanceId || !dir) return null
  const sep = dir.includes('\\') ? '\\' : '/'
  return fileUrl([dir, props.instanceId, props.icon.slice(CUSTOM_PREFIX.length)].join(sep))
})
const broken = ref(false)
watch(customUrl, () => (broken.value = false))
const style = computed(() => ({
  width: `${props.size}px`,
  height: `${props.size}px`,
  background: `linear-gradient(145deg, hsl(${spec.value.hue} 55% 38%), hsl(${spec.value.hue} 50% 20%))`,
}))
</script>

<template>
  <span class="pix inline-flex shrink-0 items-center justify-center" :style="style">
    <img v-if="customUrl && !broken" :src="customUrl" alt="" class="size-full object-cover" @error="broken = true" />
    <img v-else-if="!spec.icon" :src="mascot" alt="" class="pixelated" :style="{ width: `${size * 0.8}px` }" />
    <component :is="spec.icon" v-else :size="size * 0.52" color="#fff" :stroke-width="2" />
  </span>
</template>
