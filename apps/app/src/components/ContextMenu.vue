<script lang="ts">
import type { Component } from 'vue'

export interface MenuItem {
  label: string
  /** Faint text after the label, such as the state of the thing the item acts on. */
  hint?: string
  icon?: Component
  /** Key under which an icon set can replace `icon`. */
  iconKey?: string
  danger?: boolean
  disabled?: boolean
  separatorBefore?: boolean
  /** The menu stays open after the action, for a list that is acted on item by item. */
  keepOpen?: boolean
  action: () => void
}
</script>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import UiIcon from './UiIcon.vue'

const props = defineProps<{ x: number; y: number; items: MenuItem[] }>()
const emit = defineEmits<{ close: [] }>()

const menu = ref<HTMLElement | null>(null)
// keep the menu inside the viewport; its size is measured once it is laid out, because the labels and separators set it
const size = ref({ width: 208, height: props.items.length * 34 })
const pos = computed(() => ({
  left: `${Math.max(0, Math.min(props.x, window.innerWidth - size.value.width - 12))}px`,
  top: `${Math.max(0, Math.min(props.y, window.innerHeight - size.value.height - 12))}px`,
}))

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}
onMounted(() => {
  window.addEventListener('keydown', onKey)
  if (menu.value) size.value = { width: menu.value.offsetWidth, height: menu.value.offsetHeight }
})
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

function pick(item: MenuItem) {
  if (!item.keepOpen) emit('close')
  item.action()
}
</script>

<template>
  <Teleport to="body">
    <div class="fixed inset-0 z-[60]" @mousedown="emit('close')" @contextmenu.prevent="emit('close')">
      <div ref="menu" class="card anim-pop absolute w-max min-w-52 max-w-80 p-1 shadow-[var(--shadow-pop)]" :style="pos" role="menu" @mousedown.stop>
        <template v-for="item in items" :key="item.label">
          <div v-if="item.separatorBefore" class="my-1 h-px bg-line" />
          <button
            role="menuitem"
            class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] whitespace-nowrap hover:bg-hover disabled:pointer-events-none disabled:opacity-40"
            :class="item.danger ? 'text-danger' : 'text-fg'"
            :disabled="item.disabled"
            @click="pick(item)"
          >
            <UiIcon v-if="item.icon" :name="item.iconKey ?? ''" :icon="item.icon" :size="15" :class="item.danger ? '' : 'text-icon'" />
            {{ item.label }}
            <span v-if="item.hint" class="ml-auto pl-3 text-[12px] text-faint">{{ item.hint }}</span>
          </button>
        </template>
      </div>
    </div>
  </Teleport>
</template>
