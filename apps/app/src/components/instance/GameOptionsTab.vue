<script setup lang="ts">
import { Crosshair, Gauge, Scale, Sparkles, Undo2 } from '@lucide/vue'
import { computed, onMounted, reactive, ref, watch } from 'vue'
import * as api from '../../api/commands'
import type { GameOption } from '../../api/types'
import { t } from '../../i18n'
import { useInstancesStore } from '../../stores/instances'
import { useTasksStore } from '../../stores/tasks'
import { minecraftVersionOf } from '../../utils/format'
import { type Change, type Field, FIELDS, PRESETS, type PresetId, type Section, copyChanges, display, fits, presetChanges, requote, unquote } from '../../utils/gameOptions'
import ModalDialog from '../ModalDialog.vue'
import SelectMenu from '../SelectMenu.vue'
import ToggleSwitch from '../ToggleSwitch.vue'

const props = defineProps<{ instanceId: string }>()
const store = useInstancesStore()
const tasks = useTasksStore()

const loaded = ref(false)
/** `null` while the game has not created options.txt yet. */
const options = ref<GameOption[] | null>(null)
/** The file as it was when the tab opened (or the game last exited), for "undo changes". */
const original = ref<GameOption[] | null>(null)
const running = computed(() => store.stateOf(props.instanceId) !== 'stopped')

async function load() {
  const read = await tasks.guard(() => api.gameOptions(props.instanceId))
  if (read === undefined) return
  options.value = read
  original.value = read && read.map((o) => ({ ...o }))
  loaded.value = true
}
onMounted(load)
// The game rewrites the file when it exits.
watch(running, (now, before) => {
  if (before && !now) void load()
})

const byKey = computed(() => new Map((options.value ?? []).map((o) => [o.key, o.value])))
const SECTIONS: Section[] = ['graphics', 'controls', 'sound', 'interface']
/** Only the fields this version's file has, in the shape it stores them. */
const sections = computed(() =>
  SECTIONS.map((section) => ({
    section,
    fields: FIELDS.filter((f) => f.section === section && byKey.value.has(f.key) && fits(f, byKey.value.get(f.key)!)),
  })).filter((s) => s.fields.length),
)
const label = (key: string) => (t.gopt.fields as Record<string, string>)[key] ?? key

const justSaved = ref(false)
let savedTimer: ReturnType<typeof setTimeout> | undefined
async function write(changes: Change[]): Promise<boolean> {
  if (!changes.length) return true
  const done = await tasks.guard(() => api.setGameOptions(props.instanceId, changes.map((c) => ({ key: c.key, value: c.to }))))
  if (done === undefined) return false
  const list = options.value ?? []
  for (const change of changes) {
    const existing = list.find((o) => o.key === change.key)
    if (existing) existing.value = change.to
    else list.push({ key: change.key, value: change.to })
  }
  options.value = list
  justSaved.value = true
  clearTimeout(savedTimer)
  savedTimer = setTimeout(() => (justSaved.value = false), 1800)
  return true
}

/** Several changes at once can be taken back from the toast. */
async function applyMany(changes: Change[]) {
  if (!(await write(changes))) return
  const undo = changes.filter((c): c is Change & { from: string } => c.from !== null).map((c) => ({ key: c.key, from: c.to, to: c.from }))
  if (undo.length) tasks.toast(t.gopt.saved, 'info', { label: t.gopt.revert, run: () => void write(undo) })
}

const setValue = (key: string, to: string) => {
  const from = byKey.value.get(key) ?? null
  if (from !== to) void write([{ key, from, to }])
}

// While a slider is dragged only its label follows; the file is written on release.
const live = reactive<Record<string, number>>({})
type RangeField = Extract<Field, { kind: 'range' }>
const shownOf = (field: RangeField) => {
  const raw = Number(byKey.value.get(field.key))
  return field.toShown ? field.toShown(raw) : raw
}
function commitRange(field: RangeField, shown: number) {
  delete live[field.key]
  const raw = field.fromShown ? field.fromShown(shown) : shown
  // floats keep a decimal point the way the game writes them (`1.0`, not `1`)
  const existing = byKey.value.get(field.key) ?? ''
  const text = existing.includes('.') ? (Number.isInteger(raw) ? raw.toFixed(1) : String(Number(raw.toFixed(4)))) : String(Math.round(raw))
  setValue(field.key, text)
}
const choiceOptions = (field: Extract<Field, { kind: 'choice' }>) => {
  const current = unquote(byKey.value.get(field.key) ?? '')
  const known = field.choices.map((c) => ({ value: c.value, label: c.label() }))
  return known.some((c) => c.value === current) ? known : [...known, { value: current, label: current }]
}

