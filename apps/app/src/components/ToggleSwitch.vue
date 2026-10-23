<script setup lang="ts">
defineProps<{ label?: string; disabled?: boolean }>()
const model = defineModel<boolean>({ required: true })
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="model"
    :aria-label="label"
    :disabled="disabled"
    class="relative h-5 w-9 shrink-0 rounded-full border transition-colors disabled:opacity-40"
    :class="model ? 'border-accent bg-accent' : 'border-line bg-raised'"
    @click="model = !model"
  >
    <span
      class="absolute top-0.5 left-0.5 size-3.5 rounded-full transition-transform"
      :class="model ? 'translate-x-4 bg-accent-fg' : 'bg-faint'"
    />
  </button>
</template>
