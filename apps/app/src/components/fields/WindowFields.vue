<script setup lang="ts">
import type { WindowSettings } from '../../api/types'
import { t } from '../../i18n/ru'
import ToggleSwitch from '../ToggleSwitch.vue'

defineProps<{ disabled?: boolean }>()
const model = defineModel<WindowSettings>({ required: true })
</script>

<template>
  <div class="flex flex-wrap items-end gap-3">
    <label class="flex w-28 flex-col gap-1">
      <span class="label">{{ t.settings.width }}</span>
      <input v-model.number="model.width" type="number" min="320" class="input" :disabled="disabled || model.maximized" />
    </label>
    <label class="flex w-28 flex-col gap-1">
      <span class="label">{{ t.settings.height }}</span>
      <input v-model.number="model.height" type="number" min="240" class="input" :disabled="disabled || model.maximized" />
    </label>
    <label class="flex h-8 items-center gap-2">
      <ToggleSwitch v-model="model.maximized" :disabled="disabled" :label="t.settings.maximized" />
      <span>{{ t.settings.maximized }}</span>
    </label>
  </div>
</template>
