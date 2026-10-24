<script setup lang="ts">
import { computed } from 'vue'
import { fileUrl } from '../api/commands'
import mascot from '../assets/mascot.png'
import { useCustomizationStore } from '../stores/customization'

// Like Prism's cat: a picture in the corner of the instance area, behind the tiles and never in the way of clicks.
const custom = useCustomizationStore()
const src = computed(() => {
  const cat = custom.cat
  if (!cat) return null
  return cat === 'mascot' ? mascot : fileUrl(cat.path)
})
const isVideo = computed(() => custom.cat !== 'mascot' && custom.cat?.kind === 'video')
</script>

<template>
  <div v-if="src" class="pointer-events-none absolute right-3 bottom-0 z-0 flex h-[70%] max-h-[520px] w-[45%] items-end justify-end" aria-hidden="true">
    <video v-if="isVideo" :key="src" :src="src" class="max-h-full max-w-full object-contain" autoplay loop muted playsinline />
    <img v-else :src="src" alt="" class="max-h-full max-w-full object-contain" :class="custom.cat === 'mascot' ? 'pixelated anim-bob h-48 opacity-70' : ''" />
  </div>
</template>
