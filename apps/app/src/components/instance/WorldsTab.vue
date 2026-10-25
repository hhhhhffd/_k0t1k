<script setup lang="ts">
import { Archive, Copy, FolderOpen, Globe, Pencil, Trash2, Upload } from '@lucide/vue'
import { open, save } from '@tauri-apps/plugin-dialog'
import { onMounted, ref, watch } from 'vue'
import * as api from '../../api/commands'
import { isTauri } from '../../api/commands'
import type { World } from '../../api/types'
import { t } from '../../i18n'
import { useTasksStore } from '../../stores/tasks'
import { formatBytes, formatRelative } from '../../utils/format'
import ConfirmDialog from '../ConfirmDialog.vue'
import ModalDialog from '../ModalDialog.vue'
import PromptDialog from '../PromptDialog.vue'
import ToggleSwitch from '../ToggleSwitch.vue'
import WorldDetail from './WorldDetail.vue'

const props = defineProps<{ instanceId: string }>()
const tasks = useTasksStore()

const worlds = ref<World[]>([])
const loading = ref(true)
/** Folder name of the world whose details are open. */
const opened = ref<string | null>(null)
const renaming = ref<World | null>(null)
const renameValue = ref('')
const renameFolder = ref(false)
const copying = ref<World | null>(null)
const backingUp = ref<World | null>(null)
const deleting = ref<World | null>(null)

async function load() {
  loading.value = true
  worlds.value = (await tasks.guard(() => api.listWorlds(props.instanceId))) ?? []
  loading.value = false
}
onMounted(load)
watch(() => props.instanceId, load)

/** Runs a modifying call and reloads the list when it succeeded. */
async function change<T>(action: () => Promise<T>): Promise<T | undefined> {
  const result = await tasks.guard(action)
  if (result !== undefined) await load()
  return result
}

async function importZip() {
  const path = isTauri
    ? await open({ multiple: false, filters: [{ name: t.worlds.importFilter, extensions: ['zip'] }] })
    : 'C:\\Downloads\\Imported World.zip'
  if (typeof path !== 'string') return
  const dirName = await change(() => api.importWorld(props.instanceId, path))
  if (dirName) tasks.toast(t.worlds.imported(dirName), 'ok')
}

const openFolder = (world: World) => tasks.guard(() => api.openWorldFolder(props.instanceId, world.dirName))

function startRename(world: World) {
  renaming.value = world
  renameValue.value = world.name
  renameFolder.value = false
}
async function confirmRename() {
  const world = renaming.value
  const name = renameValue.value.trim()
  renaming.value = null
  if (!world || !name) return
  const dirName = await change(() => api.renameWorld(props.instanceId, world.dirName, name, renameFolder.value))
  if (dirName) {
    if (opened.value === world.dirName) opened.value = dirName
    tasks.toast(t.worlds.renamed, 'ok')
  }
}

async function confirmCopy(name: string) {
  const world = copying.value
  copying.value = null
  if (!world) return
  const dirName = await change(() => api.copyWorld(props.instanceId, world.dirName, name))
  if (dirName) tasks.toast(t.worlds.copied(dirName), 'ok')
}

async function backup(choose: boolean) {
  const world = backingUp.value
  backingUp.value = null
  if (!world) return
  let destination: string | undefined
  if (choose) {
    const picked = isTauri
      ? await save({ defaultPath: `${world.dirName}.zip`, filters: [{ name: t.worlds.backupFilter, extensions: ['zip'] }] })
      : `C:\\Backups\\${world.dirName}.zip`
    if (!picked) return
    destination = picked
  }
  const path = await tasks.guard(() => api.backupWorld(props.instanceId, world.dirName, destination))
  if (path) tasks.toast(t.worlds.backedUp(path), 'ok')
}

async function confirmDelete() {
  const world = deleting.value
  deleting.value = null
  if (!world) return
  const done = await change(async () => {
    await api.deleteWorld(props.instanceId, world.dirName)
    return true
  })
  if (done && opened.value === world.dirName) opened.value = null
}
</script>

