<script setup lang="ts">
import { Loader, Play, Square } from '@lucide/vue'
import { computed } from 'vue'
import { t } from '../i18n/ru'
import { useInstancesStore } from '../stores/instances'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'

const props = defineProps<{ instanceId: string; large?: boolean }>()
const store = useInstancesStore()
const settings = useSettingsStore()
const tasks = useTasksStore()

const state = computed(() => store.stateOf(props.instanceId))

function toggle() {
  if (state.value === 'stopped') {
    void tasks.guard(() => store.launch(props.instanceId, settings.settings?.selectedAccountId))
  } else {
    void tasks.guard(() => store.kill(props.instanceId))
  }
}
</script>

<template>
  <button
    class="btn"
    :class="[state === 'stopped' ? 'btn-primary pix' : 'btn-danger', large ? 'btn-lg min-w-36' : 'min-w-24']"
    :disabled="state === 'stopping'"
    @click.stop="toggle"
  >
    <Play v-if="state === 'stopped'" :size="large ? 18 : 15" fill="currentColor" />
    <Loader v-else-if="state === 'stopping'" :size="16" class="animate-spin" />
    <Square v-else :size="14" fill="currentColor" />
    {{ state === 'stopped' ? t.library.play : state === 'preparing' ? t.library.preparing : state === 'stopping' ? t.library.stopping : t.library.stop }}
  </button>
</template>
