<script setup lang="ts">
import { t } from '../../i18n/ru'
import ToggleSwitch from '../ToggleSwitch.vue'

defineProps<{ title: string; overrideLabel: string }>()
/** true = this instance overrides the global value. */
const overridden = defineModel<boolean>({ required: true })
</script>

<template>
  <section class="card p-4">
    <header class="mb-3 flex items-center justify-between">
      <h3 class="text-[14px] font-semibold">{{ title }}</h3>
      <label class="flex items-center gap-2 text-muted">
        <span>{{ overridden ? overrideLabel : t.common.useGlobal }}</span>
        <ToggleSwitch v-model="overridden" :label="overrideLabel" />
      </label>
    </header>
    <div :class="overridden ? '' : 'opacity-60'"><slot :disabled="!overridden" /></div>
  </section>
</template>
