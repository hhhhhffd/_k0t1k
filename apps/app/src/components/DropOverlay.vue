<script setup lang="ts">
import { Download } from '@lucide/vue'

/** `subtle` keeps the window visible (a hint at the bottom), for drops that aim at something on screen. */
defineProps<{ text: string; subtle?: boolean }>()
</script>

<template>
  <Teleport to="body">
    <div
      v-if="subtle"
      class="anim-fade pointer-events-none fixed bottom-12 left-1/2 z-[70] flex -translate-x-1/2 items-center gap-2 rounded-xl border-2 border-dashed border-accent bg-surface px-5 py-3 font-semibold shadow-[var(--shadow-pop)]"
      role="status"
    >
      <Download :size="18" class="text-icon" />{{ text }}
    </div>
    <div v-else class="anim-fade pointer-events-none fixed inset-0 z-[70] flex items-center justify-center bg-black/55 p-8 backdrop-blur-sm" role="status">
      <div class="card flex flex-col items-center gap-3 border-2 border-dashed border-accent px-12 py-10 text-center shadow-[var(--shadow-pop)]">
        <Download :size="32" class="text-icon" />
        <p class="text-[15px] font-semibold">{{ text }}</p>
      </div>
    </div>
  </Teleport>
</template>
