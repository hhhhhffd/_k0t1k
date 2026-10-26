<script setup lang="ts">
import { t } from '../i18n'
import { useRatscanStore } from '../stores/ratscan'
import ModalDialog from './ModalDialog.vue'

const ratscan = useRatscanStore()

function closeUnscanned() {
  // the scan before launch cannot be interrupted halfway
  if (!ratscan.unscannedPrompt?.scanning) ratscan.unscannedPrompt?.answer('cancel')
}
</script>

<template>
  <ModalDialog v-if="ratscan.unscannedPrompt" :title="t.scan.unscannedTitle" :width="480" @close="closeUnscanned">
    <div class="flex flex-col gap-2">
      <p class="text-muted">{{ t.scan.unscannedText }}</p>
      <ul class="selectable max-h-40 list-disc overflow-y-auto pl-5 font-mono text-[12px]">
        <li v-for="mod in ratscan.unscannedPrompt.mods" :key="mod.sha1">{{ mod.fileName }}</li>
      </ul>
      <p class="rounded-lg border border-warn/50 bg-warn/10 px-3 py-2 text-[12px] text-warn">{{ t.scan.disclaimer }}</p>
      <p class="text-[12px] text-faint" role="status">
        {{ ratscan.unscannedPrompt.scanning && ratscan.progress ? t.scan.scanningBeforeLaunch(ratscan.progress.done, ratscan.progress.total) : t.scan.skipHint }}
      </p>
    </div>
    <template #footer>
      <button class="btn" :disabled="ratscan.unscannedPrompt.scanning" @click="ratscan.unscannedPrompt.answer('cancel')">{{ t.common.cancel }}</button>
      <button class="btn" :disabled="ratscan.unscannedPrompt.scanning" @click="ratscan.unscannedPrompt.answer('skip')">{{ t.scan.skipScan }}</button>
      <button class="btn btn-primary" :disabled="ratscan.unscannedPrompt.scanning" @click="ratscan.unscannedPrompt.answer('scan')">{{ t.scan.scanAndLaunch }}</button>
    </template>
  </ModalDialog>
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