const preview = ref<{ title: string; changes: Change[]; whole: boolean } | null>(null)
const PRESET_ICONS = { fps: Gauge, balanced: Scale, quality: Sparkles, pvp: Crosshair } as const
const presetIds = Object.keys(PRESETS) as PresetId[]
function showPreset(id: PresetId) {
  if (!options.value) return
  preview.value = { title: t.gopt.previewTitle(t.gopt.presetNames[id]), changes: presetChanges(PRESETS[id], options.value), whole: false }
}

const others = computed(() =>
  store.instances.filter((i) => i.id !== props.instanceId).map((i) => ({ value: i.id, label: i.name, hint: minecraftVersionOf(i) })),
)
async function copyFrom(id: string) {
  const source = await tasks.guard(() => api.gameOptions(id))
  if (source === undefined) return
  if (!source) {
    tasks.toast(t.gopt.copyNothing, 'error')
    return
  }
  const name = store.byId(id)?.name ?? id
  preview.value = { title: t.gopt.copyTitle(name), changes: copyChanges(source, options.value), whole: !options.value }
}

async function confirmPreview() {
  const changes = preview.value?.changes ?? []
  preview.value = null
  await applyMany(changes)
}

const revertChanges = computed<Change[]>(() =>
  (original.value ?? []).flatMap((o) => {
    const now = byKey.value.get(o.key)
    return now !== undefined && now !== o.value ? [{ key: o.key, from: now, to: o.value }] : []
  }),
)

const filter = ref('')
const rawList = computed(() => {
  const q = filter.value.trim().toLowerCase()
  const list = options.value ?? []
  return q ? list.filter((o) => o.key.toLowerCase().includes(q) || o.value.toLowerCase().includes(q)) : list
})
</script>

