<script setup lang="ts">
import type { CommandSettings } from '../../api/types'
import { t } from '../../i18n/ru'

defineProps<{ disabled?: boolean }>()
const model = defineModel<CommandSettings>({ required: true })
</script>

<template>
  <div class="flex flex-col gap-3">
    <label class="flex flex-col gap-1">
      <span class="label">{{ t.settings.preLaunch }}</span>
      <input v-model="model.preLaunch" class="input font-mono" :disabled="disabled" />
    </label>
    <label class="flex flex-col gap-1">
      <span class="label">{{ t.settings.postExit }}</span>
      <input v-model="model.postExit" class="input font-mono" :disabled="disabled" />
    </label>
    <label class="flex flex-col gap-1">
      <span class="label">{{ t.settings.wrapper }}</span>
      <input v-model="model.wrapper" class="input font-mono" :disabled="disabled" placeholder="prime-run" />
    </label>
  </div>
</template>
