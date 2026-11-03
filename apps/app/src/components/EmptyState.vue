<script setup lang="ts">
import mascot from '../assets/mascot.png'

/** A friendly "nothing here yet" block; the default slot holds the actions that fill it. */
defineProps<{ title: string; hint?: string }>()
</script>

<template>
  <div class="flex flex-col items-center gap-2 py-12 text-center">
    <img :src="mascot" alt="" class="pixelated anim-bob mb-1 size-16 opacity-90" />
    <p class="text-[15px] font-semibold">{{ title }}</p>
    <p v-if="hint" class="max-w-md text-muted">{{ hint }}</p>
    <div v-if="$slots.default" class="mt-2 flex flex-wrap justify-center gap-2"><slot /></div>
  </div>
</template>