<template>
  <div v-if="loaded" class="@container flex flex-col gap-3 pb-4">
    <p v-if="running" class="rounded-lg border border-warn/50 bg-warn/10 px-3 py-2 text-[12.5px]">{{ t.gopt.running }}</p>
    <p v-else-if="!options" class="rounded-lg border border-line bg-raised px-3 py-2 text-[12.5px] text-muted">{{ t.gopt.missing }}</p>

    <div class="flex flex-wrap items-center gap-2">
      <div class="w-64">
        <SelectMenu
          :model-value="''"
          :options="others"
          :placeholder="t.gopt.copyFrom"
          :aria-label="t.gopt.copyFrom"
          :disabled="running || !others.length"
          searchable
          @update:model-value="copyFrom"
        />
      </div>
      <button class="btn btn-ghost" :disabled="running || !revertChanges.length" :title="t.gopt.revertHint" @click="applyMany(revertChanges)">
        <Undo2 :size="14" />{{ t.gopt.revert }}
      </button>
      <span class="ml-auto text-[12px] text-ok transition-opacity" :class="justSaved ? 'opacity-100' : 'opacity-0'" role="status">{{ justSaved ? t.gopt.saved : '' }}</span>
    </div>

    <template v-if="options">
      <section class="card flex flex-col gap-2 p-4">
        <h3 class="text-[14px] font-semibold">{{ t.gopt.presets }}</h3>
        <p class="text-[12px] text-faint">{{ t.gopt.presetHint }}</p>
        <div class="grid grid-cols-1 gap-2 @xl:grid-cols-2 @4xl:grid-cols-4">
          <button
            v-for="id in presetIds"
            :key="id"
            class="flex items-start gap-3 rounded-lg border border-line bg-raised p-3 text-left transition-colors hover:border-accent hover:bg-hover disabled:opacity-50"
            :disabled="running"
            @click="showPreset(id)"
          >
            <component :is="PRESET_ICONS[id]" :size="18" class="mt-0.5 shrink-0 text-icon" />
            <span class="flex flex-col gap-0.5">
              <span class="font-medium">{{ t.gopt.presetNames[id] }}</span>
              <span class="text-[12px] text-muted">{{ t.gopt.presetDescriptions[id] }}</span>
            </span>
          </button>
        </div>
      </section>

      <div class="columns-1 gap-3 @4xl:columns-2 [&>section]:mb-3 [&>section]:break-inside-avoid">
        <section v-for="group in sections" :key="group.section" class="card flex flex-col gap-2.5 p-4">
          <h3 class="text-[14px] font-semibold">{{ t.gopt.sections[group.section] }}</h3>
          <div v-for="field in group.fields" :key="`${field.key}-${field.kind}`" class="grid grid-cols-[minmax(140px,1fr)_minmax(0,1.4fr)] items-center gap-3">
            <span class="text-muted">{{ label(field.key) }}</span>
            <ToggleSwitch
              v-if="field.kind === 'bool'"
              :model-value="byKey.get(field.key) === 'true'"
              :label="label(field.key)"
              :disabled="running"
              @update:model-value="setValue(field.key, String($event))"
            />
            <SelectMenu
              v-else-if="field.kind === 'choice'"
              :model-value="unquote(byKey.get(field.key) ?? '')"
              :options="choiceOptions(field)"
              :aria-label="label(field.key)"
              :disabled="running"
              @update:model-value="setValue(field.key, requote($event, byKey.get(field.key) ?? ''))"
            />
            <span v-else class="flex items-center gap-2">
              <input
                type="range"
                class="min-w-0 flex-1"
                :min="field.min"
                :max="field.max"
                :step="field.step"
                :value="live[field.key] ?? shownOf(field)"
                :disabled="running"
                :aria-label="label(field.key)"
                @input="live[field.key] = Number(($event.target as HTMLInputElement).value)"
                @change="commitRange(field, Number(($event.target as HTMLInputElement).value))"
              />
              <span class="w-28 shrink-0 text-right font-mono text-[12px]">{{ field.format(live[field.key] ?? shownOf(field)) }}</span>
            </span>
          </div>
        </section>
      </div>

      <details class="card p-4">
        <summary class="cursor-pointer text-[14px] font-semibold">{{ t.gopt.allOptions(options.length) }}</summary>
        <p class="mt-2 text-[12px] text-faint">{{ t.gopt.allOptionsHint }}</p>
        <input v-model="filter" class="input mt-2 w-full" :placeholder="t.gopt.filter" />
        <div class="mt-2 flex flex-col">
          <label v-for="option in rawList" :key="option.key" class="grid grid-cols-[minmax(0,1fr)_minmax(0,1fr)] items-center gap-3 border-b border-line/50 py-1">
            <span class="truncate font-mono text-[12px] text-muted" :title="option.key">{{ option.key }}</span>
            <input
              class="input h-7 font-mono text-[12px]"
              :value="option.value"
              :disabled="running"
              @change="setValue(option.key, ($event.target as HTMLInputElement).value)"
            />
          </label>
        </div>
      </details>
    </template>

    <ModalDialog v-if="preview" :title="preview.title" @close="preview = null">
      <p v-if="preview.whole" class="mb-3 text-[12.5px] text-muted">{{ t.gopt.copyWhole }}</p>
      <p v-if="!preview.changes.length" class="text-muted">{{ t.gopt.noChanges }}</p>
      <ul v-else class="flex flex-col">
        <li v-for="change in preview.changes" :key="change.key" class="grid grid-cols-[minmax(0,1fr)_auto] gap-3 border-b border-line/50 py-1.5">
          <span class="truncate" :class="label(change.key) === change.key ? 'font-mono text-[12px]' : ''" :title="change.key">{{ label(change.key) }}</span>
          <span class="font-mono text-[12px]">
            <template v-if="change.from !== null"><span class="text-faint line-through">{{ display(change.key, change.from) }}</span> → </template>
            <span class="text-ok">{{ display(change.key, change.to) }}</span>
          </span>
        </li>
      </ul>
      <template #footer>
        <button class="btn btn-ghost" @click="preview = null">{{ t.common.cancel }}</button>
        <button class="btn btn-primary" :disabled="!preview.changes.length" @click="confirmPreview">{{ t.gopt.apply(preview.changes.length) }}</button>
      </template>
    </ModalDialog>
  </div>
</template>
