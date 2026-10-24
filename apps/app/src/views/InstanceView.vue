<script setup lang="ts">
import { ArrowLeft, Clock, FileArchive, Flame, FolderOpen, History, Package, RefreshCw, Upload } from '@lucide/vue'
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
import WorldsTab from '../components/instance/WorldsTab.vue'
import PlayButton from '../components/PlayButton.vue'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { formatPlaytime, formatRelative, loaderOf, minecraftVersionOf } from '../utils/format'
import { baseName, useFileDrop } from '../utils/useFileDrop'

const props = defineProps<{ id: string }>()
const store = useInstancesStore()
const tasks = useTasksStore()
const router = useRouter()
const route = useRoute()

const instance = computed(() => store.byId(props.id))
const loader = computed(() => (instance.value ? loaderOf(instance.value) : undefined))

type Tab = 'mods' | 'resourcepacks' | 'shaders' | 'worlds' | 'screenshots' | 'servers' | 'logs' | 'settings'
const tabs: Tab[] = ['mods', 'resourcepacks', 'shaders', 'worlds', 'screenshots', 'servers', 'logs', 'settings']
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
  <div v-if="instance" class="mx-auto flex max-w-5xl flex-col gap-4 p-6">
    <button class="btn btn-ghost -ml-2 w-fit text-muted" @click="router.push('/')"><ArrowLeft :size="15" />{{ t.nav.library }}</button>

    <header class="card flex items-center gap-4 p-4">
      <InstanceIcon :icon="instance.icon" :size="64" />
      <div class="min-w-0 flex-1">
        <h1 class="truncate text-[22px] leading-tight font-bold">{{ instance.name }}</h1>
        <div class="mt-1.5 flex flex-wrap items-center gap-1.5">
          <span class="badge">Minecraft {{ minecraftVersionOf(instance) }}</span>
          <span v-if="loader" class="badge badge-accent">{{ loader.label }} {{ loader.version }}</span>
          <span v-if="instance.group" class="badge">{{ instance.group }}</span>
          <span class="ml-2 flex items-center gap-1.5 text-muted"><Clock :size="13" />{{ formatPlaytime(instance.playtime.totalSecs) }}</span>
          <span class="flex items-center gap-1.5 text-muted"><History :size="13" />{{ formatRelative(instance.lastPlayed) }}</span>
        </div>
      </div>
      <button v-if="instance.managedPack" class="btn shrink-0" @click="showPackUpdate = true"><RefreshCw :size="15" />{{ t.pack.check }}</button>
      <button class="btn shrink-0" @click="openExportMenu"><Upload :size="15" />{{ t.transfer.export }}</button>
      <button class="btn btn-icon" :title="t.library.openFolder" :aria-label="t.library.openFolder" @click="tasks.guard(() => openInstanceFolder(instance!.id))"><FolderOpen :size="16" /></button>
      <PlayButton :instance-id="instance.id" large />
    </header>

    <div class="flex gap-1 overflow-x-auto overflow-y-hidden border-b border-line [scrollbar-width:none]" role="tablist">
      <button
        v-for="name in tabs"
        :key="name"
        role="tab"
        :aria-selected="tab === name"
        class="-mb-px h-9 border-b-2 px-3.5 font-medium transition-colors"
        :class="tab === name ? 'border-accent text-accent' : 'border-transparent text-muted hover:text-fg'"
        @click="tab = name"
      >
        {{ t.instance.tabs[name] }}
      </button>
    </div>

    <ResourceList
      v-if="tab === 'mods' || tab === 'resourcepacks' || tab === 'shaders'"
      :key="`${instance.id}-${tab}-${listKey}`"
      :instance-id="instance.id"
      :kind="kindOfTab[tab]"
      :shared-note="sharedNote(tab)"
    />
    <WorldsTab v-else-if="tab === 'worlds'" :key="instance.id" :instance-id="instance.id" />
    <ScreenshotsTab v-else-if="tab === 'screenshots'" :key="instance.id" :instance-id="instance.id" :shared="instance.sharedFolders.includes('screenshots')" />
    <ServersTab v-else-if="tab === 'servers'" :key="instance.id" :instance-id="instance.id" />
    <LogsTab v-else-if="tab === 'logs'" :instance-id="instance.id" />
    <InstanceSettingsTab v-else :key="instance.id" :instance="instance" />
  </div>
  <p v-else-if="store.loaded" class="p-10 text-center text-faint">{{ t.instance.notFound }}</p>
  <template v-if="instance">
    <ContextMenu v-if="exportMenu" :x="exportMenu.x" :y="exportMenu.y" :items="exportItems" @close="exportMenu = null" />
    <ExportDialog v-if="exportFormat" :instance="instance" :format="exportFormat" @close="exportFormat = null" />
    <PackUpdateDialog v-if="showPackUpdate && instance.managedPack" :instance="instance" @close="showPackUpdate = false" />
    <DropOverlay v-if="dragging" :text="t.transfer.dropHere" />
  </template>
</template>
