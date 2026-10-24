<script setup lang="ts">
import type { MemorySettings } from '../../api/types'
import { t } from '../../i18n/ru'
import ToggleSwitch from '../ToggleSwitch.vue'

const props = defineProps<{ disabled?: boolean; totalRamMib?: number | null }>()
const model = defineModel<MemorySettings>({ required: true })
const max = () => Math.max(props.totalRamMib ?? 16384, 1024)

function setMin(v: number) {
  model.value = { ...model.value, minMib: v, maxMib: Math.max(v, model.value.maxMib) }
}
function setMax(v: number) {
  model.value = { ...model.value, minMib: Math.min(v, model.value.minMib), maxMib: v }
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <label class="flex items-center gap-3">
      <ToggleSwitch :model-value="model.auto" :disabled="disabled" :label="t.settings.autoMemory" @update:model-value="model = { ...model, auto: $event }" />
      <span class="font-medium">{{ t.settings.autoMemory }}</span>
    </label>
    <p v-if="model.auto" class="text-[12px] text-faint">{{ t.settings.autoMemoryHint }}</p>
    <div v-for="row in [
      { label: t.settings.minMem, value: model.minMib, set: setMin },
      { label: t.settings.maxMem, value: model.maxMib, set: setMax },
    ]" :key="row.label" class="grid grid-cols-[90px_1fr_96px] items-center gap-3" :class="{ 'opacity-50': model.auto }">
      <span class="text-muted">{{ row.label }}</span>
      <input type="range" :min="256" :max="max()" :step="256" :value="row.value" :disabled="disabled || model.auto" :aria-label="row.label" @input="row.set(Number(($event.target as HTMLInputElement).value))" />
      <span class="text-right font-mono text-[12.5px]">{{ row.value }} {{ t.common.mb }}</span>
    </div>
    <p v-if="totalRamMib" class="text-[12px] text-faint">{{ t.settings.ramHint(totalRamMib) }}</p>
  </div>
</template>
