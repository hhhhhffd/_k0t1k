<script setup lang="ts">
import { Check, ChevronDown, Search } from '@lucide/vue'
import { computed, nextTick, onBeforeUnmount, ref, useId, watch } from 'vue'
import { t } from '../i18n'

export interface SelectOption {
  value: string
  label: string
  /** Muted text shown at the right of the item. */
  hint?: string
}

const props = defineProps<{
  options: SelectOption[]
  ariaLabel?: string
  placeholder?: string
  disabled?: boolean
  /** Adds a filter field; meant for long lists such as Minecraft versions. */
  searchable?: boolean
}>()
const model = defineModel<string>({ required: true })

const MAX_LIST = 288
const uid = useId()
const root = ref<HTMLElement | null>(null)
const trigger = ref<HTMLButtonElement | null>(null)
const popup = ref<HTMLElement | null>(null)
const searchInput = ref<HTMLInputElement | null>(null)
const open = ref(false)
const query = ref('')
const active = ref(0)
// `anchor` is the distance from the top (opens down) or from the bottom (opens up) of the viewport
const place = ref({ left: 0, width: 0, anchor: 0, up: false, listHeight: MAX_LIST })

const selected = computed(() => props.options.find((o) => o.value === model.value))
const visible = computed(() => {
  const needle = query.value.trim().toLowerCase()
  return needle ? props.options.filter((o) => o.label.toLowerCase().includes(needle)) : props.options
})
const optionId = (i: number) => `${uid}-opt-${i}`

function position() {
  const el = trigger.value
  if (!el) return
  const r = el.getBoundingClientRect()
  const below = window.innerHeight - r.bottom - 12
  const above = r.top - 12
  const up = below < 180 && above > below
  const space = (up ? above : below) - (props.searchable ? 44 : 0)
  place.value = {
    left: r.left,
    width: r.width,
    anchor: up ? window.innerHeight - r.top + 4 : r.bottom + 4,
    up,
    listHeight: Math.min(MAX_LIST, Math.max(120, space)),
  }
}

function scrollActive() {
  void nextTick(() => document.getElementById(optionId(active.value))?.scrollIntoView({ block: 'nearest' }))
}

function openMenu() {
  if (props.disabled || open.value) return
  query.value = ''
  active.value = Math.max(0, props.options.findIndex((o) => o.value === model.value))
  position()
  open.value = true
  scrollActive()
  if (props.searchable) void nextTick(() => searchInput.value?.focus())
}

function close(refocus = true) {
  open.value = false
  if (refocus) trigger.value?.focus()
}

function choose(i: number) {
  const opt = visible.value[i]
  if (!opt) return
  model.value = opt.value
  close()
}

function move(delta: number) {
  const n = visible.value.length
  if (!n) return
  active.value = (active.value + delta + n) % n
  scrollActive()
}

watch(query, () => {
  active.value = 0
  document.getElementById(`${uid}-list`)?.scrollTo({ top: 0 })
})

// type-to-search for the plain (non searchable) variant: jump to the first label with the typed prefix
let typed = ''
let typedTimer: ReturnType<typeof setTimeout> | undefined
function typeAhead(char: string) {
  typed += char.toLowerCase()
  clearTimeout(typedTimer)
  typedTimer = setTimeout(() => (typed = ''), 700)
  const idx = visible.value.findIndex((o) => o.label.toLowerCase().startsWith(typed))
  if (idx >= 0) {
    active.value = idx
    scrollActive()
  }
}

function onKey(e: KeyboardEvent) {
  if (!open.value) {
    if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(e.key)) {
      e.preventDefault()
      openMenu()
    }
    return
  }
  switch (e.key) {
    case 'ArrowDown': e.preventDefault(); move(1); break
    case 'ArrowUp': e.preventDefault(); move(-1); break
    case 'Home':
      if (!props.searchable) { e.preventDefault(); active.value = 0; scrollActive() }
      break
    case 'End':
      if (!props.searchable) { e.preventDefault(); active.value = visible.value.length - 1; scrollActive() }
      break
    case 'Enter': e.preventDefault(); choose(active.value); break
    // stopPropagation: Escape must close only the menu, not the dialog around it
    case 'Escape': e.preventDefault(); e.stopPropagation(); close(); break
    case 'Tab': close(false); break
    default:
      if (!props.searchable && e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
        e.preventDefault()
        typeAhead(e.key)
      }
  }
}

