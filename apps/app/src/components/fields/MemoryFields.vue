<script setup lang="ts">
import type { MemorySettings } from '../../api/types'
import { t } from '../../i18n/ru'

const props = defineProps<{ disabled?: boolean; totalRamMb?: number }>()
const model = defineModel<MemorySettings>({ required: true })
const max = () => Math.max(props.totalRamMb ?? 16384, 1024)

function setMin(v: number) {
  model.value = { minMb: v, maxMb: Math.max(v, model.value.maxMb) }
}
function setMax(v: number) {
  model.value = { minMb: Math.min(v, model.value.minMb), maxMb: v }
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <div v-for="row in [
      { label: t.settings.minMem, value: model.minMb, set: setMin },
      { label: t.settings.maxMem, value: model.maxMb, set: setMax },
    ]" :key="row.label" class="grid grid-cols-[90px_1fr_96px] items-center gap-3">
      <span class="text-muted">{{ row.label }}</span>
      <input type="range" :min="256" :max="max()" :step="256" :value="row.value" :disabled="disabled" :aria-label="row.label" @input="row.set(Number(($event.target as HTMLInputElement).value))" />
      <span class="text-right font-mono text-[12.5px]">{{ row.value }} {{ t.common.mb }}</span>
    </div>
    <p v-if="totalRamMb" class="text-[12px] text-faint">{{ t.settings.ramHint(totalRamMb) }}</p>
  </div>
</template>
