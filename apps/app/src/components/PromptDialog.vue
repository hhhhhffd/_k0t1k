<script setup lang="ts">
import { ref } from 'vue'
import { t } from '../i18n/ru'
import ModalDialog from './ModalDialog.vue'

const props = defineProps<{ title: string; label: string; initial?: string; allowEmpty?: boolean; suggestions?: string[] }>()
const emit = defineEmits<{ submit: [value: string]; close: [] }>()
const value = ref(props.initial ?? '')

function submit() {
  const v = value.value.trim()
  if (v || props.allowEmpty) emit('submit', v)
}
</script>

<template>
  <ModalDialog :title="title" :width="420" @close="emit('close')">
    <label class="flex flex-col gap-1">
      <span class="label">{{ label }}</span>
      <input v-model="value" class="input" autofocus list="prompt-suggestions" @keydown.enter="submit" />
      <datalist id="prompt-suggestions"><option v-for="s in suggestions" :key="s" :value="s" /></datalist>
    </label>
    <template #footer>
      <button class="btn" @click="emit('close')">{{ t.common.cancel }}</button>
      <button class="btn btn-primary" :disabled="!allowEmpty && !value.trim()" @click="submit">{{ t.common.save }}</button>
    </template>
  </ModalDialog>
</template>
