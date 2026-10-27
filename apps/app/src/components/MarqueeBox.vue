<script setup lang="ts">
import type { MarqueeBox } from '../utils/useMarquee'

defineProps<{ box: MarqueeBox | null }>()
</script>

<template>
  <Teleport to="body">
    <div
      v-if="box"
      class="pointer-events-none fixed z-40 rounded-[3px] border border-accent bg-accent/20"
      :style="{ left: `${box.left}px`, top: `${box.top}px`, width: `${box.width}px`, height: `${box.height}px` }"
    />
  </Teleport>
</template>
