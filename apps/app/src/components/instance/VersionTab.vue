<script setup lang="ts">
import { ArrowUpCircle, Box, FolderOpen, Pencil, Puzzle, Trash2, Wrench } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { listLoaderVersions, openInstanceFolder } from '../../api/commands'
import { COMPONENT_MINECRAFT, LOADER_UIDS, type Instance, type LoaderId, type LoaderVersion } from '../../api/types'
import { t } from '../../i18n/ru'
import { useInstancesStore } from '../../stores/instances'
import { useTasksStore } from '../../stores/tasks'
import { LOADER_LABELS, loaderOf, minecraftVersionOf } from '../../utils/format'
import ConfirmDialog from '../ConfirmDialog.vue'
import LoaderPicker from '../LoaderPicker.vue'
import McVersionList from '../McVersionList.vue'
import ModalDialog from '../ModalDialog.vue'

const props = defineProps<{ instance: Instance }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const loader = computed(() => loaderOf(props.instance))
const running = computed(() => store.stateOf(props.instance.id) !== 'stopped')
const busy = ref(false)

const rows = computed(() =>
  props.instance.components.map((c) => {
    const loaderId = (Object.keys(LOADER_UIDS) as LoaderId[]).find((id) => LOADER_UIDS[id] === c.uid)
    const name = c.uid === COMPONENT_MINECRAFT ? 'Minecraft' : loaderId ? LOADER_LABELS[loaderId] : c.uid
    return { ...c, name, kind: c.uid === COMPONENT_MINECRAFT ? 'minecraft' : loaderId ? 'loader' : 'other' }
  }),
)

// The newest recommended build for the current Minecraft version, offered when it is newer than the installed one.
const newestLoader = ref<LoaderVersion | null>(null)
watch(
  () => [minecraftVersionOf(props.instance), loader.value?.id, loader.value?.version] as const,
  async ([mc, id, version]) => {
    newestLoader.value = null
    if (!id || !version) return
    const list = await tasks.guard(() => listLoaderVersions(mc, id))
    const newest = list?.[0]
    const installed = list?.find((v) => v.version === version)
    if (newest && installed && newest.version !== version && newest.releaseTime > installed.releaseTime) newestLoader.value = newest
  },
  { immediate: true },
)

const editing = ref(false)
const draftMc = ref('')
const draftLoader = ref<LoaderId | null>(null)
const draftLoaderVersion = ref('')
function openEditor() {
  draftMc.value = minecraftVersionOf(props.instance)
  draftLoader.value = loader.value?.id ?? null
  draftLoaderVersion.value = loader.value?.version ?? ''
  editing.value = true
}
const loaderChanges = computed(() => (loader.value?.id ?? null) !== draftLoader.value)
const canApply = computed(
  () =>
    draftMc.value !== '' &&
    (!draftLoader.value || draftLoaderVersion.value !== '') &&
    (draftMc.value !== minecraftVersionOf(props.instance) || loaderChanges.value || draftLoaderVersion.value !== (loader.value?.version ?? '')),
)

