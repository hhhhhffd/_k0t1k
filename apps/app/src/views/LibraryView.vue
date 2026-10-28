<script setup lang="ts">
import { ChevronDown, ChevronRight, Loader, Tag, Trash2, X } from '@lucide/vue'
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import type { Instance } from '../api/types'
import mascot from '../assets/mascot.png'
import CatBackdrop from '../components/CatBackdrop.vue'
import ContextMenu from '../components/ContextMenu.vue'
import DropOverlay from '../components/DropOverlay.vue'
import InstanceActionDialogs from '../components/InstanceActionDialogs.vue'
import InstanceIcon from '../components/InstanceIcon.vue'
import InstancePanel from '../components/InstancePanel.vue'
import MarqueeBox from '../components/MarqueeBox.vue'
import ViewToggle from '../components/ViewToggle.vue'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useLayoutStore } from '../stores/layout'
import { useTasksStore } from '../stores/tasks'
import { addFilesToInstance } from '../utils/addFiles'
import { loaderOf, minecraftVersionOf } from '../utils/format'
import { baseName, useFileDrop } from '../utils/useFileDrop'
import { useInstanceActions } from '../utils/useInstanceActions'
import { useMarquee } from '../utils/useMarquee'
import { useSelection } from '../utils/useSelection'
import { useViewMode } from '../utils/useViewMode'

const store = useInstancesStore()
const tasks = useTasksStore()
const layout = useLayoutStore()
const router = useRouter()

const collapsed = ref(new Set<string>())
const menu = ref<{ x: number; y: number; instance: Instance } | null>(null)

const selected = computed(() => (layout.selectedId ? store.byId(layout.selectedId) : undefined))

const view = useViewMode('library', 'tiles')
/** Instances picked with Ctrl/Shift or the rubber band; when empty, the single instance in `layout.selectedId` is the selection. */
const picked = ref(new Set<string>())
const selection = useSelection(picked)
const libraryRoot = ref<HTMLElement | null>(null)
const { box } = useMarquee(picked, libraryRoot, onKey)
const pickedInstances = computed(() => store.instances.filter((i) => picked.value.has(i.id)))
const isSelected = (id: string) => (picked.value.size ? picked.value.has(id) : layout.selectedId === id)
// The side panel follows the picked instances: it shows one of them.
watch(picked, (ids) => {
  if (ids.size && !(layout.selectedId && ids.has(layout.selectedId))) layout.selectedId = [...ids].pop()!
})

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

/** Instance ids in display order (collapsed groups are skipped), for Shift ranges and Ctrl+A. */
const order = computed(() => sections.value.flatMap(([group, items]) => (collapsed.value.has(group) ? [] : items.map((i) => i.id))))

/** What an action started on `inst` applies to: all picked instances when it is one of them. */
const targetsOf = (inst: Instance) => (picked.value.has(inst.id) && picked.value.size > 1 ? pickedInstances.value : [inst])
const actions = useInstanceActions(targetsOf)
const { dialog, deleting, launch, rememberPointer, actionsFor } = actions

function onTileClick(inst: Instance, e: MouseEvent) {
  if (e.ctrlKey || e.metaKey || e.shiftKey) {
    if (!picked.value.size && selected.value) {
      picked.value = new Set([selected.value.id])
      selection.setAnchor(selected.value.id)
    }
    selection.click(inst.id, e, order.value)
  } else {
    picked.value = new Set()
  }
  layout.selectedId = inst.id
}
function clearSelection() {
  picked.value = new Set()
  layout.selectedId = null
}
function onKey(e: KeyboardEvent) {
  if (selection.onKey(e, order.value)) {
    if (e.key === 'Escape') layout.selectedId = null
  } else if (e.key === 'Delete') {
    const targets = picked.value.size ? pickedInstances.value : selected.value ? [selected.value] : []
    if (targets.length) deleting.value = targets
  }
}

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

const panelActions = computed(() => (selected.value ? actionsFor(selected.value) : []))
const menuItems = computed(() => (menu.value ? actionsFor(menu.value.instance) : []))

