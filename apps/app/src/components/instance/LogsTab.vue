<script setup lang="ts">
import { Copy, Trash2 } from '@lucide/vue'
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import type { GameLogLine } from '../../api/types'
import { t } from '../../i18n/ru'
import { useInstancesStore } from '../../stores/instances'
import { useTasksStore } from '../../stores/tasks'
import { errorMessage } from '../../utils/format'
import ToggleSwitch from '../ToggleSwitch.vue'

const props = defineProps<{ instanceId: string }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const lines = computed(() => store.logOf(props.instanceId))
const autoscroll = ref(true)
const box = ref<HTMLElement | null>(null)

function levelClass(l: GameLogLine): string {
  if (l.stream === 'launcher') return 'text-accent'
  if (/\/(ERROR|FATAL)\]/.test(l.line) || l.stream === 'stderr') return 'text-danger'
  if (/\/WARN\]/.test(l.line)) return 'text-warn'
  if (/\/(DEBUG|TRACE)\]/.test(l.line)) return 'text-faint'
  return 'text-fg'
}

function scrollToEnd() {
  void nextTick(() => {
    if (autoscroll.value && box.value) box.value.scrollTop = box.value.scrollHeight
  })
}
watch(() => lines.value.length, scrollToEnd)
onMounted(scrollToEnd)

/** Scrolling away from the bottom pauses autoscroll, like a terminal. */
function onScroll() {
  const el = box.value
  if (el) autoscroll.value = el.scrollHeight - el.scrollTop - el.clientHeight < 24
}

async function copy() {
  try {
    await navigator.clipboard.writeText(lines.value.map((l) => l.line).join('\n'))
    tasks.toast(t.instance.copied, 'ok')
  } catch (e) {
    tasks.toast(errorMessage(e), 'error')
  }
}
</script>

<template>
  <div class="flex h-[calc(100vh-250px)] min-h-72 flex-col gap-2">
    <div class="flex items-center gap-3">
      <label class="mr-auto flex items-center gap-2 text-muted">
        <ToggleSwitch v-model="autoscroll" :label="t.instance.autoscroll" @update:model-value="scrollToEnd" />
        {{ t.instance.autoscroll }}
      </label>
      <button class="btn" :disabled="!lines.length" @click="copy"><Copy :size="15" />{{ t.common.copy }}</button>
      <button class="btn" :disabled="!lines.length" @click="store.clearLog(instanceId)"><Trash2 :size="15" />{{ t.common.clear }}</button>
    </div>
    <div ref="box" class="selectable min-h-0 flex-1 overflow-y-auto rounded-xl border border-line bg-base p-3 font-mono text-[12px] leading-5" role="log" @scroll="onScroll">
      <p v-if="!lines.length" class="text-faint">{{ t.instance.logsEmpty }}</p>
      <div v-for="(l, i) in lines" :key="i" class="break-all whitespace-pre-wrap" :class="levelClass(l)">{{ l.line }}</div>
    </div>
  </div>
</template>
