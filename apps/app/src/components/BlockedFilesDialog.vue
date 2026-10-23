<script setup lang="ts">
import { ExternalLink, ShieldAlert } from '@lucide/vue'
import type { BlockedFile } from '../api/types'
import { t } from '../i18n/ru'
import { useTasksStore } from '../stores/tasks'
import { openExternal } from '../utils/external'
import ModalDialog from './ModalDialog.vue'

defineProps<{ files: BlockedFile[] }>()
const emit = defineEmits<{ close: [] }>()
const tasks = useTasksStore()
</script>

<template>
  <ModalDialog :title="t.browse.blockedTitle" :width="620" @close="emit('close')">
    <p class="mb-3 flex items-start gap-2 text-warn">
      <ShieldAlert :size="16" class="mt-px shrink-0" />
      <span>{{ t.browse.blocked }}. {{ t.browse.blockedHint }}</span>
    </p>
    <ul class="flex flex-col gap-1.5">
      <li v-for="f in files" :key="f.targetPath" class="card flex items-center gap-3 px-3 py-2">
        <div class="min-w-0 flex-1">
          <div class="truncate font-semibold">{{ f.name }}</div>
          <div class="truncate font-mono text-[12px] text-faint">{{ f.targetPath }}</div>
        </div>
        <button v-if="f.downloadPage" class="btn h-7" @click="tasks.guard(() => openExternal(f.downloadPage!))">
          <ExternalLink :size="14" />{{ t.browse.blockedLink }}
        </button>
      </li>
    </ul>
    <template #footer>
      <button class="btn btn-primary" @click="emit('close')">{{ t.common.close }}</button>
    </template>
  </ModalDialog>
</template>
