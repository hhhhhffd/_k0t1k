<script setup lang="ts">
import { LayoutGrid, List } from '@lucide/vue'
import { t } from '../i18n'
import type { ViewMode } from '../utils/useViewMode'

const mode = defineModel<ViewMode>({ required: true })
const options = [
  { value: 'list', icon: List, label: () => t.common.viewList },
  { value: 'tiles', icon: LayoutGrid, label: () => t.common.viewTiles },
] as const
</script>

<template>
  <div class="inline-flex shrink-0 rounded-lg border border-line bg-raised p-0.5" role="group">
    <button
      v-for="o in options"
      :key="o.value"
      type="button"
      class="flex size-6 items-center justify-center rounded-md transition-colors"
      :class="mode === o.value ? 'bg-accent/20 text-accent' : 'text-faint hover:text-fg'"
      :title="o.label()"
      :aria-label="o.label()"
      :aria-pressed="mode === o.value"
      @click="mode = o.value"
    >
      <component :is="o.icon" :size="14" />
    </button>
  </div>
</template>
