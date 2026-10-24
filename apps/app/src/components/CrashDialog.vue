<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import * as api from '../api/commands'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { openExternal } from '../utils/external'
import { uploadSessionLog } from '../utils/uploadLog'
import ModalDialog from './ModalDialog.vue'

const store = useInstancesStore()
const tasks = useTasksStore()
const router = useRouter()

// crashes are shown one after another
const crash = computed(() => store.crashes[0])
const instanceName = computed(() => (crash.value ? (store.byId(crash.value.instanceId)?.name ?? crash.value.instanceId) : ''))
const uploading = ref(false)
const link = ref<string | undefined>()
watch(crash, () => (link.value = undefined))

async function upload() {
  if (!crash.value) return
  uploading.value = true
  link.value = await uploadSessionLog(crash.value.sessionId)
  uploading.value = false
}

function openReport() {
  if (crash.value) void tasks.guard(() => api.openCrashReport(crash.value!.sessionId))
}

function close() {
  if (crash.value) store.dismissCrash(crash.value.sessionId)
}

function showLogs() {
  if (!crash.value) return
  const instanceId = crash.value.instanceId
  close()
  void router.push({ name: 'instance', params: { id: instanceId }, query: { tab: 'logs' } })
}
</script>

<template>
  <ModalDialog v-if="crash" :title="t.crash.title" :width="480" @close="close">
    <div class="flex flex-col gap-2">
      <p class="font-semibold">{{ instanceName }}</p>
      <p class="text-muted">{{ t.crash.exitCode(crash.exitCode) }} <template v-if="crash.crashReport">{{ t.crash.hasReport }}</template></p>
      <p v-if="crash.crashReport" class="selectable break-all font-mono text-[12px] text-faint">{{ crash.crashReport }}</p>
      <p v-if="link" class="text-[13px]">
        {{ t.instance.uploadedLink }}:
        <a :href="link" class="selectable text-accent underline" @click.prevent="openExternal(link)">{{ link }}</a>
      </p>
    </div>
    <template #footer>
      <button v-if="crash.crashReport" class="btn" @click="openReport">{{ t.crash.openReport }}</button>
      <button class="btn" :disabled="uploading" @click="upload">{{ uploading ? t.instance.uploading : t.instance.uploadLog }}</button>
      <button class="btn" @click="showLogs">{{ t.crash.logs }}</button>
      <button class="btn btn-primary" @click="close">{{ t.common.close }}</button>
    </template>
  </ModalDialog>
</template>
