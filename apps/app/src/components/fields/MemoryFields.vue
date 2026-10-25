<script setup lang="ts">
import type { MemorySettings } from '../../api/types'
import { t } from '../../i18n'
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
/** Typed value: committed on blur/Enter, clamped to the slider bounds; garbage reverts to the current value. */
function commit(event: Event, set: (v: number) => void, current: number) {
  const el = event.target as HTMLInputElement
  const typed = Number(el.value)
  const v = el.value.trim() === '' || !Number.isFinite(typed) ? current : Math.min(Math.max(Math.round(typed), 256), max())
  el.value = String(v)
  if (v !== current) set(v)
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
    ]" :key="row.label" class="grid grid-cols-[90px_1fr_auto] items-center gap-3" :class="{ 'opacity-50': model.auto }">
      <span class="text-muted">{{ row.label }}</span>
      <input type="range" :min="256" :max="max()" :step="256" :value="row.value" :disabled="disabled || model.auto" :aria-label="row.label" @input="row.set(Number(($event.target as HTMLInputElement).value))" />
      <span class="flex items-center gap-1.5">
        <input
          type="number"
          class="input w-24 text-right font-mono"
          :min="256"
          :max="max()"
          :value="row.value"
          :disabled="disabled || model.auto"
          :aria-label="`${row.label}, ${t.common.mb}`"
          @blur="commit($event, row.set, row.value)"
          @keydown.enter="($event.target as HTMLInputElement).blur()"
        />
        <span class="text-[12px] text-faint">{{ t.common.mb }}</span>
      </span>
    </div>
    <p v-if="totalRamMib" class="text-[12px] text-faint">{{ t.settings.ramHint(totalRamMib) }}</p>
  </div>
</template>
