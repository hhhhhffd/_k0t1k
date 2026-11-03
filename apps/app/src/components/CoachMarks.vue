<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { t } from '../i18n'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'

/** The hints shown once, the first time an instance page opens: they point at elements marked `data-tour`. */
const settings = useSettingsStore()
const tasks = useTasksStore()

const STEPS = [
  { anchor: 'play', text: () => t.tour.play },
  { anchor: 'mods', text: () => t.tour.mods },
  { anchor: 'settings', text: () => t.tour.settings },
]
const WIDTH = 288

const index = ref(0)
const finished = ref(false)
const target = ref<DOMRect | null>(null)
const blocked = ref(false)

const anchorOf = (i: number) => document.querySelector<HTMLElement>(`[data-tour="${STEPS[i].anchor}"]`)
/** Steps whose element is missing are skipped. */
function firstAvailable(from: number) {
  let i = from
  while (i < STEPS.length && !anchorOf(i)) i++
  return i
}

function measure() {
  // a dialog on top hides the hints until it is closed
  blocked.value = !!document.querySelector('[aria-modal="true"]')
  const next = firstAvailable(index.value)
  if (next >= STEPS.length) {
    void finish()
    return
  }
  if (next !== index.value) index.value = next
  target.value = anchorOf(index.value)!.getBoundingClientRect()
}

async function finish() {
  if (finished.value) return
  finished.value = true
  await tasks.guard(() => settings.markOnboarding('instanceTour'))
}
function next() {
  index.value = firstAvailable(index.value + 1)
  if (index.value >= STEPS.length) void finish()
  else {
    anchorOf(index.value)?.scrollIntoView({ block: 'nearest', inline: 'nearest' })
    measure()
  }
}

const active = computed(() => !!settings.settings && !settings.settings.onboarding.instanceTourDone && !finished.value)
const isLast = computed(() => firstAvailable(index.value + 1) >= STEPS.length)
const bubble = computed(() => {
  const r = target.value
  if (!r) return {}
  const left = Math.min(Math.max(8, r.left + r.width / 2 - WIDTH / 2), window.innerWidth - WIDTH - 8)
  // below the element, or above when there is no room
  const below = r.bottom + 160 < window.innerHeight
  return below ? { left: `${left}px`, top: `${r.bottom + 10}px` } : { left: `${left}px`, bottom: `${window.innerHeight - r.top + 10}px` }
})

let timer: number | undefined
onMounted(() => {
  window.addEventListener('resize', measure)
  window.addEventListener('scroll', measure, true)
  // the anchors appear after the page has rendered, and dialogs open without any event of ours
  timer = window.setInterval(() => active.value && measure(), 300)
})
onBeforeUnmount(() => {
  window.removeEventListener('resize', measure)
  window.removeEventListener('scroll', measure, true)
  window.clearInterval(timer)
})
</script>

<template>
  <Teleport to="body">
    <div v-if="active && target && !blocked" class="pointer-events-none fixed inset-0 z-[48]">
      <div
        class="absolute rounded-md ring-2 ring-accent shadow-[0_0_0_4px_color-mix(in_srgb,var(--c-accent)_30%,transparent)]"
        :style="{ left: `${target.left - 3}px`, top: `${target.top - 3}px`, width: `${target.width + 6}px`, height: `${target.height + 6}px` }"
      />
      <div class="card anim-pop pointer-events-auto absolute flex flex-col gap-2 p-3 shadow-[var(--shadow-pop)]" :style="{ ...bubble, width: `${WIDTH}px` }" role="status">
        <p class="text-[13px]">{{ STEPS[index].text() }}</p>
        <div class="flex items-center gap-2">
          <span class="text-[12px] text-faint">{{ t.tour.stepOf(index + 1, STEPS.length) }}</span>
          <button class="btn btn-ghost ml-auto" @click="finish">{{ t.tour.skip }}</button>
          <button class="btn btn-primary" @click="isLast ? finish() : next()">{{ isLast ? t.tour.done : t.tour.next }}</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>
