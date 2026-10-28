<script setup lang="ts">
import { Archive, Copy, FolderOpen, Globe, ImageOff, Layers, Pencil, Play, RefreshCw, Sprout, Trash2, Upload } from '@lucide/vue'
import { open, save } from '@tauri-apps/plugin-dialog'
import { computed, onMounted, ref, watch } from 'vue'
import * as api from '../../api/commands'
import { isTauri } from '../../api/commands'
import type { World } from '../../api/types'
import { t } from '../../i18n'
import { useInstancesStore } from '../../stores/instances'
import { useSettingsStore } from '../../stores/settings'
import { useTasksStore } from '../../stores/tasks'
import { formatBytes, formatRelative } from '../../utils/format'
import { useMarquee } from '../../utils/useMarquee'
import { useSelection } from '../../utils/useSelection'
import { useViewMode } from '../../utils/useViewMode'
import ActionPanel from '../ActionPanel.vue'
import type { PanelAction } from '../ActionPanel.vue'
import ConfirmDialog from '../ConfirmDialog.vue'
import ContextMenu from '../ContextMenu.vue'
import type { MenuItem } from '../ContextMenu.vue'
import MarqueeBox from '../MarqueeBox.vue'
import ModalDialog from '../ModalDialog.vue'
import PromptDialog from '../PromptDialog.vue'
import ToggleSwitch from '../ToggleSwitch.vue'
import ViewToggle from '../ViewToggle.vue'
import WorldDetail from './WorldDetail.vue'

const props = defineProps<{ instanceId: string }>()
const tasks = useTasksStore()
const instances = useInstancesStore()
const settings = useSettingsStore()

const worlds = ref<World[]>([])
const loading = ref(true)
/** Folder name of the world whose details are open. */
const opened = ref<string | null>(null)
const renaming = ref<World | null>(null)
const renameValue = ref('')
const renameFolder = ref(false)
const copying = ref<World | null>(null)
const backingUp = ref<World | null>(null)
const deleting = ref<World[]>([])
const view = useViewMode('worlds')
const selected = ref(new Set<string>())
const selection = useSelection(selected)
const selectedWorlds = computed(() => worlds.value.filter((w) => selected.value.has(w.dirName)))
/** The world the single-world actions apply to: only when exactly one is selected. */
const current = computed(() => (selectedWorlds.value.length === 1 ? selectedWorlds.value[0]! : null))
const order = () => worlds.value.map((w) => w.dirName)
function onKey(e: KeyboardEvent) {
  if (selection.onKey(e, order())) return
  if (e.key === 'Delete' && selectedWorlds.value.length) deleting.value = selectedWorlds.value
}
const tabRoot = ref<HTMLElement | null>(null)
const { box } = useMarquee(selected, tabRoot, onKey)