async function apply(mc: string, next: { id: LoaderId; version: string } | null) {
  busy.value = true
  const saved = await tasks.guard(() => store.changeVersions(props.instance.id, mc, next))
  busy.value = false
  if (saved) tasks.toast(t.version.changed, 'ok')
  return saved
}
async function applyEditor() {
  editing.value = false
  await apply(draftMc.value, draftLoader.value ? { id: draftLoader.value, version: draftLoaderVersion.value } : null)
}
function updateLoader() {
  if (loader.value && newestLoader.value) void apply(minecraftVersionOf(props.instance), { id: loader.value.id, version: newestLoader.value.version })
}
const confirmRemove = ref(false)
function removeLoader() {
  confirmRemove.value = false
  void apply(minecraftVersionOf(props.instance), null)
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <p v-if="running" class="rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-[12.5px] text-warn">{{ t.version.stopFirst }}</p>
    <p v-else-if="instance.managedPack" class="rounded-lg border border-line bg-raised px-3 py-2 text-[12.5px] text-muted">{{ t.version.managedNote }}</p>

    <div class="flex gap-3">
      <div class="card min-w-0 flex-1 overflow-hidden">
        <div class="grid grid-cols-[1fr_200px] border-b border-line px-3 py-2 text-[12px] font-semibold text-faint">
          <span>{{ t.version.component }}</span>
          <span>{{ t.version.version }}</span>
        </div>
        <button
          v-for="row in rows"
          :key="row.uid"
          class="grid w-full grid-cols-[1fr_200px] items-center px-3 py-2 text-left hover:bg-hover disabled:cursor-default"
          :class="row.dependencyOnly ? 'text-faint' : ''"
          :disabled="running || busy || row.kind === 'other'"
          :title="row.kind === 'other' ? '' : t.version.change"
          @dblclick="openEditor"
        >
          <span class="flex min-w-0 items-center gap-2.5">
            <Box v-if="row.kind === 'minecraft'" :size="16" class="shrink-0 text-ok" />
            <Puzzle v-else-if="row.kind === 'loader'" :size="16" class="shrink-0 text-accent" />
            <Wrench v-else :size="16" class="shrink-0 text-faint" />
            <span class="truncate font-medium">{{ row.name }}</span>
            <span v-if="row.dependencyOnly" class="badge">{{ t.version.dependency }}</span>
          </span>
          <span class="truncate font-mono text-[12.5px]">{{ row.version }}</span>
        </button>
      </div>

      <div class="flex w-56 shrink-0 flex-col gap-1">
        <button class="btn justify-start" :disabled="running || busy" @click="openEditor"><Pencil :size="15" class="text-accent" />{{ t.version.change }}</button>
        <button class="btn justify-start" :disabled="running || busy" @click="openEditor">
          <Puzzle :size="15" class="text-accent" />{{ loader ? t.version.replaceLoader : t.version.installLoader }}
        </button>
        <button v-if="loader && newestLoader" class="btn justify-start" :disabled="running || busy" @click="updateLoader">
          <ArrowUpCircle :size="15" class="text-ok" />
          <span class="truncate">{{ t.version.updateLoader(newestLoader.version) }}</span>
        </button>
        <button v-if="loader" class="btn justify-start" :disabled="running || busy" @click="confirmRemove = true">
          <Trash2 :size="15" class="text-danger" />{{ t.version.removeLoader }}
        </button>
        <div class="my-1 h-px bg-line" />
        <button class="btn justify-start" @click="tasks.guard(() => openInstanceFolder(instance.id))"><FolderOpen :size="15" class="text-warn" />{{ t.library.openFolder }}</button>
      </div>
    </div>

    <ModalDialog v-if="editing" :title="t.version.dialogTitle" large @close="editing = false">
      <div class="flex min-h-0 flex-1 flex-col gap-3 p-4">
        <div class="flex min-h-0 flex-[3] flex-col"><McVersionList v-model="draftMc" /></div>
        <div class="flex min-h-0 flex-[2] flex-col border-t border-line pt-3">
          <LoaderPicker v-model:loader="draftLoader" v-model:loader-version="draftLoaderVersion" :mc-version="draftMc" />
        </div>
        <p v-if="loaderChanges && loader" class="shrink-0 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-[12.5px] text-warn">{{ t.version.loaderWarning }}</p>
        <p class="shrink-0 text-[12px] text-faint">{{ t.version.dialogHint }}</p>
      </div>
      <template #footer>
        <button class="btn" @click="editing = false">{{ t.common.cancel }}</button>
        <button class="btn btn-primary" :disabled="!canApply" @click="applyEditor">{{ t.version.apply }}</button>
      </template>
    </ModalDialog>
    <ConfirmDialog
      v-if="confirmRemove && loader"
      :title="t.version.removeLoaderTitle"
      :text="t.version.removeLoaderText(loader.label)"
      :confirm-label="t.version.removeLoader"
      @confirm="removeLoader"
      @close="confirmRemove = false"
    />
  </div>
</template>
