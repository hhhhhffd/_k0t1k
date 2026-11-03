<script setup lang="ts">
import { Loader, Minus, Plus, Search } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import type { Instance } from '../api/types'
import mascot from '../assets/mascot.png'
import ContextMenu from '../components/ContextMenu.vue'
import CreateInstanceDialog from '../components/CreateInstanceDialog.vue'
import GettingStarted from '../components/GettingStarted.vue'
import GetStarted from '../components/GetStarted.vue'
import InstanceActionDialogs from '../components/InstanceActionDialogs.vue'
import InstanceIcon from '../components/InstanceIcon.vue'
import UiIcon from '../components/UiIcon.vue'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useLayoutStore } from '../stores/layout'
import { useInstanceActions } from '../utils/useInstanceActions'
import InstanceView from './InstanceView.vue'

/** The library in the Steam-like mode: instances listed by group on the left, the chosen one's page on the right. */
const store = useInstancesStore()
const layout = useLayoutStore()
const route = useRoute()
const router = useRouter()

const actions = useInstanceActions()
const { launch, rememberPointer, actionsFor } = actions
const menu = ref<{ x: number; y: number; instance: Instance } | null>(null)
const menuItems = computed(() => (menu.value ? actionsFor(menu.value.instance) : []))
const showCreate = ref(false)
const collapsed = ref(new Set<string>())

/** The instance route picks the page; the plain library shows the last selected instance. */
const currentId = computed(() => (route.name === 'instance' ? String(route.params.id) : layout.selectedId))
const highlightAdd = computed(() => store.loaded && !store.instances.length)
const current = computed(() => (currentId.value ? store.byId(currentId.value) : undefined))

const UNGROUPED = ''
const sections = computed(() => {
  const needle = layout.search.trim().toLowerCase()
  const map = new Map<string, Instance[]>()
  for (const inst of store.instances) {
    if (needle && !inst.name.toLowerCase().includes(needle)) continue
    const key = inst.group ?? UNGROUPED
    map.set(key, [...(map.get(key) ?? []), inst])
  }
  for (const items of map.values()) items.sort((a, b) => a.name.localeCompare(b.name, 'ru'))
  return [...map.entries()].sort(([a], [b]) => (a === UNGROUPED ? 1 : b === UNGROUPED ? -1 : a.localeCompare(b, 'ru')))
})

function toggle(group: string) {
  const next = new Set(collapsed.value)
  if (!next.delete(group)) next.add(group)
  collapsed.value = next
}
const open = (inst: Instance) => router.push({ name: 'instance', params: { id: inst.id } })
function openMenu(e: MouseEvent, inst: Instance) {
  menu.value = { x: e.clientX, y: e.clientY, instance: inst }
}
</script>

<template>
  <div class="flex h-full min-h-0" @pointerdown.capture="rememberPointer">
    <aside class="flex w-64 shrink-0 flex-col border-r border-line bg-surface">
      <div class="relative p-2">
        <Search :size="14" class="pointer-events-none absolute top-4.5 left-4.5 text-faint" />
        <input v-model="layout.search" class="input h-8 !pl-8" :placeholder="t.library.searchPlaceholder" :aria-label="t.common.search" />
      </div>
      <nav class="min-h-0 flex-1 overflow-y-auto pb-2">
        <section v-for="[group, items] in sections" :key="group">
          <button class="flex h-7 w-full items-center gap-1.5 px-2.5 text-[11.5px] font-bold tracking-wide text-muted uppercase hover:text-fg" :aria-expanded="!collapsed.has(group)" @click="toggle(group)">
            <component :is="collapsed.has(group) ? Plus : Minus" :size="12" />
            <span class="truncate">{{ group || t.library.ungrouped }}</span>
            <span class="font-normal text-faint">({{ items.length }})</span>
          </button>
          <template v-if="!collapsed.has(group)">
            <button
              v-for="inst in items"
              :key="inst.id"
              class="flex h-7 w-full items-center gap-2 pr-2 pl-5 text-left text-[13px]"
              :class="inst.id === current?.id ? 'bg-accent/20 text-fg' : 'text-muted hover:bg-hover hover:text-fg'"
              :aria-current="inst.id === current?.id ? 'page' : undefined"
              @click="open(inst)"
              @dblclick="launch(inst)"
              @contextmenu.prevent="openMenu($event, inst)"
            >
              <InstanceIcon :icon="inst.icon" :instance-id="inst.id" :size="18" />
              <span class="min-w-0 flex-1 truncate" :class="store.stateOf(inst.id) !== 'stopped' ? 'font-semibold text-ok' : ''">{{ inst.name }}</span>
            </button>
          </template>
        </section>
        <div v-for="p in store.pending" :key="`pending-${p.key}`" class="flex h-7 items-center gap-2 pl-5 text-[13px] text-faint" aria-busy="true">
          <Loader :size="14" class="animate-spin text-accent" /><span class="truncate">{{ p.name }}</span>
        </div>
        <p v-if="store.loaded && store.instances.length && !sections.length" class="px-3 py-6 text-center text-faint">{{ t.common.nothingFound }}</p>
      </nav>
      <GettingStarted />
      <button
        class="relative flex h-10 shrink-0 items-center gap-2 border-t border-line px-3 text-[13px]"
        :class="highlightAdd ? 'bg-accent font-semibold text-accent-fg hover:brightness-110' : 'text-muted hover:bg-hover hover:text-fg'"
        @click="showCreate = true"
      >
        <span v-if="highlightAdd" class="pointer-events-none absolute inset-0 animate-pulse ring-2 ring-inset ring-white/60" aria-hidden="true" />
        <UiIcon name="add" :icon="Plus" :size="16" :class="highlightAdd ? 'text-accent-fg' : 'text-icon'" />{{ t.toolbar.add }}
      </button>
    </aside>

    <div class="min-w-0 flex-1">
      <InstanceView v-if="current" :id="current.id" />
      <div v-else-if="store.loaded" class="flex h-full flex-col items-center gap-2 overflow-y-auto px-6 py-4 text-center" :class="store.instances.length ? 'justify-center' : ''">
        <img :src="mascot" alt="" class="pixelated anim-bob shrink-0" :class="store.instances.length ? 'size-24' : 'size-16'" />
        <p class="text-[16px] font-semibold">{{ store.instances.length ? t.steam.pick : t.library.empty }}</p>
        <template v-if="!store.instances.length && !store.pending.length">
          <p class="text-muted">{{ t.library.emptyHint }}</p>
          <GetStarted />
        </template>
      </div>
    </div>

    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :items="menuItems" @close="menu = null" />
    <InstanceActionDialogs :actions="actions" />
    <CreateInstanceDialog v-if="showCreate" @close="showCreate = false" />
  </div>
</template>
