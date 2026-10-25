<script setup lang="ts">
import { t } from '../i18n'
import { useRatscanStore } from '../stores/ratscan'
import ModalDialog from './ModalDialog.vue'

const ratscan = useRatscanStore()
</script>

<template>
  <ModalDialog v-if="ratscan.launchPrompt" :title="t.scan.launchTitle" :width="480" @close="ratscan.launchPrompt.answer('cancel')">
    <div class="flex flex-col gap-2">
      <p class="text-muted">{{ t.scan.launchText }}</p>
      <ul class="selectable list-disc pl-5 font-mono text-[12px] text-danger">
        <li v-for="mod in ratscan.launchPrompt.mods" :key="mod.fileName">{{ mod.fileName }}</li>
      </ul>
    </div>
    <template #footer>
      <button class="btn" @click="ratscan.launchPrompt.answer('cancel')">{{ t.common.cancel }}</button>
      <button class="btn" @click="ratscan.launchPrompt.answer('launch')">{{ t.scan.launchAnyway }}</button>
      <button class="btn btn-primary" @click="ratscan.launchPrompt.answer('disable')">{{ t.scan.disableAndLaunch }}</button>
    </template>
  </ModalDialog>
</template>
