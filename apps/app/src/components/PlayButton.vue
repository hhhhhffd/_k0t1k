<script setup lang="ts">
import { ChevronDown, Loader, Play, Plus, Square } from '@lucide/vue'
import { computed, ref, watch } from 'vue'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'
import ContextMenu, { type MenuItem } from './ContextMenu.vue'
import NoAccountDialog from './NoAccountDialog.vue'

const props = defineProps<{ instanceId: string; large?: boolean }>()
const store = useInstancesStore()
const settings = useSettingsStore()
const tasks = useTasksStore()

const state = computed(() => store.stateOf(props.instanceId))
const live = computed(() => store.liveSessionsOf(props.instanceId))
const needAccount = ref(false)
const stopMenu = ref<{ x: number; y: number } | null>(null)

// Stopping one session keeps the menu open, so the rest can be stopped one by one; it goes away with the chevron.
const stopItems = computed<MenuItem[]>(() => [
  { label: t.library.stopAll, hint: `${live.value.length}`, icon: Square, action: () => void tasks.guard(() => store.kill(props.instanceId)) },
  ...live.value.map((s) => ({
    label: t.library.stopSession(s.number),
    hint: t.library.sessionStates[s.state as 'preparing' | 'running' | 'stopping'],
    separatorBefore: s.sessionId === live.value[0]?.sessionId,
    disabled: s.state === 'stopping',
    keepOpen: true,
    action: () => void tasks.guard(() => store.killSession(s.sessionId)),
  })),
])
watch(
  () => live.value.length,
  (n) => {
    if (n < 2) stopMenu.value = null
  },
)

function launch() {
  // Settings are loaded but nobody is signed in: ask for an account instead of failing in the backend.
  if (settings.settings && !settings.settings.selectedAccountId) {
    needAccount.value = true
    return
  }
  void tasks.guard(() => store.launch(props.instanceId, settings.settings?.selectedAccountId ?? undefined))
}

function openStopMenu(e: MouseEvent) {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect()
  stopMenu.value = { x: rect.left, y: rect.bottom + 4 }
}
</script>

<template>
  <button v-if="state === 'stopped'" data-tour="play" class="btn btn-primary pix" :class="large ? 'btn-lg min-w-36' : 'min-w-24'" @click.stop="launch">
    <Play :size="large ? 18 : 15" fill="currentColor" />
    {{ t.library.play }}
  </button>
  <!-- the instance has live sessions: Stop acts on all of them, "+" starts one more -->
  <div v-else data-tour="play" class="flex gap-1.5" @click.stop>
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
  <NoAccountDialog
    v-if="needAccount"
    @close="needAccount = false"
    @added="needAccount = false; launch()"
  />
</template>
