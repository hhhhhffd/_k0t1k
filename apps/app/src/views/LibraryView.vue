<script setup lang="ts">
import { ChevronDown, ChevronRight, Copy, Loader, FileArchive, Flame, FolderOpen, Package, Pencil, Play, Puzzle, RefreshCw, Square, Tag, TextCursorInput, Trash2, Upload } from '@lucide/vue'
import { computed, onBeforeUnmount, ref } from 'vue'
import { useRouter } from 'vue-router'
import { openInstanceFolder } from '../api/commands'
import type { ExportFormat, Instance } from '../api/types'
import mascot from '../assets/mascot.png'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import ContextMenu from '../components/ContextMenu.vue'
import type { MenuItem } from '../components/ContextMenu.vue'
import DropOverlay from '../components/DropOverlay.vue'
import ExportDialog from '../components/ExportDialog.vue'
import InstanceIcon from '../components/InstanceIcon.vue'
import InstancePanel from '../components/InstancePanel.vue'
import PackUpdateDialog from '../components/PackUpdateDialog.vue'
import PromptDialog from '../components/PromptDialog.vue'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useLayoutStore } from '../stores/layout'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'
import { loaderOf, minecraftVersionOf } from '../utils/format'
import { baseName, useFileDrop } from '../utils/useFileDrop'

const store = useInstancesStore()
const settings = useSettingsStore()
const tasks = useTasksStore()
const layout = useLayoutStore()
const router = useRouter()

const collapsed = ref(new Set<string>())
const menu = ref<{ x: number; y: number; instance: Instance } | null>(null)
const dialog = ref<{ kind: 'rename' | 'copy' | 'group' | 'delete'; instance: Instance } | null>(null)
const exportMenu = ref<{ x: number; y: number; instance: Instance } | null>(null)
const exporting = ref<{ format: ExportFormat; instance: Instance } | null>(null)
const packUpdate = ref<Instance | null>(null)

const selected = computed(() => (layout.selectedId ? store.byId(layout.selectedId) : undefined))

const UNGROUPED = ''
const sections = computed(() => {
  const needle = layout.search.trim().toLowerCase()
  const filtered = store.instances.filter((i) => !needle || i.name.toLowerCase().includes(needle))
  const map = new Map<string, Instance[]>()
  for (const inst of filtered) {
    const key = inst.group ?? UNGROUPED
    map.set(key, [...(map.get(key) ?? []), inst])
  }
  for (const items of map.values()) items.sort((a, b) => a.name.localeCompare(b.name, 'ru'))
  for (const p of store.pending) if (!map.has(p.group ?? UNGROUPED)) map.set(p.group ?? UNGROUPED, [])
  return [...map.entries()].sort(([a], [b]) => (a === UNGROUPED ? -1 : b === UNGROUPED ? 1 : a.localeCompare(b, 'ru')))
})

const pendingIn = (group: string) => store.pending.filter((p) => (p.group ?? UNGROUPED) === group)
/** Background download of the game files of an instance (after creating it or changing its versions). */
function downloadOf(inst: Instance) {
  const task = tasks.tasks.find((e) => e.task === `game:${inst.id}` && !e.finished && !e.error)
  return task ? { percent: task.total ? Math.round((task.done / task.total) * 100) : 0, stage: task.stage } : null
}

function toggle(group: string) {
  const next = new Set(collapsed.value)
  if (!next.delete(group)) next.add(group)
  collapsed.value = next
}

const launch = (inst: Instance) => void tasks.guard(() => store.launch(inst.id, settings.settings?.selectedAccountId ?? undefined))

// The export formats open at the pointer, wherever "Export" was picked (panel or context menu).
let lastPointer = { x: 0, y: 0 }
const rememberPointer = (e: PointerEvent) => (lastPointer = { x: e.clientX, y: e.clientY })

