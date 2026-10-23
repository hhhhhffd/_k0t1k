<script setup lang="ts">
import mascot from './assets/mascot.png'
</script>

<template>
  <main class="flex h-screen items-center justify-center gap-4 bg-neutral-950 text-neutral-100">
    <img :src="mascot" alt="" class="size-16 [image-rendering:pixelated]" />
    <h1 class="text-2xl font-bold">_k0t1k</h1>
  </main>
</template>
