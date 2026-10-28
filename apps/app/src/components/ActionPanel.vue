<script lang="ts">
import type { Component } from 'vue'

export interface PanelAction {
  label: string
  icon: Component
  action: () => void
  disabled?: boolean
  title?: string
  primary?: boolean
  danger?: boolean
  busy?: boolean
  separatorBefore?: boolean
}
</script>

<script setup lang="ts">
defineProps<{ actions: PanelAction[]; footer?: PanelAction[] }>()

const iconColor = (a: PanelAction) => (a.primary ? '' : a.danger ? 'text-danger' : 'text-icon')
</script>

<template>
  <aside data-no-marquee class="sticky top-0 flex w-56 shrink-0 flex-col gap-1 self-start">
    <template v-for="(a, i) in [...actions, ...(footer ?? [])]" :key="a.label">
      <div v-if="a.separatorBefore || (footer?.length && i === actions.length)" class="my-1 h-px bg-line" />
      <button :class="a.primary ? 'btn btn-primary justify-start' : 'btn justify-start'" :disabled="a.disabled" :title="a.title ?? a.label" @click="a.action">
        <component :is="a.icon" :size="15" class="shrink-0" :class="[iconColor(a), a.busy ? 'animate-spin' : '']" />
        <span class="truncate">{{ a.label }}</span>
      </button>
    </template>
  </aside>
</template>