/** One action list for the side panel and the context menu, in Prism's order. */
function actionsFor(inst: Instance): MenuItem[] {
  const running = store.stateOf(inst.id) !== 'stopped'
  return [
    { label: running ? t.library.playAgain : t.panel.launch, icon: Play, iconClass: 'text-ok', action: () => launch(inst) },
    { label: t.library.stop, icon: Square, iconClass: 'text-danger', disabled: !running, action: () => void tasks.guard(() => store.kill(inst.id)) },
    { label: t.panel.edit, icon: Pencil, iconClass: 'text-accent', separatorBefore: true, action: () => router.push({ name: 'instance', params: { id: inst.id } }) },
    { label: t.panel.versions, icon: Puzzle, iconClass: 'text-accent', action: () => router.push({ name: 'instance', params: { id: inst.id }, query: { tab: 'version' } }) },
    { label: t.library.changeGroup, icon: Tag, iconClass: 'text-warn', action: () => (dialog.value = { kind: 'group', instance: inst }) },
    { label: t.library.rename, icon: TextCursorInput, iconClass: 'text-muted', action: () => (dialog.value = { kind: 'rename', instance: inst }) },
    { label: t.panel.folder, icon: FolderOpen, iconClass: 'text-warn', action: () => void tasks.guard(() => openInstanceFolder(inst.id)) },
    { label: t.transfer.export, icon: Upload, iconClass: 'text-ok', action: () => (exportMenu.value = { ...lastPointer, instance: inst }) },
    ...(inst.managedPack ? [{ label: t.pack.check, icon: RefreshCw, iconClass: 'text-ok', action: () => (packUpdate.value = inst) }] : []),
    { label: t.library.copy, icon: Copy, iconClass: 'text-muted', action: () => (dialog.value = { kind: 'copy', instance: inst }) },
    { label: t.library.delete, icon: Trash2, iconClass: 'text-danger', danger: true, separatorBefore: true, action: () => (dialog.value = { kind: 'delete', instance: inst }) },
  ]
}
const panelActions = computed(() => (selected.value ? actionsFor(selected.value) : []))
const menuItems = computed(() => (menu.value ? actionsFor(menu.value.instance) : []))
const exportItems = computed<MenuItem[]>(() => {
  const inst = exportMenu.value?.instance
  if (!inst) return []
  const pick = (format: ExportFormat) => () => (exporting.value = { format, instance: inst })
  return [
    { label: t.transfer.exportInstance, icon: FileArchive, action: pick('instance') },
    { label: t.transfer.exportModrinth, icon: Package, iconClass: 'text-ok', action: pick('modrinth') },
    { label: t.transfer.exportCurseForge, icon: Flame, iconClass: 'text-warn', action: pick('curseForge') },
  ]
})

function openMenu(e: MouseEvent, inst: Instance) {
  layout.selectedId = inst.id
  menu.value = { x: e.clientX, y: e.clientY, instance: inst }
}

async function submitPrompt(value: string) {
  const d = dialog.value
  if (!d) return
  dialog.value = null
  if (d.kind === 'rename') await tasks.guard(() => store.rename(d.instance.id, value))
  else if (d.kind === 'group') await tasks.guard(() => store.setGroup(d.instance.id, value || null))
  else {
    const copy = await tasks.guard(() => store.copy(d.instance.id, value))
    if (copy) layout.selectedId = copy.id
  }
}
async function confirmDelete() {
  const d = dialog.value
  dialog.value = null
  if (d) await tasks.guard(() => store.remove(d.instance.id))
}

