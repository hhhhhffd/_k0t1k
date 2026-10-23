<script setup lang="ts">
import { ChevronDown, ChevronRight, Copy, FileUp, FolderInput, FolderOpen, LayoutGrid, List, Pencil, Plus, Search, Trash2 } from '@lucide/vue'
import { open } from '@tauri-apps/plugin-dialog'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { isTauri, openInstanceFolder } from '../api/commands'
import DropOverlay from '../components/DropOverlay.vue'
import type { Instance } from '../api/types'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import ContextMenu from '../components/ContextMenu.vue'
import type { MenuItem } from '../components/ContextMenu.vue'
import CreateInstanceDialog from '../components/CreateInstanceDialog.vue'
import InstanceCard from '../components/InstanceCard.vue'
import PromptDialog from '../components/PromptDialog.vue'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import mascot from '../assets/mascot.png'
import { baseName, useFileDrop } from '../utils/useFileDrop'

const store = useInstancesStore()
const tasks = useTasksStore()
const router = useRouter()

const VIEW_KEY = 'k0t1k.library.view'
function readView(): 'grid' | 'list' {
  try {
    return localStorage.getItem(VIEW_KEY) === 'list' ? 'list' : 'grid'
  } catch {
    return 'grid'
  }
}
const view = ref<'grid' | 'list'>(readView())
function setView(v: 'grid' | 'list') {
  view.value = v
  try {
    localStorage.setItem(VIEW_KEY, v)
  } catch {
    // view preference is a convenience only
  }
}

const search = ref('')
const collapsed = ref(new Set<string>())
const showCreate = ref(false)
const menu = ref<{ x: number; y: number; instance: Instance } | null>(null)
const dialog = ref<{ kind: 'rename' | 'copy' | 'group' | 'delete'; instance: Instance } | null>(null)

const UNGROUPED = ''
const sections = computed(() => {
  const needle = search.value.trim().toLowerCase()
  const filtered = store.instances.filter((i) => !needle || i.name.toLowerCase().includes(needle))
  const map = new Map<string, Instance[]>()
  for (const inst of filtered) {
    const key = inst.group ?? UNGROUPED
    map.set(key, [...(map.get(key) ?? []), inst])
  }
  return [...map.entries()].sort(([a], [b]) => (a === UNGROUPED ? -1 : b === UNGROUPED ? 1 : a.localeCompare(b, 'ru')))
})
const hasGroups = computed(() => store.groups.length > 0)

function toggle(group: string) {
  const next = new Set(collapsed.value)
  if (!next.delete(group)) next.add(group)
  collapsed.value = next
}

const menuItems = computed<MenuItem[]>(() => {
  const inst = menu.value?.instance
  if (!inst) return []
  return [
    { label: t.library.openFolder, icon: FolderOpen, action: () => void tasks.guard(() => openInstanceFolder(inst.id)) },
    { label: t.library.copy, icon: Copy, action: () => (dialog.value = { kind: 'copy', instance: inst }) },
    { label: t.library.rename, icon: Pencil, action: () => (dialog.value = { kind: 'rename', instance: inst }) },
    { label: t.library.changeGroup, icon: FolderInput, action: () => (dialog.value = { kind: 'group', instance: inst }) },
    { label: t.library.delete, icon: Trash2, danger: true, separatorBefore: true, action: () => (dialog.value = { kind: 'delete', instance: inst }) },
  ]
})

async function submitPrompt(value: string) {
  const d = dialog.value
  if (!d) return
  dialog.value = null
  if (d.kind === 'rename') await tasks.guard(() => store.rename(d.instance.id, value))
  else if (d.kind === 'copy') await tasks.guard(() => store.copy(d.instance.id, value))
  else if (d.kind === 'group') await tasks.guard(() => store.setGroup(d.instance.id, value || null))
}
async function confirmDelete() {
  const d = dialog.value
  dialog.value = null
  if (d) await tasks.guard(() => store.remove(d.instance.id))
}

