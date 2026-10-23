<script setup lang="ts">
import { computed } from 'vue'
import type { SearchHit } from '../api/types'

const props = withDefaults(defineProps<{ hit: SearchHit; size?: number }>(), { size: 48 })
// no icon URL (or mock data): stable colored tile from the project slug
const hue = computed(() => [...props.hit.slug].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7))
</script>

<template>
  <img v-if="hit.iconUrl" :src="hit.iconUrl" alt="" class="shrink-0 rounded-xl object-cover" :style="{ width: `${size}px`, height: `${size}px` }" />
  <span
    v-else
    class="pix flex shrink-0 items-center justify-center font-bold text-white"
    :style="{ width: `${size}px`, height: `${size}px`, fontSize: `${size * 0.42}px`, background: `linear-gradient(145deg, hsl(${hue} 50% 40%), hsl(${hue} 45% 22%))` }"
  >
    {{ hit.title.charAt(0).toUpperCase() }}
  </span>
</template>
