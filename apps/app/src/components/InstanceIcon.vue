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
import { computed } from 'vue'
import mascot from '../assets/mascot.png'

const props = withDefaults(defineProps<{ icon: string; size?: number }>(), { size: 48 })
const spec = computed(() => INSTANCE_ICONS[props.icon] ?? INSTANCE_ICONS.grass!)
const style = computed(() => ({
  width: `${props.size}px`,
  height: `${props.size}px`,
  background: `linear-gradient(145deg, hsl(${spec.value.hue} 55% 38%), hsl(${spec.value.hue} 50% 20%))`,
}))
</script>

<template>
  <span class="pix inline-flex shrink-0 items-center justify-center" :style="style">
    <img v-if="!spec.icon" :src="mascot" alt="" class="pixelated" :style="{ width: `${size * 0.8}px` }" />
    <component :is="spec.icon" v-else :size="size * 0.52" color="#fff" :stroke-width="2" />
  </span>
</template>
