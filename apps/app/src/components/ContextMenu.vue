<script lang="ts">
import type { Component } from 'vue'

export interface MenuItem {
  label: string
  icon?: Component
  /** Key under which an icon set can replace `icon`. */
  iconKey?: string
  /** Colours the icon only, e.g. `text-ok`. */
  iconClass?: string
  danger?: boolean
  disabled?: boolean
  separatorBefore?: boolean
  action: () => void
}
</script>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from 'vue'
import UiIcon from './UiIcon.vue'

const props = defineProps<{ x: number; y: number; items: MenuItem[] }>()
const emit = defineEmits<{ close: [] }>()

// keep the menu inside the viewport
const pos = computed(() => ({
  left: `${Math.min(props.x, window.innerWidth - 220)}px`,
  top: `${Math.min(props.y, window.innerHeight - props.items.length * 34 - 16)}px`,
}))

function onKey(e: KeyboardEvent) {
  if (e.key === 'Escape') emit('close')
}
onMounted(() => window.addEventListener('keydown', onKey))
onBeforeUnmount(() => window.removeEventListener('keydown', onKey))

function pick(item: MenuItem) {
  emit('close')
  item.action()
}
</script>

<template>
  <Teleport to="body">
    <div class="fixed inset-0 z-[60]" @mousedown="emit('close')" @contextmenu.prevent="emit('close')">
      <div class="card anim-pop absolute w-52 p-1 shadow-[var(--shadow-pop)]" :style="pos" role="menu" @mousedown.stop>
        <template v-for="item in items" :key="item.label">
          <div v-if="item.separatorBefore" class="my-1 h-px bg-line" />
          <button
            role="menuitem"
            class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-[13px] hover:bg-hover disabled:pointer-events-none disabled:opacity-40"
            :class="item.danger ? 'text-danger' : 'text-fg'"
            :disabled="item.disabled"
            @click="pick(item)"
          >
            <UiIcon v-if="item.icon" :name="item.iconKey ?? ''" :icon="item.icon" :size="15" :class="item.iconClass" />
            {{ item.label }}
          </button>
        </template>
      </div>
    </div>
  </Teleport>
</template>
