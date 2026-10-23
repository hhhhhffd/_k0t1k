<script setup lang="ts">
import { Copy, Trash2 } from '@lucide/vue'
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import type { GameLogLine, SessionId } from '../../api/types'
import { t } from '../../i18n/ru'
import { MAX_LOG_LINES, useInstancesStore } from '../../stores/instances'
import { useTasksStore } from '../../stores/tasks'
import { errorMessage } from '../../utils/format'
import ToggleSwitch from '../ToggleSwitch.vue'

const props = defineProps<{ instanceId: string }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const sessions = computed(() => store.sessionsOf(props.instanceId))
// null follows the newest session, so launching another copy switches to its log
const picked = ref<SessionId | null>(null)
const selected = computed(() => {
  const list = sessions.value
  return list.find((s) => s.sessionId === picked.value) ?? list[list.length - 1]
})
const buffer = computed(() => (selected.value ? store.logOf(selected.value.sessionId) : { lines: [], dropped: 0 }))
const lines = computed(() => buffer.value.lines)
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
watch(
  () => sessions.value[sessions.value.length - 1]?.sessionId,
  () => {
    picked.value = null
  },
)
watch(selected, scrollToEnd)
// the buffer is replaced on every flush, and its length stops changing once the cap is reached
watch(lines, scrollToEnd)
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
    <div v-if="sessions.length > 1" class="flex flex-wrap gap-1" role="tablist" :aria-label="t.instance.sessionsLabel">
      <button
        v-for="s in sessions"
        :key="s.sessionId"
        role="tab"
        :aria-selected="selected?.sessionId === s.sessionId"
        class="btn h-7 gap-1.5 px-2.5 text-[12px]"
        :class="selected?.sessionId === s.sessionId ? 'btn-primary' : ''"
        @click="picked = s.sessionId"
      >
        <span v-if="s.state !== 'stopped'" class="size-1.5 rounded-full bg-ok" />
        {{ t.instance.session(s.number) }}
      </button>
    </div>
    <div class="flex items-center gap-3">
      <label class="mr-auto flex items-center gap-2 text-muted">
        <ToggleSwitch v-model="autoscroll" :label="t.instance.autoscroll" @update:model-value="scrollToEnd" />
        {{ t.instance.autoscroll }}
      </label>
      <button class="btn" :disabled="!lines.length" @click="copy"><Copy :size="15" />{{ t.common.copy }}</button>
      <button class="btn" :disabled="!lines.length" @click="selected && store.clearLog(selected.sessionId)"><Trash2 :size="15" />{{ t.common.clear }}</button>
    </div>
    <div ref="box" class="selectable min-h-0 flex-1 overflow-y-auto rounded-xl border border-line bg-base p-3 font-mono text-[12px] leading-5" role="log" @scroll="onScroll">
      <p v-if="buffer.dropped" class="mb-2 text-faint">{{ t.instance.logsTrimmed(MAX_LOG_LINES) }}</p>
      <p v-if="!lines.length" class="text-faint">{{ t.instance.logsEmpty }}</p>
      <!-- v-memo: a line never changes, so the thousands of existing rows are skipped on every flush -->
      <div v-for="l in lines" :key="l.seq" v-memo="[l.seq]" class="break-all whitespace-pre-wrap" :class="levelClass(l)">{{ l.line }}</div>
    </div>
  </div>
</template>