function openMenu(e: MouseEvent, inst: Instance) {
  if (!picked.value.has(inst.id)) picked.value = new Set()
  layout.selectedId = inst.id
  menu.value = { x: e.clientX, y: e.clientY, instance: inst }
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

const dropTarget = computed(() => (dragPosition.value ? instanceAt(dragPosition.value.x, dragPosition.value.y) : undefined))

/** The instance tile under a point, if any. */
const instanceAt = (x: number, y: number) =>
  store.byId(document.elementFromPoint(x, y)?.closest<HTMLElement>('[data-instance-id]')?.dataset.instanceId ?? '')

// Files dropped on a tile go into that instance (mods, packs, shaders, worlds); elsewhere modpacks and
// instance zips are imported one after another.
const { dragging, position: dragPosition } = useFileDrop(async (paths, at) => {
  const target = instanceAt(at.x, at.y)
  if (target) {
    layout.selectedId = target.id
    await addFilesToInstance(target.id, null, paths)
    return
  }
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
    <div class="relative min-w-0 flex-1">
      <CatBackdrop />
      <div class="absolute inset-0 overflow-y-auto px-3 py-2">
      <div ref="libraryRoot" class="min-h-full outline-none" @click.self="clearSelection">
        <div v-if="store.instances.length" class="flex justify-end px-1 pb-1" @click.self="clearSelection"><ViewToggle v-model="view" /></div>
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
          <div v-if="!collapsed.has(group)" class="flex gap-1 px-1 pb-2" :class="view === 'tiles' ? 'flex-wrap' : 'flex-col'">
            <div
              v-for="inst in items"
              :key="inst.id"
              class="tile group flex cursor-pointer rounded-lg outline-none"
              :data-instance-id="inst.id"
              :data-select-key="inst.id"
              data-no-marquee
              :class="[
                view === 'tiles' ? 'w-[104px] flex-col items-center gap-1.5 p-2' : 'w-full items-center gap-3 px-2 py-1.5',
                isSelected(inst.id) ? 'tile-selected' : 'hover:bg-hover',
                drag?.moved && drag.instance.id === inst.id ? 'opacity-40' : '',
                dropTarget?.id === inst.id ? 'bg-accent/15 ring-2 ring-accent' : '',
              ]"
              :title="`${minecraftVersionOf(inst)}${loaderOf(inst) ? ` · ${loaderOf(inst)!.label}${loaderOf(inst)!.disabled ? ` (${t.version.disabled})` : ''}` : ''}`"
              tabindex="0"
              role="button"
              :aria-pressed="isSelected(inst.id)"
              @pointerdown="tileDown($event, inst)"
              @click="onTileClick(inst, $event)"
              @dblclick="launch(inst)"
              @keydown.enter="launch(inst)"
              @focus="!picked.size && (layout.selectedId = inst.id)"
              @contextmenu.prevent="openMenu($event, inst)"
            >
              <span class="relative">
                <InstanceIcon :icon="inst.icon" :instance-id="inst.id" :size="view === 'tiles' ? 52 : 32" />
                <span
                  v-if="store.stateOf(inst.id) !== 'stopped'"
                  class="absolute -top-1 -right-1 size-3 rounded-full border-2 border-surface bg-ok shadow-[0_0_8px_var(--c-ok)]"
                  :title="t.library.running"
                />
              </span>
              <span
                class="rounded px-1 text-[12.5px] leading-tight"
                :class="view === 'tiles' ? 'line-clamp-2 w-full text-center break-words' : 'min-w-0 flex-1 truncate'"
              >{{ inst.name }}</span>
              <span v-if="view === 'list'" class="shrink-0 text-[12px] text-faint">
                {{ minecraftVersionOf(inst) }}<template v-if="loaderOf(inst)"> · {{ loaderOf(inst)!.label }}</template>
              </span>
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
      </div>
      <div
        v-if="picked.size > 1"
        class="absolute bottom-3 left-1/2 z-10 flex -translate-x-1/2 items-center gap-2 rounded-xl border border-line bg-surface px-3 py-2 shadow-lg"
      >
        <span class="text-muted">{{ t.instance.selectedCount(picked.size) }}</span>
        <button class="btn" @click="dialog = { kind: 'group', instance: pickedInstances[0]!, targets: pickedInstances }"><Tag :size="15" />{{ t.library.changeGroup }}</button>
        <button class="btn btn-danger" @click="deleting = pickedInstances"><Trash2 :size="15" />{{ t.common.delete }}</button>
        <button class="btn" :title="t.common.close" :aria-label="t.common.close" @click="picked = new Set()"><X :size="15" /></button>
      </div>
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
    <DropOverlay v-if="dragging" subtle :text="dropTarget ? t.transfer.dropOnInstance(dropTarget.name) : t.transfer.dropImport" />
    <MarqueeBox :box="box" />
    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :items="menuItems" @close="menu = null" />
    <InstanceActionDialogs :actions="actions" @deleted="picked = new Set([...picked].filter((id) => store.byId(id)))" />
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
