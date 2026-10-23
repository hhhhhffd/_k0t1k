<script setup lang="ts">
import { Download } from '@lucide/vue'

defineProps<{ text: string }>()
</script>

<template>
  <Teleport to="body">
    <div class="anim-fade pointer-events-none fixed inset-0 z-[70] flex items-center justify-center bg-black/55 p-8 backdrop-blur-sm" role="status">
      <div class="card flex flex-col items-center gap-3 border-2 border-dashed border-accent px-12 py-10 text-center shadow-[var(--shadow-pop)]">
        <Download :size="32" class="text-accent" />
        <p class="text-[15px] font-semibold">{{ text }}</p>
      </div>
    </div>
  </Teleport>
</template>