// Tiles are dragged onto another group with pointer events: WebView2 gives HTML drag and drop to Tauri's file drop.
const drag = ref<{ instance: Instance; x: number; y: number; moved: boolean; over: string | null } | null>(null)
let dragStart = { x: 0, y: 0 }
function tileDown(e: PointerEvent, inst: Instance) {
  if (e.button !== 0) return
  dragStart = { x: e.clientX, y: e.clientY }
  drag.value = { instance: inst, x: e.clientX, y: e.clientY, moved: false, over: null }
  window.addEventListener('pointermove', dragMove)
  window.addEventListener('pointerup', dragEnd)
}
function dragMove(e: PointerEvent) {
  const d = drag.value
  if (!d) return
  if (!d.moved && Math.hypot(e.clientX - dragStart.x, e.clientY - dragStart.y) < 6) return
  const target = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>('[data-group]')
  drag.value = { ...d, x: e.clientX, y: e.clientY, moved: true, over: target?.dataset.group ?? null }
}
function dragEnd() {
  const d = drag.value
  stopDrag()
  if (!d?.moved || d.over === null || d.over === (d.instance.group ?? UNGROUPED)) return
  void tasks.guard(() => store.setGroup(d.instance.id, d.over || null))
}
function stopDrag() {
  drag.value = null
  window.removeEventListener('pointermove', dragMove)
  window.removeEventListener('pointerup', dragEnd)
}
onBeforeUnmount(stopDrag)

// Dropped modpacks and instance zips are imported one after another.
const { dragging } = useFileDrop(async (paths) => {
  for (const path of paths) {
    if (!/\.(mrpack|zip)$/i.test(path)) {
      tasks.toast(t.transfer.dropImportSkipped(baseName(path)), 'error')
      continue
    }
    const imported = await tasks.guard(() => store.importModpack(path))
    if (imported) {
      tasks.toast(t.transfer.importedOk(imported.name), 'ok')
      layout.selectedId = imported.id
    }
  }
})
</script>

