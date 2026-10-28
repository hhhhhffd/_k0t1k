<script setup lang="ts">
import { ArrowLeft, Clock, FileArchive, Flame, FolderOpen, History, Package, RefreshCw, Upload } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { openInstanceFolder } from '../api/commands'
import type { ExportFormat } from '../api/types'
import ContextMenu from '../components/ContextMenu.vue'
import type { MenuItem } from '../components/ContextMenu.vue'
import DropOverlay from '../components/DropOverlay.vue'
import ExportDialog from '../components/ExportDialog.vue'
import PackUpdateDialog from '../components/PackUpdateDialog.vue'
import InstanceIcon from '../components/InstanceIcon.vue'
import InstanceTabContent, { INSTANCE_TAB_ICONS, INSTANCE_TABS, type InstanceTab, kindOfTab } from '../components/instance/InstanceTabContent.vue'
import PlayButton from '../components/PlayButton.vue'
import SteamHero from '../components/SteamHero.vue'
import UiIcon from '../components/UiIcon.vue'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useLayoutStore } from '../stores/layout'
import { useTasksStore } from '../stores/tasks'
import { formatPlaytime, formatRelative, loaderOf, minecraftVersionOf } from '../utils/format'
import { addFilesToInstance } from '../utils/addFiles'
import { useFileDrop } from '../utils/useFileDrop'

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
/** The Steam-like mode shows the page under the version's art, with the tabs in a row. */
const steam = computed(() => layout.mode === 'steam')

// `?tab=logs` opens a tab directly (the crash dialog does)
const tabFromRoute = () => INSTANCE_TABS.find((name) => name === route.query.tab) ?? 'mods'
const tab = ref<InstanceTab>(tabFromRoute())
watch(() => route.query.tab, () => (tab.value = tabFromRoute()))

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
  if (await addFilesToInstance(instance.value.id, kindOfTab(tab.value), paths)) listKey.value++
})
</script>

<template>
  <div v-if="instance && steam" class="h-full min-h-0 overflow-y-auto">
    <SteamHero
      :instance="instance"
      @export="openExportMenu"
      @update="showPackUpdate = true"
      @folder="tasks.guard(() => openInstanceFolder(instance!.id))"
      @settings="tab = 'settings'"
    />
    <nav class="mx-6 flex flex-wrap rounded-sm bg-surface" role="tablist">
      <button
        v-for="name in INSTANCE_TABS"
        :key="name"
        role="tab"
        :aria-selected="tab === name"
        class="steam-tab flex h-11 shrink-0 items-center gap-2 px-4 text-[13px] transition-colors"
        :class="tab === name ? 'steam-tab-active text-fg' : 'text-muted hover:bg-hover hover:text-fg'"
        @click="tab = name"
      >
        <UiIcon :name="`tab-${name}`" :icon="INSTANCE_TAB_ICONS[name]" :size="15" class="text-icon" />
        {{ t.instance.tabs[name] }}
      </button>
    </nav>
    <section class="px-6 pt-5 pb-6">
      <h2 class="section-title mb-3">{{ t.instance.tabs[tab] }}</h2>
      <InstanceTabContent :instance="instance" :tab="tab" :list-key="listKey" />
    </section>
  </div>
  <div v-else-if="instance" class="flex h-full min-h-0 flex-col">
    <header class="flex shrink-0 items-center gap-3 border-b border-line px-4 py-3">
      <button class="btn btn-ghost btn-icon shrink-0" :title="t.nav.library" :aria-label="t.nav.library" @click="router.push('/')"><ArrowLeft :size="17" /></button>
      <InstanceIcon :icon="instance.icon" :instance-id="instance.id" :size="44" />
      <div class="min-w-0 flex-1">
        <h1 class="truncate text-[17px] leading-tight font-bold">{{ instance.name }}</h1>
        <div class="mt-1 flex flex-wrap items-center gap-1.5 text-[12px]">
          <span class="badge">Minecraft {{ minecraftVersionOf(instance) }}</span>
          <span v-if="loader" class="badge" :class="loader.disabled ? 'text-faint line-through' : 'badge-accent'" :title="loader.disabled ? t.version.disabled : ''">{{ loader.label }} {{ loader.version }}</span>
          <span v-if="instance.group" class="badge">{{ instance.group }}</span>
          <span class="ml-1 flex items-center gap-1 text-muted"><Clock :size="12" />{{ formatPlaytime(instance.playtime.totalSecs) }}</span>
          <span class="flex items-center gap-1 text-muted"><History :size="12" />{{ formatRelative(instance.lastPlayed) }}</span>
        </div>
      </div>
      <button v-if="instance.managedPack" class="tool shrink-0" @click="showPackUpdate = true"><RefreshCw :size="15" class="text-icon" />{{ t.pack.check }}</button>
      <button class="tool shrink-0" @click="openExportMenu"><Upload :size="15" class="text-icon" />{{ t.transfer.export }}</button>
      <button class="tool shrink-0" :title="t.instance.instanceFolderHint" @click="tasks.guard(() => openInstanceFolder(instance!.id))"><FolderOpen :size="15" class="text-icon" />{{ t.instance.instanceFolder }}</button>
      <PlayButton :instance-id="instance.id" />
    </header>

    <div class="flex min-h-0 flex-1">
      <nav class="flex w-48 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-line bg-surface p-1.5" role="tablist" aria-orientation="vertical">
        <button
          v-for="name in INSTANCE_TABS"
          :key="name"
          role="tab"
          :aria-selected="tab === name"
          class="tool h-9 w-full justify-start"
          :class="tab === name ? 'tool-active text-accent' : ''"
          @click="tab = name"
        >
          <UiIcon :name="`tab-${name}`" :icon="INSTANCE_TAB_ICONS[name]" :size="16" class="text-icon" />
          {{ t.instance.tabs[name] }}
        </button>
      </nav>

      <section class="min-w-0 flex-1 overflow-y-auto p-4">
        <h2 class="mb-3 text-[15px] font-semibold">{{ t.instance.tabs[tab] }}</h2>
        <InstanceTabContent :instance="instance" :tab="tab" :list-key="listKey" />
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

<style scoped>
.steam-tab {
  border-bottom: 2px solid transparent;
}
.steam-tab-active {
  border-bottom-color: var(--c-accent);
  background: color-mix(in srgb, var(--c-accent) 10%, transparent);
}
</style>
