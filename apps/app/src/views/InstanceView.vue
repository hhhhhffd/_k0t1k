<script setup lang="ts">
import { ArrowLeft, Camera, Clock, FileArchive, Flame, FolderOpen, Globe, History, Image, Layers, Package, Puzzle, RefreshCw, ScrollText, Server, Settings2, Sparkles, Upload } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { addLocalFiles, openInstanceFolder } from '../api/commands'
import type { ExportFormat } from '../api/types'
import ContextMenu from '../components/ContextMenu.vue'
import type { MenuItem } from '../components/ContextMenu.vue'
import DropOverlay from '../components/DropOverlay.vue'
import ExportDialog from '../components/ExportDialog.vue'
import PackUpdateDialog from '../components/PackUpdateDialog.vue'
import InstanceIcon from '../components/InstanceIcon.vue'
import InstanceSettingsTab from '../components/instance/InstanceSettingsTab.vue'
import LogsTab from '../components/instance/LogsTab.vue'
import ResourceList from '../components/instance/ResourceList.vue'
import ScreenshotsTab from '../components/instance/ScreenshotsTab.vue'
import ServersTab from '../components/instance/ServersTab.vue'
import VersionTab from '../components/instance/VersionTab.vue'
import WorldsTab from '../components/instance/WorldsTab.vue'
import PlayButton from '../components/PlayButton.vue'
import UiIcon from '../components/UiIcon.vue'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useLayoutStore } from '../stores/layout'
import { useTasksStore } from '../stores/tasks'
import { formatPlaytime, formatRelative, loaderOf, minecraftVersionOf } from '../utils/format'
import { baseName, useFileDrop } from '../utils/useFileDrop'

const props = defineProps<{ id: string }>()
const store = useInstancesStore()
const tasks = useTasksStore()
const layout = useLayoutStore()
const router = useRouter()
const route = useRoute()

const instance = computed(() => store.byId(props.id))
// the status bar and the library follow the instance being edited
watch(() => props.id, (id) => (layout.selectedId = id), { immediate: true })
const loader = computed(() => (instance.value ? loaderOf(instance.value) : undefined))

type Tab = 'version' | 'mods' | 'resourcepacks' | 'shaders' | 'worlds' | 'screenshots' | 'servers' | 'logs' | 'settings'
const tabs: Tab[] = ['version', 'mods', 'resourcepacks', 'shaders', 'worlds', 'screenshots', 'servers', 'logs', 'settings']
const tabIcons = {
  version: { icon: Layers, color: 'text-ok' },
  mods: { icon: Puzzle, color: 'text-accent' },
  resourcepacks: { icon: Image, color: 'text-warn' },
  shaders: { icon: Sparkles, color: 'text-warn' },
  worlds: { icon: Globe, color: 'text-ok' },
  screenshots: { icon: Camera, color: 'text-muted' },
  servers: { icon: Server, color: 'text-accent' },
  logs: { icon: ScrollText, color: 'text-muted' },
  settings: { icon: Settings2, color: 'text-muted' },
} as const
// `?tab=logs` opens a tab directly (the crash dialog does)
const tabFromRoute = () => tabs.find((name) => name === route.query.tab) ?? 'mods'
const tab = ref<Tab>(tabFromRoute())
watch(() => route.query.tab, () => (tab.value = tabFromRoute()))
const sharedFolderOfTab = { resourcepacks: 'resourcepacks', shaders: 'shaderpacks' } as const
/** The notice shown above a resource list whose folder is linked to the shared one. */
function sharedNote(tab: Tab) {
  if (tab !== 'resourcepacks' && tab !== 'shaders') return undefined
  const folder = sharedFolderOfTab[tab]
  return instance.value?.sharedFolders.includes(folder) ? t.sharedFolder.notes[folder] : undefined
}
const kindOfTab = { mods: 'mod', resourcepacks: 'resourcePack', shaders: 'shaderPack' } as const

const exportMenu = ref<{ x: number; y: number } | null>(null)
const exportFormat = ref<ExportFormat | null>(null)
const showPackUpdate = ref(false)
const exportItems: MenuItem[] = [
  { label: t.transfer.exportInstance, icon: FileArchive, action: () => (exportFormat.value = 'instance') },
  { label: t.transfer.exportModrinth, icon: Package, action: () => (exportFormat.value = 'modrinth') },
  { label: t.transfer.exportCurseForge, icon: Flame, action: () => (exportFormat.value = 'curseForge') },
]
function openExportMenu(e: MouseEvent) {
  const box = (e.currentTarget as HTMLElement).getBoundingClientRect()
  exportMenu.value = { x: box.left, y: box.bottom + 4 }
}

