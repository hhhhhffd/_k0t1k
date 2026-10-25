<script setup lang="ts">
import type { RelocationReport } from '../api/types'
import { t } from '../i18n'
import ModalDialog from './ModalDialog.vue'

defineProps<{ report: RelocationReport }>()
const emit = defineEmits<{ close: [] }>()
</script>

<template>
  <ModalDialog :title="t.onboarding.reportTitle" :width="560" @close="emit('close')">
    <div class="flex flex-col gap-3">
      <p class="text-muted">{{ t.onboarding.reportIntro }}</p>
      <ul class="flex list-disc flex-col gap-1.5 pl-5">
        <li v-for="problem in report.problems" :key="problem" class="selectable font-mono text-[12px] break-all">{{ problem }}</li>
      </ul>
      <template v-if="report.moved.length">
        <p class="text-muted">{{ t.onboarding.movedIntro }}</p>
        <p class="selectable font-mono text-[12px] break-all">{{ report.moved.join(', ') }}</p>
      </template>
      <p v-if="report.installedTo" class="selectable font-mono text-[12px] break-all text-faint">{{ report.installedTo }}</p>
    </div>
    <template #footer>
      <button class="btn btn-primary" @click="emit('close')">{{ t.common.close }}</button>
    </template>
  </ModalDialog>
</template>