<template>
  <div class="flex h-full min-h-0" :class="layout.panel === 'left' ? 'flex-row-reverse' : ''" @pointerdown.capture="rememberPointer">
    <div class="min-w-0 flex-1 overflow-y-auto px-3 py-2" @click.self="layout.selectedId = null">
      <div v-if="store.loaded && !store.instances.length && !store.pending.length" class="flex flex-col items-center gap-3 py-24 text-center">
        <img :src="mascot" alt="" class="pixelated anim-bob size-24 opacity-90" />
        <p class="text-[16px] font-semibold">{{ t.library.empty }}</p>
        <p class="text-muted">{{ t.library.emptyHint }}</p>
      </div>
      <p v-else-if="store.loaded && !sections.length" class="py-16 text-center text-faint">{{ t.common.nothingFound }}</p>

      <section
        v-for="[group, items] in sections"
        :key="group"
        :data-group="group"
        class="mb-1 rounded-lg transition-colors"
        :class="drag?.moved && drag.over === group && group !== (drag.instance.group ?? '') ? 'bg-accent/10 ring-1 ring-accent/50' : ''"
      >
        <button class="flex w-full items-center gap-1.5 px-1 py-1.5 text-faint hover:text-fg" :aria-expanded="!collapsed.has(group)" @click="toggle(group)">
          <component :is="collapsed.has(group) ? ChevronRight : ChevronDown" :size="15" />
          <span class="text-[12.5px] font-semibold">{{ group || t.library.ungrouped }}</span>
          <span class="text-[11px]">{{ items.length + pendingIn(group).length }}</span>
          <span class="ml-1 h-px flex-1 bg-line" />
        </button>
        <div v-if="!collapsed.has(group)" class="flex flex-wrap gap-1 px-1 pb-2">
          <div
            v-for="inst in items"
            :key="inst.id"
            class="tile group flex w-[104px] cursor-pointer flex-col items-center gap-1.5 rounded-lg p-2 outline-none"
            :class="[layout.selectedId === inst.id ? 'tile-selected' : 'hover:bg-hover', drag?.moved && drag.instance.id === inst.id ? 'opacity-40' : '']"
            :title="`${minecraftVersionOf(inst)}${loaderOf(inst) ? ` · ${loaderOf(inst)!.label}` : ''}`"
            tabindex="0"
            role="button"
            :aria-pressed="layout.selectedId === inst.id"
            @pointerdown="tileDown($event, inst)"
            @click="layout.selectedId = inst.id"
            @dblclick="launch(inst)"
            @keydown.enter="launch(inst)"
            @focus="layout.selectedId = inst.id"
            @contextmenu.prevent="openMenu($event, inst)"
          >
            <span class="relative">
              <InstanceIcon :icon="inst.icon" :instance-id="inst.id" :size="52" />
              <span
                v-if="store.stateOf(inst.id) !== 'stopped'"
                class="absolute -top-1 -right-1 size-3 rounded-full border-2 border-surface bg-ok shadow-[0_0_8px_var(--c-ok)]"
                :title="t.library.running"
              />
            </span>
            <span class="line-clamp-2 w-full rounded px-1 text-center text-[12.5px] leading-tight break-words">{{ inst.name }}</span>
            <span v-if="downloadOf(inst)" class="h-1 w-16 overflow-hidden rounded-full bg-raised" :title="`${t.library.downloading}: ${downloadOf(inst)!.stage}`">
              <span class="block h-full bg-accent transition-[width]" :style="{ width: `${downloadOf(inst)!.percent}%` }" />
            </span>
          </div>
          <div
            v-for="p in pendingIn(group)"
            :key="`pending-${p.key}`"
            class="flex w-[104px] flex-col items-center gap-1.5 rounded-lg p-2 opacity-80"
            :title="t.library.creating"
            aria-busy="true"
          >
            <span class="pix flex size-[52px] items-center justify-center bg-raised"><Loader :size="22" class="animate-spin text-accent" /></span>
            <span class="line-clamp-2 w-full px-1 text-center text-[12.5px] leading-tight break-words">{{ p.name }}</span>
            <span class="text-[11px] text-faint">{{ t.library.creating }}</span>
          </div>
        </div>
      </section>
    </div>

    <InstancePanel
      :instance="selected"
      :actions="panelActions"
      @rename="selected && (dialog = { kind: 'rename', instance: selected })"
      @edit="selected && router.push({ name: 'instance', params: { id: selected.id }, query: { tab: 'settings' } })"
    />

    <Teleport to="body">
      <div v-if="drag?.moved" class="pointer-events-none fixed z-[70] opacity-90" :style="{ left: `${drag.x - 26}px`, top: `${drag.y - 26}px` }">
        <InstanceIcon :icon="drag.instance.icon" :instance-id="drag.instance.id" :size="52" />
      </div>
    </Teleport>
    <DropOverlay v-if="dragging" :text="t.transfer.dropImport" />
    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :items="menuItems" @close="menu = null" />
    <ContextMenu v-if="exportMenu" :x="exportMenu.x" :y="exportMenu.y" :items="exportItems" @close="exportMenu = null" />
    <ExportDialog v-if="exporting" :instance="exporting.instance" :format="exporting.format" @close="exporting = null" />
    <PackUpdateDialog v-if="packUpdate?.managedPack" :instance="packUpdate" @close="packUpdate = null" />
    <PromptDialog
      v-if="dialog && dialog.kind !== 'delete'"
      :title="dialog.kind === 'rename' ? t.library.rename : dialog.kind === 'copy' ? t.library.copy : t.library.changeGroup"
      :label="dialog.kind === 'rename' ? t.library.newName : dialog.kind === 'copy' ? t.library.copyName : t.library.groupName"
      :initial="dialog.kind === 'rename' ? dialog.instance.name : dialog.kind === 'copy' ? `${dialog.instance.name} (копия)` : (dialog.instance.group ?? '')"
      :allow-empty="dialog.kind === 'group'"
      :suggestions="dialog.kind === 'group' ? store.groups : []"
      @submit="submitPrompt"
      @close="dialog = null"
    />
    <ConfirmDialog
      v-if="dialog?.kind === 'delete'"
      :title="t.library.deleteTitle"
      :text="t.library.deleteText(dialog.instance.name)"
      @confirm="confirmDelete"
      @close="dialog = null"
    />
  </div>
</template>

<style scoped>
.tile-selected {
  background: color-mix(in srgb, var(--c-accent) 18%, transparent);
  box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--c-accent) 55%, transparent);
}
.tile:focus-visible {
  box-shadow: inset 0 0 0 2px var(--c-accent);
}
</style>
