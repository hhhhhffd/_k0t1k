<script setup lang="ts">
import { Check } from '@lucide/vue'

const model = defineModel<boolean>({ required: true })
defineProps<{ label: string }>()
</script>

<template>
  <label class="flex cursor-pointer items-center gap-2 py-0.5 select-none">
    <input v-model="model" type="checkbox" class="peer sr-only" />
    <span
      class="flex size-4 shrink-0 items-center justify-center rounded border transition-colors peer-focus-visible:outline-2 peer-focus-visible:outline-accent"
      :class="model ? 'border-accent bg-accent text-accent-fg' : 'border-line bg-base'"
    >
      <Check v-if="model" :size="12" :stroke-width="3.5" />
    </span>
    <span>{{ label }}</span>
  </label>
</template>