async function load() {
  loading.value = true
  worlds.value = (await tasks.guard(() => api.listWorlds(props.instanceId))) ?? []
  selected.value = new Set(worlds.value.filter((w) => selected.value.has(w.dirName)).map((w) => w.dirName))
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
const openSaves = () => tasks.guard(() => api.openContentFolder(props.instanceId, 'world'))

const play = (world: World) =>
  void tasks.guard(() => instances.launch(props.instanceId, settings.settings?.selectedAccountId ?? undefined, undefined, world.dirName))

async function copySeed(world: World) {
  if (!world.seed) return
  try {
    await navigator.clipboard.writeText(world.seed)
    tasks.toast(t.worlds.seedCopied, 'ok')
  } catch {
    tasks.toast(world.seed)
  }
}

async function resetIcon(world: World) {
  const done = await change(async () => {
    await api.resetWorldIcon(props.instanceId, world.dirName)
    return true
  })
  if (done) tasks.toast(t.worlds.iconReset, 'ok')
}

function worldActions(world: World | null, doomed: World[]): PanelAction[] {
  const none = !world
  return [
    { label: t.worlds.play, icon: Play, disabled: none, title: t.worlds.playHint, action: () => world && play(world) },
    { label: t.worlds.rename, icon: Pencil, disabled: none, action: () => world && startRename(world) },
    { label: t.worlds.copy, icon: Copy, disabled: none, action: () => (copying.value = world) },
    { label: t.worlds.delete, icon: Trash2, danger: true, disabled: !doomed.length, action: () => (deleting.value = doomed) },
    { label: t.worlds.manageDatapacks, icon: Layers, separatorBefore: true, disabled: none, action: () => world && (opened.value = world.dirName) },
    { label: t.worlds.backup, icon: Archive, disabled: none, action: () => (backingUp.value = world) },
    { label: t.worlds.copySeed, icon: Sprout, disabled: !world?.seed, action: () => world && void copySeed(world) },
    { label: t.worlds.resetIcon, icon: ImageOff, disabled: !world?.icon, action: () => world && void resetIcon(world) },
    { label: t.worlds.openFolder, icon: FolderOpen, disabled: none, action: () => world && void openFolder(world) },
  ]
}
const panelActions = computed<PanelAction[]>(() => [
  { label: t.worlds.import, icon: Upload, primary: true, action: () => void importZip() },
  ...worldActions(current.value, selectedWorlds.value),
])
const footerActions = computed<PanelAction[]>(() => [
  { label: t.worlds.refresh, icon: RefreshCw, disabled: loading.value, action: () => void load() },
  { label: t.worlds.openSaves, icon: FolderOpen, action: () => void openSaves() },
])

const menu = ref<{ x: number; y: number; items: MenuItem[] } | null>(null)
function openMenu(e: MouseEvent, world: World) {
  if (!selected.value.has(world.dirName)) selected.value = new Set([world.dirName])
  // Only deleting makes sense for several worlds at once.
  const items = selectedWorlds.value.length > 1 ? worldActions(null, selectedWorlds.value).filter((a) => a.danger) : worldActions(world, [world])
  menu.value = { x: e.clientX, y: e.clientY, items }
}

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
  const doomed = deleting.value
  deleting.value = []
  if (!doomed.length) return
  await tasks.guard(async () => {
    for (const world of doomed) await api.deleteWorld(props.instanceId, world.dirName)
  })
  // Reload even after a failure: the worlds deleted before it must leave the list.
  await load()
  if (opened.value && !worlds.value.some((w) => w.dirName === opened.value)) opened.value = null
}
</script>