/** Clicking an item must not steal focus from the trigger; the search field still needs it. */
function keepFocus(e: MouseEvent) {
  if (!(e.target instanceof HTMLInputElement)) e.preventDefault()
}

function onOutside(e: Event) {
  const target = e.target as Node
  if (!root.value?.contains(target) && !popup.value?.contains(target)) close(false)
}
function onScrollOrResize(e: Event) {
  // scrolling the list itself must not close it
  if (e.target instanceof Node && popup.value?.contains(e.target)) return
  close(false)
}

watch(open, (isOpen) => {
  const act = isOpen ? 'addEventListener' : 'removeEventListener'
  document[act]('mousedown', onOutside, true)
  window[act]('scroll', onScrollOrResize, true)
  window[act]('resize', onScrollOrResize)
})
onBeforeUnmount(() => {
  clearTimeout(typedTimer)
  document.removeEventListener('mousedown', onOutside, true)
  window.removeEventListener('scroll', onScrollOrResize, true)
  window.removeEventListener('resize', onScrollOrResize)
})
</script>

<template>
  <div ref="root" class="min-w-0">
    <button
      ref="trigger"
      type="button"
      class="input flex items-center gap-2 text-left"
      role="combobox"
      aria-haspopup="listbox"
      :aria-expanded="open"
      :aria-controls="`${uid}-list`"
      :aria-label="ariaLabel"
      :disabled="disabled"
      @click="open ? close() : openMenu()"
      @keydown="onKey"
    >
      <span class="min-w-0 flex-1 truncate" :class="selected ? '' : 'text-faint'">{{ selected?.label ?? placeholder ?? '' }}</span>
      <ChevronDown :size="15" class="shrink-0 text-faint transition-transform" :class="open ? 'rotate-180' : ''" />
    </button>

    <Teleport to="body">
      <div
        v-if="open"
        ref="popup"
        class="anim-pop card fixed z-[70] flex flex-col overflow-hidden shadow-[var(--shadow-pop)]"
        :style="{
          left: `${place.left}px`,
          width: `${place.width}px`,
          minWidth: '160px',
          ...(place.up ? { bottom: `${place.anchor}px` } : { top: `${place.anchor}px` }),
        }"
        @mousedown="keepFocus"
      >
        <div v-if="searchable" class="flex items-center gap-2 border-b border-line px-2.5 py-1.5">
          <Search :size="14" class="shrink-0 text-faint" />
          <input
            ref="searchInput"
            v-model="query"
            class="h-6 min-w-0 flex-1 border-0 bg-transparent p-0 text-fg outline-none placeholder:text-faint"
            :placeholder="t.common.search"
            :aria-label="t.common.search"
            :aria-controls="`${uid}-list`"
            :aria-activedescendant="visible.length ? optionId(active) : undefined"
            @keydown="onKey"
          />
        </div>
        <ul :id="`${uid}-list`" role="listbox" class="overflow-y-auto p-1" :style="{ maxHeight: `${place.listHeight}px` }" :aria-label="ariaLabel">
          <li
            v-for="(o, i) in visible"
            :id="optionId(i)"
            :key="o.value"
            role="option"
            :aria-selected="o.value === model"
            class="flex h-8 cursor-pointer items-center gap-2 rounded-md px-2.5"
            :class="[i === active ? 'bg-hover' : '', o.value === model ? 'text-accent' : '']"
            @mousemove="active = i"
            @click="choose(i)"
          >
            <span class="min-w-0 flex-1 truncate">{{ o.label }}</span>
            <span v-if="o.hint" class="shrink-0 text-[11px] text-faint">{{ o.hint }}</span>
            <Check v-if="o.value === model" :size="14" class="shrink-0" />
          </li>
          <li v-if="!visible.length" class="px-2.5 py-2 text-faint">{{ t.common.nothingFound }}</li>
        </ul>
      </div>
    </Teleport>
  </div>
</template>
