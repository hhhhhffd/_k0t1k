<script setup lang="ts">
import ToggleSwitch from '../ToggleSwitch.vue'

defineProps<{ title: string; overrideLabel: string }>()
/** true = this instance overrides the global value. */
const overridden = defineModel<boolean>({ required: true })
</script>

<template>
  <section class="card p-4">
    <header class="flex items-center justify-between" :class="overridden ? 'mb-3' : ''">
      <h3 class="text-[14px] font-semibold">{{ title }}</h3>
      <label class="flex items-center gap-2 text-muted">
        <span>{{ overrideLabel }}</span>
        <ToggleSwitch v-model="overridden" :label="overrideLabel" />
      </label>
    </header>
    <slot v-if="overridden" :disabled="false" />
  </section>
</template>