<template>
  <WorldDetail
    v-if="opened && worlds.find((w) => w.dirName === opened)"
    :instance-id="instanceId"
    :world="worlds.find((w) => w.dirName === opened)!"
    @back="opened = null"
  />
  <div v-else class="flex flex-col gap-3">
    <div class="flex items-center gap-2">
      <span class="mr-auto text-faint">{{ worlds.length }}</span>
      <button class="btn" @click="importZip"><Upload :size="15" />{{ t.worlds.import }}</button>
    </div>

    <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
    <p v-else-if="!worlds.length" class="py-10 text-center text-faint">{{ t.worlds.empty }}</p>
    <ul v-else class="flex flex-col gap-1.5">
      <li v-for="w in worlds" :key="w.dirName" class="card flex items-center gap-3 px-3 py-2.5">
        <button class="flex min-w-0 flex-1 items-center gap-3 text-left" @click="opened = w.dirName">
          <img v-if="w.icon" :src="w.icon" alt="" class="pixelated size-10 shrink-0 rounded-md" />
          <span v-else class="pix flex size-10 shrink-0 items-center justify-center bg-ok/15 text-ok"><Globe :size="18" /></span>
          <span class="min-w-0 flex-1">
            <span class="flex items-center gap-2">
              <span class="truncate font-semibold">{{ w.name }}</span>
              <span v-if="w.gameMode" class="badge">{{ t.worlds.gameModes[w.gameMode] }}</span>
              <span v-if="w.hardcore" class="badge border-danger/50 bg-danger/15 text-danger">{{ t.worlds.hardcore }}</span>
              <span v-if="w.versionName" class="badge">{{ w.versionName }}</span>
              <span v-if="w.gameMode === null" class="badge text-warn">{{ t.worlds.unreadable }}</span>
            </span>
            <span class="block truncate text-[12px] text-faint">{{ w.dirName }} · {{ formatBytes(w.size) }} · {{ formatRelative(w.lastPlayed) }}</span>
          </span>
        </button>
        <button class="btn btn-ghost btn-icon text-muted" :title="t.worlds.openFolder" :aria-label="t.worlds.openFolder" @click="openFolder(w)"><FolderOpen :size="15" /></button>
        <button class="btn btn-ghost btn-icon text-muted" :title="t.worlds.backup" :aria-label="t.worlds.backup" @click="backingUp = w"><Archive :size="15" /></button>
        <button class="btn btn-ghost btn-icon text-muted" :title="t.worlds.copy" :aria-label="t.worlds.copy" @click="copying = w"><Copy :size="15" /></button>
        <button class="btn btn-ghost btn-icon text-muted" :title="t.worlds.rename" :aria-label="t.worlds.rename" @click="startRename(w)"><Pencil :size="15" /></button>
        <button class="btn btn-ghost btn-icon text-muted hover:!text-danger" :title="t.worlds.delete" :aria-label="t.worlds.delete" @click="deleting = w"><Trash2 :size="15" /></button>
      </li>
    </ul>

    <ModalDialog v-if="renaming" :title="t.worlds.renameTitle" :width="420" @close="renaming = null">
      <div class="flex flex-col gap-3">
        <label class="flex flex-col gap-1">
          <span class="label">{{ t.worlds.name }}</span>
          <input v-model="renameValue" class="input" autofocus @keydown.enter="confirmRename" />
        </label>
        <label class="flex items-center gap-2 text-muted">
          <ToggleSwitch v-model="renameFolder" :label="t.worlds.renameFolder" />
          {{ t.worlds.renameFolder }}
        </label>
      </div>
      <template #footer>
        <button class="btn" @click="renaming = null">{{ t.common.cancel }}</button>
        <button class="btn btn-primary" :disabled="!renameValue.trim()" @click="confirmRename">{{ t.common.save }}</button>
      </template>
    </ModalDialog>

    <PromptDialog
      v-if="copying"
      :title="t.worlds.copyTitle"
      :label="t.worlds.name"
      :initial="`${copying.name} (${t.worlds.copySuffix})`"
      @submit="confirmCopy"
      @close="copying = null"
    />

    <ModalDialog v-if="backingUp" :title="t.worlds.backupTitle" :width="460" @close="backingUp = null">
      <p class="text-muted">{{ t.worlds.backupHint }}</p>
      <template #footer>
        <button class="btn" @click="backup(true)">{{ t.worlds.backupChoose }}</button>
        <button class="btn btn-primary" @click="backup(false)">{{ t.worlds.backupDefault }}</button>
      </template>
    </ModalDialog>

    <ConfirmDialog
      v-if="deleting"
      :title="t.worlds.deleteTitle"
      :text="t.worlds.deleteText(deleting.name)"
      @confirm="confirmDelete"
      @close="deleting = null"
    />
  </div>
</template>
