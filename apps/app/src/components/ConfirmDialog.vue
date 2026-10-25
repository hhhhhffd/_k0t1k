<script setup lang="ts">
import { t } from '../i18n'
import ModalDialog from './ModalDialog.vue'

defineProps<{ title: string; text: string; confirmLabel?: string }>()
const emit = defineEmits<{ confirm: []; close: [] }>()
</script>

<template>
  <ModalDialog :title="title" :width="420" @close="emit('close')">
    <p class="text-muted">{{ text }}</p>
    <template #footer>
      <button class="btn" @click="emit('close')">{{ t.common.cancel }}</button>
      <button class="btn btn-danger" @click="emit('confirm')">{{ confirmLabel ?? t.common.delete }}</button>
    </template>
  </ModalDialog>
</template>