<template>
  <WorldDetail
    v-if="opened && worlds.find((w) => w.dirName === opened)"
    :instance-id="instanceId"
    :world="worlds.find((w) => w.dirName === opened)!"
    @back="opened = null"
  />
  <div v-else ref="tabRoot" class="flex flex-col gap-3">
    <div class="flex items-start gap-3">
      <div class="flex min-w-0 flex-1 flex-col gap-2">
        <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
        <p v-else-if="!worlds.length" class="py-10 text-center text-faint">{{ t.worlds.empty }}</p>
        <template v-else>
        <div class="flex items-center gap-2">
          <span class="mr-auto text-faint">{{ t.instance.counted(worlds.length) }}<template v-if="selected.size"> · {{ t.instance.selectedCount(selected.size) }}</template></span>
          <ViewToggle v-model="view" />
        </div>
        <div
          class="!outline-none"
          :class="view === 'list' ? 'overflow-hidden rounded-xl border border-line' : ''"
          @dragstart.prevent
        >
          <div v-if="view === 'list'" class="flex items-center gap-3 border-b border-line px-3 py-2 text-[12px] font-semibold text-faint">
            <span class="w-10 shrink-0" />
            <span class="min-w-0 flex-1">{{ t.worlds.name }} · {{ worlds.length }}</span>
            <span class="w-28 shrink-0">{{ t.worlds.mode }}</span>
            <span class="w-24 shrink-0">{{ t.worlds.version }}</span>
            <span class="w-28 shrink-0">{{ t.worlds.lastPlayed }}</span>
            <span class="w-20 shrink-0 text-right">{{ t.worlds.size }}</span>
          </div>
          <ul role="listbox" aria-multiselectable="true" :class="view === 'tiles' ? 'grid grid-cols-[repeat(auto-fill,minmax(230px,1fr))] gap-2.5' : ''">
            <li
              v-for="w in worlds"
              :key="w.dirName"
              role="option"
              :data-select-key="w.dirName"
              :aria-selected="selected.has(w.dirName)"
              class="flex cursor-default"
              :class="[
                view === 'list' ? 'items-center gap-3 border-b border-line/60 px-3 py-2 last:border-b-0' : 'flex-col gap-2 rounded-xl border border-line p-3',
                selected.has(w.dirName) ? (view === 'list' ? 'bg-accent/12' : 'border-accent bg-accent/12') : 'hover:bg-hover',
              ]"
              @click="selection.click(w.dirName, $event, order())"
              @dblclick="opened = w.dirName"
              @contextmenu.prevent="openMenu($event, w)"
            >
              <div v-if="view === 'tiles'" class="flex items-center gap-2.5">
                <img v-if="w.icon" :src="w.icon" alt="" class="pixelated size-12 shrink-0 rounded-md" />
                <span v-else class="pix flex size-12 shrink-0 items-center justify-center bg-ok/15 text-ok"><Globe :size="20" /></span>
                <span class="min-w-0">
                  <span class="block truncate font-semibold">{{ w.name }}</span>
                  <span class="block truncate text-[12px] text-faint">{{ w.dirName }}</span>
                </span>
              </div>
              <template v-if="view === 'tiles'">
                <div class="flex flex-wrap gap-1.5">
                  <span v-if="w.gameMode" class="badge">{{ t.worlds.gameModes[w.gameMode] }}</span>
                  <span v-if="w.hardcore" class="badge border-danger/50 bg-danger/15 text-danger">{{ t.worlds.hardcore }}</span>
                  <span v-if="w.gameMode === null" class="badge text-warn">{{ t.worlds.unreadable }}</span>
                  <span v-if="w.versionName" class="badge">{{ w.versionName }}</span>
                </div>
                <div class="flex justify-between text-[12px] text-faint">
                  <span>{{ formatRelative(w.lastPlayed) }}</span>
                  <span>{{ formatBytes(w.size) }}</span>
                </div>
              </template>
              <template v-else>
              <img v-if="w.icon" :src="w.icon" alt="" class="pixelated size-10 shrink-0 rounded-md" />
              <span v-else class="pix flex size-10 shrink-0 items-center justify-center bg-ok/15 text-ok"><Globe :size="18" /></span>
              <span class="min-w-0 flex-1">
                <span class="flex items-center gap-2">
                  <span class="truncate font-semibold">{{ w.name }}</span>
                  <span v-if="w.hardcore" class="badge border-danger/50 bg-danger/15 text-danger">{{ t.worlds.hardcore }}</span>
                  <span v-if="w.gameMode === null" class="badge text-warn">{{ t.worlds.unreadable }}</span>
                </span>
                <span class="block truncate text-[12px] text-faint">{{ w.dirName }}</span>
              </span>
              <span class="w-28 shrink-0 truncate text-[12.5px] text-muted">{{ w.gameMode ? t.worlds.gameModes[w.gameMode] : '—' }}</span>
              <span class="w-24 shrink-0 truncate text-[12.5px] text-muted">{{ w.versionName ?? '—' }}</span>
              <span class="w-28 shrink-0 truncate text-[12.5px] text-faint">{{ formatRelative(w.lastPlayed) }}</span>
              <span class="w-20 shrink-0 text-right text-[12.5px] text-faint">{{ formatBytes(w.size) }}</span>
              </template>
            </li>
          </ul>
        </div>
        <p class="text-[12px] text-faint">{{ t.worlds.selectionHint }}</p>
        </template>
      </div>
      <MarqueeBox :box="box" />
      <ActionPanel :actions="panelActions" :footer="footerActions" />
    </div>

    <ModalDialog v-if="renaming" :title="t.worlds.renameTitle" size="sm" @close="renaming = null">
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

    <ModalDialog v-if="backingUp" :title="t.worlds.backupTitle" size="sm" @close="backingUp = null">
      <p class="text-muted">{{ t.worlds.backupHint }}</p>
      <template #footer>
        <button class="btn" @click="backup(true)">{{ t.worlds.backupChoose }}</button>
        <button class="btn btn-primary" @click="backup(false)">{{ t.worlds.backupDefault }}</button>
      </template>
    </ModalDialog>

    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :items="menu.items" @close="menu = null" />

    <ConfirmDialog
      v-if="deleting.length"
      :title="deleting.length > 1 ? t.worlds.deleteMany(deleting.length) : t.worlds.deleteTitle"
      :text="deleting.length > 1 ? deleting.map((w) => w.name).join(', ') : t.worlds.deleteText(deleting[0]!.name)"
      @confirm="confirmDelete"
      @close="deleting = []"
    />
  </div>
</template>
