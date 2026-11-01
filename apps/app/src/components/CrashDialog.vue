<script setup lang="ts">
import { ExternalLink, Upload } from '@lucide/vue'
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import * as api from '../api/commands'
import type { GameCrashed, SessionId } from '../api/types'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { openExternal } from '../utils/external'
import { uploadSessionLog } from '../utils/uploadLog'
import ModalDialog from './ModalDialog.vue'

/** One dialog for every crash not yet dismissed: several copies of the game crashing at once make one list, not a queue of windows. */
const store = useInstancesStore()
const tasks = useTasksStore()
const router = useRouter()

const uploading = ref(new Set<SessionId>())
const links = ref(new Map<SessionId, string>())

const nameOf = (crash: GameCrashed) => store.byId(crash.instanceId)?.name ?? crash.instanceId
const numberOf = (crash: GameCrashed) => store.sessionsOf(crash.instanceId).find((s) => s.sessionId === crash.sessionId)?.number

async function upload(crash: GameCrashed) {
  uploading.value = new Set(uploading.value).add(crash.sessionId)
  const link = await uploadSessionLog(crash.sessionId)
  uploading.value = new Set([...uploading.value].filter((id) => id !== crash.sessionId))
  if (link) links.value = new Map(links.value).set(crash.sessionId, link)
}

function openReport(crash: GameCrashed) {
  void tasks.guard(() => api.openCrashReport(crash.sessionId))
}

function close() {
  for (const crash of store.crashes) store.dismissCrash(crash.sessionId)
}

function showLogs(crash: GameCrashed) {
  const instanceId = crash.instanceId
  close()
  void router.push({ name: 'instance', params: { id: instanceId }, query: { tab: 'logs' } })
}
</script>

<template>
  <ModalDialog v-if="store.crashes.length" :title="t.crash.title" size="md" @close="close">
    <ul class="flex flex-col gap-2">
      <li v-for="crash in store.crashes" :key="crash.sessionId" class="flex flex-col gap-1.5 rounded-lg border border-line bg-raised px-3 py-2.5">
        <div class="flex items-center gap-2">
          <span class="min-w-0 flex-1 truncate font-semibold">{{ nameOf(crash) }}<template v-if="numberOf(crash) !== undefined"> · {{ t.instance.session(numberOf(crash)!) }}</template></span>
          <span class="text-[12px] text-muted">{{ t.crash.exitCode(crash.exitCode) }}</span>
        </div>
        <p v-if="crash.crashReport" class="selectable break-all font-mono text-[12px] text-faint">{{ crash.crashReport }}</p>
        <p v-else class="text-[12px] text-faint">{{ t.crash.noReport }}</p>
        <a
          v-if="links.get(crash.sessionId)"
          :href="links.get(crash.sessionId)"
          class="selectable text-[12px] text-accent underline"
          :title="t.instance.uploadedLink"
          @click.prevent="openExternal(links.get(crash.sessionId)!)"
        >{{ links.get(crash.sessionId) }}</a>
        <div class="flex flex-wrap gap-1.5">
          <button v-if="crash.crashReport" class="btn h-7 text-[12px]" @click="openReport(crash)"><ExternalLink :size="13" />{{ t.crash.openReport }}</button>
          <button class="btn h-7 text-[12px]" :disabled="uploading.has(crash.sessionId)" @click="upload(crash)">
            <Upload :size="13" />{{ uploading.has(crash.sessionId) ? t.instance.uploading : t.instance.uploadLog }}
          </button>
          <button class="btn h-7 text-[12px]" @click="showLogs(crash)">{{ t.crash.logs }}</button>
        </div>
      </li>
    </ul>
    <template #footer>
      <button class="btn btn-primary" @click="close">{{ t.common.close }}</button>
    </template>
  </ModalDialog>
</template>
