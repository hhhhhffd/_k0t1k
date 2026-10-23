<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { Check, ExternalLink, FolderSearch, Loader, ShieldAlert } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { getDownloadsDir, isTauri, scanBlockedFiles } from '../api/commands'
import type { BlockedFile } from '../api/types'
import { t } from '../i18n/ru'
import { useTasksStore } from '../stores/tasks'
import { openExternal } from '../utils/external'
import ModalDialog from './ModalDialog.vue'

const POLL_MS = 2000

const props = defineProps<{ files: BlockedFile[]; instanceId: string }>()
const emit = defineEmits<{ close: [] }>()
const tasks = useTasksStore()

const watching = ref(false)
/** The folder the user chose; `null` is the Downloads folder. */
const folder = ref<string | null>(null)
const downloadsDir = ref('')
const found = ref(new Set<string>())
let timer: ReturnType<typeof setInterval> | undefined
let scanning = false

const shownFolder = computed(() => folder.value ?? downloadsDir.value)
const missingCount = computed(() => props.files.filter((f) => !found.value.has(f.targetPath)).length)

onMounted(async () => {
  downloadsDir.value = (await tasks.guard(getDownloadsDir)) ?? ''
})

function stop() {
  watching.value = false
  clearInterval(timer)
  timer = undefined
}

async function scan() {
  if (scanning) return
  scanning = true
  const result = await tasks.guard(() => scanBlockedFiles(props.instanceId, props.files, folder.value))
  scanning = false
  if (!watching.value) return
  if (!result) {
    stop()
    return
  }
  found.value = new Set(result.found)
  if (!result.missing.length) stop()
}

/** Looks now and then again every couple of seconds while the dialog is open, so a download that finishes is picked up. */
function start() {
  stop()
  watching.value = true
  void scan()
  timer = setInterval(() => void scan(), POLL_MS)
}

async function chooseFolder() {
  const chosen = isTauri ? await open({ directory: true, multiple: false }) : 'C:\\Users\\Player\\Desktop'
  if (typeof chosen !== 'string') return
  folder.value = chosen
  start()
}

onBeforeUnmount(stop)
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
        <span v-if="found.has(f.targetPath)" class="badge badge-accent"><Check :size="12" />{{ t.blockedScan.placed }}</span>
        <button v-else-if="f.downloadPage" class="btn h-7" @click="tasks.guard(() => openExternal(f.downloadPage!))">
          <ExternalLink :size="14" />{{ t.browse.blockedLink }}
        </button>
      </li>
    </ul>
    <div v-if="watching || found.size" class="mt-3 flex flex-col gap-1 text-[12px] text-muted">
      <span class="flex items-center gap-1.5">
        <Loader v-if="watching" :size="13" class="animate-spin" />
        {{ missingCount ? t.blockedScan.found(files.length - missingCount, files.length) : t.blockedScan.allFound }}
        <template v-if="watching && missingCount"> · {{ t.blockedScan.watching }}</template>
      </span>
      <span class="truncate text-faint">{{ t.blockedScan.folder }}: {{ shownFolder }}</span>
    </div>
    <template #footer>
      <button class="btn" @click="chooseFolder"><FolderSearch :size="15" />{{ t.blockedScan.chooseFolder }}</button>
      <button v-if="missingCount && !watching" class="btn btn-primary" @click="start">
        <FolderSearch :size="15" />{{ t.blockedScan.downloaded }}
      </button>
      <button class="btn" @click="emit('close')">{{ t.common.close }}</button>
    </template>
  </ModalDialog>
</template>
