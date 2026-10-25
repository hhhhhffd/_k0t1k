<script setup lang="ts">
import { t } from '../i18n'
import ModalDialog from './ModalDialog.vue'

defineProps<{ title: string; text: string }>()
const emit = defineEmits<{ accept: []; close: [] }>()
</script>

<template>
  <ModalDialog :title="title" :width="680" @close="emit('close')">
    <div class="flex flex-col gap-2 p-4">
      <p class="text-muted">{{ t.client.termsIntro }}</p>
      <pre class="max-h-[45vh] overflow-y-auto rounded-lg border border-line bg-base p-3 text-[12px] leading-relaxed break-words whitespace-pre-wrap">{{ text }}</pre>
    </div>
    <template #footer>
      <button class="btn" @click="emit('close')">{{ t.client.decline }}</button>
      <button class="btn btn-primary" @click="emit('accept')">{{ t.client.accept }}</button>
    </template>
  </ModalDialog>
</template>
