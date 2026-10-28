<script lang="ts">
import { Camera, Globe, Image, Layers, Puzzle, ScrollText, Server, Settings2, SlidersHorizontal, Sparkles } from '@lucide/vue'

export type InstanceTab = 'version' | 'mods' | 'resourcepacks' | 'shaders' | 'worlds' | 'screenshots' | 'servers' | 'options' | 'logs' | 'settings'
export const INSTANCE_TABS: InstanceTab[] = ['version', 'mods', 'resourcepacks', 'shaders', 'worlds', 'screenshots', 'servers', 'options', 'logs', 'settings']
export const INSTANCE_TAB_ICONS = {
  version: Layers,
  mods: Puzzle,
  resourcepacks: Image,
  shaders: Sparkles,
  worlds: Globe,
  screenshots: Camera,
  servers: Server,
  options: SlidersHorizontal,
  logs: ScrollText,
  settings: Settings2,
} as const
/** The content kind whose folder a tab lists; files dropped on that tab go there. */
export const KIND_OF_TAB = { mods: 'mod', resourcepacks: 'resourcePack', shaders: 'shaderPack' } as const
export const kindOfTab = (tab: InstanceTab) => (tab === 'mods' || tab === 'resourcepacks' || tab === 'shaders' ? KIND_OF_TAB[tab] : null)
</script>

<script setup lang="ts">
import type { Instance } from '../../api/types'
import { t } from '../../i18n'
import { clientOf } from '../../utils/format'
import GameOptionsTab from './GameOptionsTab.vue'
import InstanceSettingsTab from './InstanceSettingsTab.vue'
import LogsTab from './LogsTab.vue'
import ResourceList from './ResourceList.vue'
import ScreenshotsTab from './ScreenshotsTab.vue'
import ServersTab from './ServersTab.vue'
import VersionTab from './VersionTab.vue'
import WorldsTab from './WorldsTab.vue'

/** `listKey` changes when files were added from outside, so the lists reload. */
const props = defineProps<{ instance: Instance; tab: InstanceTab; listKey: number }>()

const sharedFolderOfTab = { resourcepacks: 'resourcepacks', shaders: 'shaderpacks' } as const
/** The notice shown above a resource list whose folder is linked to the shared one. */
function sharedNote(tab: InstanceTab) {
  if (tab !== 'resourcepacks' && tab !== 'shaders') return undefined
  const folder = sharedFolderOfTab[tab]
  return props.instance.sharedFolders.includes(folder) ? t.sharedFolder.notes[folder] : undefined
}
</script>

<template>
  <p v-if="tab === 'mods' && clientOf(instance)" class="mb-3 rounded-lg border border-line bg-raised px-3 py-2 text-[12.5px] text-muted">
    {{ t.client.modsNote(clientOf(instance)!.label) }}
  </p>
  <ResourceList
    v-if="tab === 'mods' || tab === 'resourcepacks' || tab === 'shaders'"
    :key="`${instance.id}-${tab}-${listKey}`"
    :instance-id="instance.id"
    :kind="KIND_OF_TAB[tab]"
    :shared-note="sharedNote(tab)"
  />
  <VersionTab v-else-if="tab === 'version'" :instance="instance" />
  <WorldsTab v-else-if="tab === 'worlds'" :key="`${instance.id}-${listKey}`" :instance-id="instance.id" />
  <ScreenshotsTab v-else-if="tab === 'screenshots'" :key="instance.id" :instance-id="instance.id" :shared="instance.sharedFolders.includes('screenshots')" />
  <ServersTab v-else-if="tab === 'servers'" :key="instance.id" :instance-id="instance.id" />
  <GameOptionsTab v-else-if="tab === 'options'" :key="instance.id" :instance-id="instance.id" />
  <LogsTab v-else-if="tab === 'logs'" :instance-id="instance.id" />
  <InstanceSettingsTab v-else :key="instance.id" :instance="instance" />
</template>