async function importPack() {
  const path = isTauri
    ? await open({ multiple: false, filters: [{ name: t.transfer.importFilter, extensions: ['mrpack', 'zip'] }] })
    : 'C:\\Downloads\\Example Pack.mrpack'
  if (typeof path === 'string') void tasks.guard(() => store.importModpack(path))
}

// Dropped modpacks and instance zips are imported one after another.
const { dragging } = useFileDrop(async (paths) => {
  for (const path of paths) {
    if (!/\.(mrpack|zip)$/i.test(path)) {
      tasks.toast(t.transfer.dropImportSkipped(baseName(path)), 'error')
      continue
    }
    const imported = await tasks.guard(() => store.importModpack(path))
    if (imported) tasks.toast(t.transfer.importedOk(imported.name), 'ok')
  }
})
</script>

<template>
  <div class="mx-auto flex max-w-6xl flex-col gap-4 p-6">
    <header class="flex items-center gap-3">
      <h1 class="mr-auto text-xl font-bold">{{ t.nav.library }}</h1>
      <div class="relative w-64">
        <Search :size="15" class="pointer-events-none absolute top-2.5 left-2.5 text-faint" />
        <input v-model="search" class="input !pl-8" :placeholder="t.library.searchPlaceholder" :aria-label="t.common.search" />
      </div>
      <div class="flex gap-0.5 rounded-lg border border-line p-0.5">
        <button class="btn btn-ghost btn-icon h-7 !w-7" :class="view === 'grid' ? '!bg-hover' : ''" :title="t.library.grid" :aria-label="t.library.grid" @click="setView('grid')"><LayoutGrid :size="15" /></button>
        <button class="btn btn-ghost btn-icon h-7 !w-7" :class="view === 'list' ? '!bg-hover' : ''" :title="t.library.list" :aria-label="t.library.list" @click="setView('list')"><List :size="15" /></button>
      </div>
      <button class="btn" @click="importPack"><FileUp :size="15" />{{ t.library.import }}</button>
      <button class="btn btn-primary" @click="showCreate = true"><Plus :size="16" />{{ t.library.new }}</button>
    </header>

    <div v-if="store.loaded && !store.instances.length" class="flex flex-col items-center gap-3 py-24 text-center">
      <img :src="mascot" alt="" class="pixelated anim-bob size-24 opacity-90" />
      <p class="text-[16px] font-semibold">{{ t.library.empty }}</p>
      <p class="text-muted">{{ t.library.emptyHint }}</p>
      <button class="btn btn-primary mt-2" @click="showCreate = true"><Plus :size="16" />{{ t.library.new }}</button>
    </div>
    <p v-else-if="store.loaded && !sections.length" class="py-16 text-center text-faint">{{ t.common.nothingFound }}</p>

    <section v-for="[group, items] in sections" :key="group">
      <button
        v-if="hasGroups"
        class="mb-2 flex items-center gap-1.5 text-faint hover:text-fg"
        :aria-expanded="!collapsed.has(group)"
        @click="toggle(group)"
      >
        <component :is="collapsed.has(group) ? ChevronRight : ChevronDown" :size="15" />
        <span class="section-title">{{ group || t.library.ungrouped }}</span>
        <span class="badge">{{ items.length }}</span>
      </button>
      <div
        v-if="!collapsed.has(group)"
        :class="view === 'grid' ? 'grid grid-cols-[repeat(auto-fill,minmax(230px,1fr))] gap-3' : 'flex flex-col gap-2'"
      >
        <InstanceCard
          v-for="inst in items"
          :key="inst.id"
          :instance="inst"
          :list="view === 'list'"
          @open="router.push({ name: 'instance', params: { id: inst.id } })"
          @menu="menu = { x: $event.clientX, y: $event.clientY, instance: inst }"
        />
      </div>
    </section>

    <CreateInstanceDialog v-if="showCreate" @close="showCreate = false" />
    <DropOverlay v-if="dragging" :text="t.transfer.dropImport" />
    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :items="menuItems" @close="menu = null" />
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
