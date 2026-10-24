<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { Check, ChevronDown, Compass, FileUp, FolderOpen, Library, Plus, Search, Settings, User } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { isTauri, openDataDir, openSharedFolder } from '../api/commands'
import mascot from '../assets/mascot.png'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useLayoutStore } from '../stores/layout'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'
import CreateInstanceDialog from './CreateInstanceDialog.vue'
import GripHandle from './GripHandle.vue'

const store = useInstancesStore()
const settings = useSettingsStore()
const tasks = useTasksStore()
const layout = useLayoutStore()
const route = useRoute()
const router = useRouter()

const showCreate = ref(false)
const folderMenu = ref<{ x: number; y: number } | null>(null)
const accountsOpen = ref(false)
const onLibrary = computed(() => route.name === 'library')

const folderItems: MenuItem[] = [
  { label: t.toolbar.dataFolder, icon: FolderOpen, action: () => void tasks.guard(openDataDir) },
  ...(['resourcepacks', 'shaderpacks', 'screenshots'] as const).map((folder, i) => ({
    label: t.toolbar.sharedFolder(t.sharedFolder.names[folder]),
    icon: FolderOpen,
    separatorBefore: i === 0,
    action: () => void tasks.guard(() => openSharedFolder(folder)),
  })),
]
/** Menus open away from the window edge the toolbar is docked to. */
function menuBelow(e: MouseEvent) {
  const box = (e.currentTarget as HTMLElement).getBoundingClientRect()
  return { x: box.left, y: layout.toolbar === 'top' ? box.bottom + 4 : box.top - folderItems.length * 34 - 16 }
}

async function importPack() {
  const path = isTauri
    ? await open({ multiple: false, filters: [{ name: t.transfer.importFilter, extensions: ['mrpack', 'zip'] }] })
    : 'C:\\Downloads\\Example Pack.mrpack'
  if (typeof path !== 'string') return
  const imported = await tasks.guard(() => store.importModpack(path))
  if (imported) layout.selectedId = imported.id
}

const accounts = computed(() => settings.settings?.accounts ?? [])
const current = computed(() => accounts.value.find((a) => a.id === settings.settings?.selectedAccountId))
async function pickAccount(id: string) {
  accountsOpen.value = false
  await tasks.guard(() => settings.selectAccount(id))
}
</script>

<template>
  <div class="relative z-30 flex h-11 shrink-0 items-center gap-0.5 bg-surface px-1.5" :class="layout.toolbar === 'top' ? 'border-b border-line' : 'border-t border-line'">
    <GripHandle axis="y" class="mx-1" @dock="layout.toolbar = $event === 'start' ? 'top' : 'bottom'" />
    <button class="tool" :class="onLibrary ? 'tool-active' : ''" @click="router.push('/')"><Library :size="16" class="text-muted" />{{ t.nav.library }}</button>
    <div class="mx-1 h-5 w-px bg-line" />
    <button class="tool" @click="showCreate = true"><Plus :size="17" class="text-ok" />{{ t.toolbar.add }}</button>
    <button class="tool" @click="importPack"><FileUp :size="16" class="text-accent" />{{ t.library.import }}</button>
    <button class="tool" @click="folderMenu = menuBelow($event)"><FolderOpen :size="16" class="text-warn" />{{ t.toolbar.folders }}<ChevronDown :size="13" class="text-faint" /></button>
    <button class="tool" :class="route.name === 'browse' ? 'tool-active' : ''" @click="router.push('/browse')"><Compass :size="16" class="text-accent" />{{ t.nav.browse }}</button>
    <button class="tool" :class="route.name === 'settings' ? 'tool-active' : ''" @click="router.push('/settings')"><Settings :size="16" class="text-muted" />{{ t.nav.settings }}</button>

    <div class="flex-1" />
    <div v-if="onLibrary" class="relative mr-1 w-52">
      <Search :size="14" class="pointer-events-none absolute top-2 left-2.5 text-faint" />
      <input v-model="layout.search" class="input h-7.5 !pl-8" :placeholder="t.library.searchPlaceholder" :aria-label="t.common.search" />
    </div>
    <img :src="mascot" alt="" class="pixelated anim-bob mx-1 size-7" />

    <div class="relative">
      <button class="tool" :aria-label="t.account.switch" :aria-expanded="accountsOpen" @click="accountsOpen = !accountsOpen">
        <span class="pix flex size-6 items-center justify-center bg-accent/20 text-accent"><User :size="14" /></span>
        <span class="max-w-36 truncate">{{ current?.name ?? t.account.none }}</span>
        <ChevronDown :size="13" class="text-faint" />
      </button>
      <template v-if="accountsOpen">
        <div class="fixed inset-0 z-40" @mousedown="accountsOpen = false" />
        <div class="card anim-pop absolute right-0 z-50 w-56 p-1 shadow-[var(--shadow-pop)]" :class="layout.toolbar === 'top' ? 'top-[calc(100%+6px)]' : 'bottom-[calc(100%+6px)]'">
          <button v-for="acc in accounts" :key="acc.id" class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-hover" @click="pickAccount(acc.id)">
            <span class="flex-1 truncate">{{ acc.name }}</span>
            <span class="text-[11px] text-faint">{{ t.account.kinds[acc.kind] }}</span>
            <Check v-if="acc.id === current?.id" :size="14" class="text-accent" />
          </button>
          <div v-if="accounts.length" class="my-1 h-px bg-line" />
          <RouterLink to="/settings#accounts" class="flex h-8 items-center rounded-md px-2 text-faint hover:bg-hover" @click="accountsOpen = false">{{ t.account.add }}</RouterLink>
        </div>
      </template>
    </div>

    <ContextMenu v-if="folderMenu" :x="folderMenu.x" :y="folderMenu.y" :items="folderItems" @close="folderMenu = null" />
    <CreateInstanceDialog v-if="showCreate" @close="showCreate = false" />
  </div>
</template>
