<script setup lang="ts">
import { ArrowLeft, Clock, FileArchive, Flame, FolderOpen, History, Package, RefreshCw, Upload } from '@lucide/vue'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { openInstanceFolder } from '../api/commands'
import type { ExportFormat } from '../api/types'
import CoachMarks from '../components/CoachMarks.vue'
import ContextMenu from '../components/ContextMenu.vue'
import type { MenuItem } from '../components/ContextMenu.vue'
import DropOverlay from '../components/DropOverlay.vue'
import ExportDialog from '../components/ExportDialog.vue'
import PackUpdateDialog from '../components/PackUpdateDialog.vue'
import InstanceIcon from '../components/InstanceIcon.vue'
import InstanceTabContent, { INSTANCE_TAB_ICONS, INSTANCE_TABS, type InstanceTab, kindOfTab } from '../components/instance/InstanceTabContent.vue'
import PlayButton from '../components/PlayButton.vue'
import SteamHero from '../components/SteamHero.vue'
import SteamPlayBar from '../components/SteamPlayBar.vue'
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

/** The tabs the first-open tour points at. */
const tourAnchor = (name: InstanceTab) => (name === 'mods' || name === 'settings' ? name : undefined)

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

// Panels that stick inside a tab stay below the pinned play bar and tabs, and no taller than the room left under them.
const page = ref<HTMLElement | null>(null)
const pinned = ref<HTMLElement | null>(null)
const pinnedHeight = ref(0)
const pageHeight = ref(0)
const resize = new ResizeObserver(() => {
  pinnedHeight.value = pinned.value?.offsetHeight ?? 0
  pageHeight.value = page.value?.clientHeight ?? 0
  measureTabs()
})
watch([page, pinned], (now, before) => {
  for (const el of before ?? []) if (el) resize.unobserve(el)
  for (const el of now) if (el) resize.observe(el)
})
onBeforeUnmount(() => resize.disconnect())
// 24px: the bottom padding of the tab content
const stickyVars = computed(() => ({ '--sticky-top': `${pinnedHeight.value}px`, '--sticky-room': `${Math.max(0, pageHeight.value - pinnedHeight.value - 24)}px` }))

// The tab row scrolls sideways without a scrollbar: the wheel scrolls it, and the chosen tab is brought into view.
const tabRow = ref<HTMLElement | null>(null)
/** Which ends of the tab row hide tabs; those ends fade out. */
const tabsHidden = ref({ left: false, right: false })
function measureTabs() {
  const row = tabRow.value
  if (row) tabsHidden.value = { left: row.scrollLeft > 1, right: row.scrollLeft + row.clientWidth < row.scrollWidth - 1 }
}
function wheelTabs(e: WheelEvent) {
  const row = tabRow.value
  if (!row || row.scrollWidth <= row.clientWidth || e.deltaX) return
  e.preventDefault()
  row.scrollLeft += e.deltaY
}
watch(tab, () => void nextTick(() => tabRow.value?.querySelector('[aria-selected="true"]')?.scrollIntoView({ block: 'nearest', inline: 'nearest' })))

// Files dropped on a content tab go into its folder; elsewhere on the page each file's type is identified.
const listKey = ref(0)
const { dragging } = useFileDrop(async (paths) => {
  if (!instance.value) return
  if (await addFilesToInstance(instance.value.id, kindOfTab(tab.value), paths)) listKey.value++
})
</script>

<template>
  <div v-if="instance && steam" ref="page" class="h-full min-h-0 overflow-y-auto" :style="stickyVars">
    <SteamHero :instance="instance" />
    <!-- at least one screen tall, so the art can scroll away completely and the play bar stays pinned at the top;
         the logs tab takes exactly one screen, so its log box fills the window instead of growing past it -->
    <div class="flex flex-col" :class="tab === 'logs' ? 'h-full' : 'min-h-full'">
      <!-- the play bar and the tabs stay pinned at the top while the content scrolls -->
      <div ref="pinned" data-no-marquee class="sticky top-0 z-[45] shrink-0 bg-base pb-3 shadow-[0_8px_12px_-8px_var(--c-base)]">
        <SteamPlayBar :instance="instance" @export="openExportMenu" @update="showPackUpdate = true" @folder="tasks.guard(() => openInstanceFolder(instance!.id))" />
        <!-- one row that scrolls sideways in a narrow window, so the pinned block keeps its height -->
        <nav
          ref="tabRow"
          class="no-scrollbar mx-6 flex overflow-x-auto rounded-sm bg-surface"
          :class="{ 'fade-left': tabsHidden.left, 'fade-right': tabsHidden.right }"
          role="tablist"
          @wheel="wheelTabs"
          @scroll="measureTabs"
        >
          <button
            v-for="name in INSTANCE_TABS"
            :key="name"
            role="tab"
            :data-tour="tourAnchor(name)"
            :aria-selected="tab === name"
            class="steam-tab flex h-11 shrink-0 items-center gap-1.5 px-2.5 text-[13px] transition-colors"
            :class="tab === name ? 'steam-tab-active text-fg' : 'text-muted hover:bg-hover hover:text-fg'"
            @click="tab = name"
          >
            <UiIcon :name="`tab-${name}`" :icon="INSTANCE_TAB_ICONS[name]" :size="15" class="text-icon" />
            {{ t.instance.tabs[name] }}
          </button>
        </nav>
      </div>
      <section class="flex min-h-0 flex-1 flex-col px-6 pt-2 pb-6">
        <h2 class="section-title mb-3 shrink-0">{{ t.instance.tabs[tab] }}</h2>
        <InstanceTabContent :instance="instance" :tab="tab" :list-key="listKey" :fill="tab === 'logs'" />
      </section>
    </div>
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
          :data-tour="tourAnchor(name)"
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
    <CoachMarks />
    <DropOverlay v-if="dragging" :text="t.transfer.dropHere" />
  </template>
</template>

<style scoped>
.steam-tab {
  border-bottom: 2px solid transparent;
}
.no-scrollbar {
  scrollbar-width: none;
}
.no-scrollbar::-webkit-scrollbar {
  display: none;
}
/* tabs past an edge of the row fade out there, so the row reads as scrollable */
.fade-right {
  mask-image: linear-gradient(to right, black calc(100% - 48px), transparent);
}
.fade-left {
  mask-image: linear-gradient(to left, black calc(100% - 48px), transparent);
}
.fade-left.fade-right {
  mask-image: linear-gradient(to right, transparent, black 48px, black calc(100% - 48px), transparent);
}
.steam-tab-active {
  border-bottom-color: var(--c-accent);
  background: color-mix(in srgb, var(--c-accent) 10%, transparent);
}
</style>
