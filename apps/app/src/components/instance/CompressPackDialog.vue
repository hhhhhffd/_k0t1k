<script setup lang="ts">
import { save } from '@tauri-apps/plugin-dialog'
import { onBeforeUnmount, onMounted, ref } from 'vue'
import * as api from '../../api/commands'
import type { CompressedPackInfo, LocalResource, LocalResourceKind } from '../../api/types'
import { t } from '../../i18n'
import { useTasksStore } from '../../stores/tasks'
import { formatBytes, plainText } from '../../utils/format'
import ModalDialog from '../ModalDialog.vue'

const props = defineProps<{ instanceId: string; kind: LocalResourceKind; resource: LocalResource }>()
const emit = defineEmits<{ close: []; replaced: [zipName: string] }>()
const tasks = useTasksStore()

/** `null` while the folder is being zipped. */
const info = ref<CompressedPackInfo | null>(null)
const busy = ref(false)
let unmounted = false

const discard = () => void api.discardCompressedPack().catch((e: unknown) => console.warn('cannot drop the compressed pack', e))

onMounted(async () => {
  info.value = (await tasks.guard(() => api.compressPack(props.instanceId, props.kind, props.resource.fileName))) ?? null
  if (unmounted) discard() // closed while the folder was being zipped
  else if (!info.value) emit('close')
})
// The bytes stay on the Rust side until the dialog is closed, however it is closed.
onBeforeUnmount(() => {
  unmounted = true
  discard()
})

async function replace() {
  if (!info.value || busy.value) return
  busy.value = true
  const zipName = await tasks.guard(() => api.replacePackWithZip(props.instanceId, props.kind, props.resource.fileName, info.value!.token))
  busy.value = false
  if (zipName) emit('replaced', zipName)
}

async function saveToComputer() {
  if (!info.value || busy.value) return
  const name = `${props.resource.fileName}.zip`
  const path = api.isTauri ? await save({ defaultPath: name, filters: [{ name: 'ZIP', extensions: ['zip'] }] }) : `C:\\Downloads\\${name}`
  if (!path) return
  busy.value = true
  const saved = await tasks.guard(() => api.saveCompressedPack(info.value!.token, path).then(() => true))
  busy.value = false
  if (saved) tasks.toast(t.compress.savedTo(path), 'ok')
}
</script>

<template>
  <ModalDialog :title="t.compress.title" size="sm" @close="emit('close')">
    <p v-if="!info" class="py-4 text-center text-faint">{{ t.packs.working }}</p>
    <div v-else class="flex flex-col gap-2">
      <p class="truncate font-semibold">{{ plainText(resource.fileName) }}</p>
      <p>{{ t.compress.sizes(formatBytes(info.folderBytes), formatBytes(info.zipBytes)) }}</p>
      <p class="text-muted">{{ t.compress.files(info.fileCount) }}</p>
      <p class="text-[12px] text-faint">{{ t.compress.replaceHint }}</p>
    </div>
    <template #footer>
      <button class="btn" @click="emit('close')">{{ t.common.cancel }}</button>
      <button class="btn btn-primary" :disabled="!info || busy" @click="saveToComputer">{{ t.compress.save }}</button>
      <button class="btn btn-primary" :disabled="!info || busy" @click="replace">{{ t.compress.replace }}</button>
    </template>
  </ModalDialog>
</template>