// Files dropped on a content tab go into its folder; elsewhere on the page each file's type is identified.
const listKey = ref(0)
const { dragging } = useFileDrop(async (paths) => {
  if (!instance.value) return
  const kind = tab.value === 'mods' || tab.value === 'resourcepacks' || tab.value === 'shaders' ? kindOfTab[tab.value] : null
  const report = await tasks.guard(() => addLocalFiles(instance.value!.id, kind, paths))
  if (!report) return
  if (report.added.length) tasks.toast(t.transfer.addedFiles(report.added.length), 'ok')
  for (const skipped of report.skipped) {
    tasks.toast(t.transfer.skippedFile(baseName(skipped.path), t.transfer.skipped[skipped.reason]), 'error')
  }
  listKey.value++
})
</script>

<template>
  <div v-if="instance" class="flex h-full min-h-0 flex-col">
    <header class="flex shrink-0 items-center gap-3 border-b border-line px-4 py-3">
      <button class="btn btn-ghost btn-icon shrink-0" :title="t.nav.library" :aria-label="t.nav.library" @click="router.push('/')"><ArrowLeft :size="17" /></button>
      <InstanceIcon :icon="instance.icon" :instance-id="instance.id" :size="44" />
      <div class="min-w-0 flex-1">
        <h1 class="truncate text-[17px] leading-tight font-bold">{{ instance.name }}</h1>
        <div class="mt-1 flex flex-wrap items-center gap-1.5 text-[12px]">
          <span class="badge">Minecraft {{ minecraftVersionOf(instance) }}</span>
          <span v-if="loader" class="badge badge-accent">{{ loader.label }} {{ loader.version }}</span>
          <span v-if="instance.group" class="badge">{{ instance.group }}</span>
          <span class="ml-1 flex items-center gap-1 text-muted"><Clock :size="12" />{{ formatPlaytime(instance.playtime.totalSecs) }}</span>
          <span class="flex items-center gap-1 text-muted"><History :size="12" />{{ formatRelative(instance.lastPlayed) }}</span>
        </div>
      </div>
      <button v-if="instance.managedPack" class="tool shrink-0" @click="showPackUpdate = true"><RefreshCw :size="15" class="text-ok" />{{ t.pack.check }}</button>
      <button class="tool shrink-0" @click="openExportMenu"><Upload :size="15" class="text-ok" />{{ t.transfer.export }}</button>
      <button class="tool shrink-0" @click="tasks.guard(() => openInstanceFolder(instance!.id))"><FolderOpen :size="15" class="text-warn" />{{ t.panel.folder }}</button>
      <PlayButton :instance-id="instance.id" />
    </header>

    <div class="flex min-h-0 flex-1">
      <nav class="flex w-48 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-line bg-surface p-1.5" role="tablist" aria-orientation="vertical">
        <button
          v-for="name in tabs"
          :key="name"
          role="tab"
          :aria-selected="tab === name"
          class="tool h-9 w-full justify-start"
          :class="tab === name ? 'tool-active text-accent' : ''"
          @click="tab = name"
        >
          <UiIcon :name="`tab-${name}`" :icon="tabIcons[name].icon" :size="16" :class="tab === name ? '' : tabIcons[name].color" />
          {{ t.instance.tabs[name] }}
        </button>
      </nav>

      <section class="min-w-0 flex-1 overflow-y-auto p-4">
        <h2 class="mb-3 text-[15px] font-semibold">{{ t.instance.tabs[tab] }}</h2>
        <ResourceList
          v-if="tab === 'mods' || tab === 'resourcepacks' || tab === 'shaders'"
          :key="`${instance.id}-${tab}-${listKey}`"
          :instance-id="instance.id"
          :kind="kindOfTab[tab]"
          :shared-note="sharedNote(tab)"
        />
        <VersionTab v-else-if="tab === 'version'" :instance="instance" />
        <WorldsTab v-else-if="tab === 'worlds'" :key="instance.id" :instance-id="instance.id" />
        <ScreenshotsTab v-else-if="tab === 'screenshots'" :key="instance.id" :instance-id="instance.id" :shared="instance.sharedFolders.includes('screenshots')" />
        <ServersTab v-else-if="tab === 'servers'" :key="instance.id" :instance-id="instance.id" />
        <LogsTab v-else-if="tab === 'logs'" :instance-id="instance.id" />
        <InstanceSettingsTab v-else :key="instance.id" :instance="instance" />
      </section>
    </div>
  </div>
  <p v-else-if="store.loaded" class="p-10 text-center text-faint">{{ t.instance.notFound }}</p>
  <template v-if="instance">
    <ContextMenu v-if="exportMenu" :x="exportMenu.x" :y="exportMenu.y" :items="exportItems" @close="exportMenu = null" />
    <ExportDialog v-if="exportFormat" :instance="instance" :format="exportFormat" @close="exportFormat = null" />
    <PackUpdateDialog v-if="showPackUpdate && instance.managedPack" :instance="instance" @close="showPackUpdate = false" />
    <DropOverlay v-if="dragging" :text="t.transfer.dropHere" />
  </template>
</template>
