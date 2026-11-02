<script setup lang="ts">
import { Download, ExternalLink } from '@lucide/vue'
import { computed } from 'vue'
import { t } from '../i18n'
import { useTasksStore } from '../stores/tasks'
import { useUpdateStore } from '../stores/update'
import { openExternal } from '../utils/external'
import ModalDialog from './ModalDialog.vue'

const update = useUpdateStore()
const tasks = useTasksStore()

const downloadPercent = computed(() => {
  const task = tasks.tasks.find((x) => x.task === 'update' && !x.finished && !x.error)
  return task?.total ? Math.min(100, Math.round((task.done / task.total) * 100)) : null
})
// same reasons as in the settings: a running game, or an install that is impossible from here
const blockedReason = computed(() => (update.gameRunning ? t.update.afterGame : (update.status?.installBlocked ?? '')))
</script>

<template>
  <ModalDialog v-if="update.release" :title="t.update.dialogTitle" size="sm" @close="update.prompting = false">
    <p class="mb-3">
      {{ t.update.dialogVersions(update.status?.currentVersion ?? '', update.release.version) }}
    </p>
    <button class="btn mb-3" @click="openExternal(update.release.notesUrl)"><ExternalLink :size="15" />{{ t.update.notes }}</button>
    <div
      v-if="update.installing"
      class="h-1.5 overflow-hidden rounded-full bg-raised"
      role="progressbar"
      :aria-label="t.update.installing"
      :aria-valuenow="downloadPercent ?? undefined"
      aria-valuemax="100"
    >
      <div class="h-full rounded-full bg-accent transition-[width]" :style="{ width: `${downloadPercent ?? 100}%` }" />
    </div>
    <p v-else-if="blockedReason" class="text-[12px] text-warn">{{ blockedReason }}</p>
    <p v-if="update.error" class="selectable mt-2 rounded-lg border border-danger/50 bg-danger/10 px-3 py-2 text-[12px] break-words text-danger" role="alert">{{ update.error }}</p>
    <template #footer>
      <button class="btn" :disabled="update.installing" @click="update.skip()">{{ t.update.skipVersion }}</button>
      <button class="btn" @click="update.prompting = false">{{ t.update.later }}</button>
      <button class="btn btn-primary" :disabled="update.installing || !!blockedReason" @click="update.install()">
        <Download :size="15" />{{ update.installing ? t.update.installing : t.update.updateNow }}
      </button>
    </template>
  </ModalDialog>
</template>
