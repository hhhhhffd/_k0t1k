<script setup lang="ts">
import { FolderOpen, Trash2 } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import * as api from '../../api/commands'
import type { Screenshot } from '../../api/types'
import { t } from '../../i18n'
import { useTasksStore } from '../../stores/tasks'
import { useMarquee } from '../../utils/useMarquee'
import ConfirmDialog from '../ConfirmDialog.vue'
import MarqueeBox from '../MarqueeBox.vue'
import ScreenshotViewer from './ScreenshotViewer.vue'

const props = defineProps<{ instanceId: string; shared: boolean }>()
const tasks = useTasksStore()

const shots = ref<Screenshot[]>([])
const loading = ref(true)
const viewing = ref<number | null>(null)
const deleting = ref<Screenshot[]>([])
const selected = ref(new Set<string>())
const { box, onPointerDown } = useMarquee(selected)
const selectedShots = computed(() => shots.value.filter((s) => selected.value.has(s.fileName)))

/** A plain click opens the viewer; with Ctrl/Shift, or once something is selected, it selects instead. */
function onShotClick(index: number, e: MouseEvent) {
  const name = shots.value[index]!.fileName
  if (!(e.ctrlKey || e.metaKey || e.shiftKey || selected.value.size)) {
    viewing.value = index
    return
  }
  const next = new Set(e.ctrlKey || e.metaKey || e.shiftKey ? selected.value : [])
  if (e.shiftKey && anchor !== null) {
    const [from, to] = [anchor, index].sort((a, b) => a - b)
    shots.value.slice(from!, to! + 1).forEach((s) => next.add(s.fileName))
  } else if (next.has(name) && (e.ctrlKey || e.metaKey)) {
    next.delete(name)
  } else {
    next.add(name)
  }
  if (!e.shiftKey) anchor = index
  selected.value = next
}
let anchor: number | null = null
function onKey(e: KeyboardEvent) {
  if (e.key === 'a' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault()
    selected.value = new Set(shots.value.map((s) => s.fileName))
  } else if (e.key === 'Delete' && selectedShots.value.length) {
    deleting.value = selectedShots.value
  } else if (e.key === 'Escape') {
    selected.value = new Set()
  }
}

async function load() {
  loading.value = true
  shots.value = (await tasks.guard(() => api.listScreenshots(props.instanceId))) ?? []
  selected.value = new Set([...selected.value].filter((name) => shots.value.some((s) => s.fileName === name)))
  loading.value = false
}
onMounted(load)
watch(() => props.instanceId, load)

const openFolder = () => tasks.guard(() => api.openScreenshotsFolder(props.instanceId))
const openShot = (shot: Screenshot) => tasks.guard(() => api.openScreenshot(props.instanceId, shot.fileName))

async function confirmDelete() {
  const doomed = deleting.value
  deleting.value = []
  if (!doomed.length) return
  const done = await tasks.guard(async () => {
    for (const shot of doomed) await api.deleteScreenshot(props.instanceId, shot.fileName)
    return true
  })
  if (!done) return
  await load()
  if (viewing.value !== null) viewing.value = shots.value.length ? Math.min(viewing.value, shots.value.length - 1) : null
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <p v-if="shared" class="flex items-center gap-2 text-muted">
      <span class="badge badge-accent">{{ t.sharedFolder.badge }}</span>{{ t.shots.sharedNote }}
    </p>
    <div class="flex items-center gap-2">
      <span class="mr-auto text-faint">{{ t.shots.count(shots.length) }}</span>
      <button v-if="selectedShots.length" class="btn btn-danger" @click="deleting = selectedShots">
        <Trash2 :size="15" />{{ t.common.delete }} ({{ selectedShots.length }})
      </button>
      <button class="btn" @click="openFolder"><FolderOpen :size="15" />{{ t.shots.openFolder }}</button>
    </div>

    <div class="flex min-h-[40vh] flex-col outline-none" tabindex="-1" @pointerdown="onPointerDown" @dragstart.prevent @keydown="onKey">
      <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
      <p v-else-if="!shots.length" class="py-10 text-center text-faint">{{ t.shots.empty }}</p>
      <ul v-else class="grid grid-cols-[repeat(auto-fill,minmax(clamp(190px,15vw,340px),1fr))] gap-2.5">
        <li v-for="(s, i) in shots" :key="s.fileName">
          <div
            role="button"
            tabindex="0"
            :data-select-key="s.fileName"
            class="card group block w-full cursor-pointer overflow-hidden text-left transition-colors hover:border-accent/60"
            :class="selected.has(s.fileName) ? '!border-accent !bg-accent/15' : ''"
            @click="onShotClick(i, $event)"
            @keydown.enter.self="viewing = i"
          >
            <img :src="api.fileUrl(s.path)" :alt="s.fileName" loading="lazy" decoding="async" class="aspect-video w-full bg-raised object-cover" />
            <span class="block truncate px-2 py-1.5 text-[12px] text-muted">{{ s.fileName }}</span>
          </div>
        </li>
      </ul>
    </div>
    <MarqueeBox :box="box" />

    <ScreenshotViewer
      v-if="viewing !== null"
      :index="viewing"
      @update:index="viewing = $event"
      :shots="shots"
      @open="openShot"
      @delete="deleting = [$event]"
      @close="viewing = null"
    />

    <ConfirmDialog
      v-if="deleting.length"
      :title="deleting.length > 1 ? t.shots.deleteMany(deleting.length) : t.shots.delete"
      :text="deleting.map((s) => s.fileName).join(', ')"
      @confirm="confirmDelete"
      @close="deleting = []"
    />
  </div>
</template>
