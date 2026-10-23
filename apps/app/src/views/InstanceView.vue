<script setup lang="ts">
import { ArrowLeft, Clock, FolderOpen, History } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { openInstanceFolder } from '../api/commands'
import InstanceIcon from '../components/InstanceIcon.vue'
import InstanceSettingsTab from '../components/instance/InstanceSettingsTab.vue'
import LogsTab from '../components/instance/LogsTab.vue'
import ResourceList from '../components/instance/ResourceList.vue'
import WorldsTab from '../components/instance/WorldsTab.vue'
import PlayButton from '../components/PlayButton.vue'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useTasksStore } from '../stores/tasks'
import { formatPlaytime, formatRelative, loaderOf, minecraftVersionOf } from '../utils/format'

const props = defineProps<{ id: string }>()
const store = useInstancesStore()
const tasks = useTasksStore()
const router = useRouter()

const instance = computed(() => store.byId(props.id))
const loader = computed(() => (instance.value ? loaderOf(instance.value) : undefined))

type Tab = 'mods' | 'resourcepacks' | 'shaders' | 'worlds' | 'logs' | 'settings'
const tab = ref<Tab>('mods')
const tabs: Tab[] = ['mods', 'resourcepacks', 'shaders', 'worlds', 'logs', 'settings']
const kindOfTab = { mods: 'mod', resourcepacks: 'resourcepack', shaders: 'shader' } as const
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
      <button class="btn btn-icon" :title="t.library.openFolder" :aria-label="t.library.openFolder" @click="tasks.guard(() => openInstanceFolder(instance!.id))"><FolderOpen :size="16" /></button>
      <PlayButton :instance-id="instance.id" large />
    </header>

    <div class="flex gap-1 border-b border-line" role="tablist">
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
      :key="`${instance.id}-${tab}`"
      :instance-id="instance.id"
      :kind="kindOfTab[tab]"
    />
    <WorldsTab v-else-if="tab === 'worlds'" :key="instance.id" :instance-id="instance.id" />
    <LogsTab v-else-if="tab === 'logs'" :instance-id="instance.id" />
    <InstanceSettingsTab v-else :key="instance.id" :instance="instance" />
  </div>
  <p v-else-if="store.loaded" class="p-10 text-center text-faint">{{ t.instance.notFound }}</p>
</template>
