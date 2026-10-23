<script setup lang="ts">
import { ChevronDown, Loader, Play, Plus, Square } from '@lucide/vue'
import { computed, ref } from 'vue'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'

const props = defineProps<{ instanceId: string; large?: boolean }>()
const store = useInstancesStore()
const settings = useSettingsStore()
const tasks = useTasksStore()

const state = computed(() => store.stateOf(props.instanceId))
const live = computed(() => store.liveSessionsOf(props.instanceId))
const stopMenu = ref<{ x: number; y: number } | null>(null)

const stopItems = computed<MenuItem[]>(() => [
  { label: t.library.stopAll, icon: Square, action: () => void tasks.guard(() => store.kill(props.instanceId)) },
  ...live.value.map((s) => ({
    label: t.library.stopSession(s.number, t.library.sessionStates[s.state as 'preparing' | 'running' | 'stopping']),
    separatorBefore: s.sessionId === live.value[0]?.sessionId,
    action: () => void tasks.guard(() => store.killSession(s.sessionId)),
  })),
])

function launch() {
  void tasks.guard(() => store.launch(props.instanceId, settings.settings?.selectedAccountId ?? undefined))
}

function openStopMenu(e: MouseEvent) {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
  stopMenu.value = { x: rect.left, y: rect.bottom + 4 }
}
</script>

<template>
  <button v-if="state === 'stopped'" class="btn btn-primary pix" :class="large ? 'btn-lg min-w-36' : 'min-w-24'" @click.stop="launch">
    <Play :size="large ? 18 : 15" fill="currentColor" />
    {{ t.library.play }}
  </button>
  <!-- the instance has live sessions: Stop acts on all of them, "+" starts one more -->
  <div v-else class="flex gap-1.5" @click.stop>
    <button
      class="btn btn-danger flex-1"
      :class="large ? 'btn-lg min-w-36' : 'min-w-24'"
      :disabled="state === 'stopping'"
      @click="tasks.guard(() => store.kill(instanceId))"
    >
      <Loader v-if="state === 'stopping'" :size="16" class="animate-spin" />
      <Square v-else :size="14" fill="currentColor" />
      {{ state === 'preparing' ? t.library.preparing : state === 'stopping' ? t.library.stopping : t.library.stop }}
      <span v-if="live.length > 1" class="badge" :title="`${live.length}`">{{ live.length }}</span>
    </button>
    <button
      v-if="live.length > 1"
      class="btn btn-icon"
      :class="large ? 'h-10 w-10' : ''"
      :title="t.library.stopAll"
      :aria-label="t.library.stopAll"
      @click="openStopMenu"
    >
      <ChevronDown :size="15" />
    </button>
    <button class="btn btn-icon" :class="large ? 'h-10 w-10' : ''" :title="t.library.playAgain" :aria-label="t.library.playAgain" @click="launch">
      <Plus :size="large ? 18 : 15" />
    </button>
    <ContextMenu v-if="stopMenu" :x="stopMenu.x" :y="stopMenu.y" :items="stopItems" @close="stopMenu = null" />
  </div>
</template>
