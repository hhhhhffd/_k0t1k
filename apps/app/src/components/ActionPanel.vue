<script lang="ts">
import type { Component } from 'vue'

export interface PanelAction {
  label: string
  icon: Component
  action: () => void
  disabled?: boolean
  /** Tooltip, e.g. why the action is disabled. */
  title?: string
  primary?: boolean
  danger?: boolean
  /** Spins the icon while the action runs. */
  busy?: boolean
  separatorBefore?: boolean
}
</script>

<script setup lang="ts">
/** The column of actions next to a list (Prism's side panel): the top group works on the selection, `footer` sits at the bottom. */
defineProps<{ actions: PanelAction[]; footer?: PanelAction[] }>()
</script>

<template>
  <aside class="card sticky top-0 flex w-56 shrink-0 flex-col gap-0.5 self-start p-1.5">
    <template v-for="a in actions" :key="a.label">
      <div v-if="a.separatorBefore" class="mx-1 my-1 border-t border-line" />
      <button
        :class="a.primary ? 'btn btn-primary mb-1 w-full justify-start' : 'tool w-full justify-start'"
        :disabled="a.disabled"
        :title="a.title ?? a.label"
        @click="a.action"
      >
        <component :is="a.icon" :size="15" class="shrink-0" :class="[a.busy ? 'animate-spin' : '', a.danger && !a.disabled ? 'text-danger' : '']" />
        <span class="truncate" :class="a.danger && !a.disabled ? 'text-danger' : ''">{{ a.label }}</span>
      </button>
    </template>
    <template v-if="footer?.length">
      <div class="mx-1 my-1 border-t border-line" />
      <button v-for="a in footer" :key="a.label" class="tool w-full justify-start text-muted" :disabled="a.disabled" :title="a.title ?? a.label" @click="a.action">
        <component :is="a.icon" :size="15" class="shrink-0" />
        <span class="truncate">{{ a.label }}</span>
      </button>
    </template>
  </aside>
</template>
