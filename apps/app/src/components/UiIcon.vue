<script setup lang="ts">
import type { Component } from 'vue'
import { computed } from 'vue'
import { fileUrl } from '../api/commands'
import { useCustomizationStore } from '../stores/customization'

/** A built-in icon that the chosen icon set can replace with its `<name>.png|svg|gif|webp` file. */
const props = withDefaults(defineProps<{ name: string; icon?: Component; size?: number }>(), { size: 16 })
const custom = useCustomizationStore()
const file = computed(() => custom.iconFiles[props.name])
</script>

<template>
  <img v-if="file" :src="fileUrl(file)" alt="" class="shrink-0 object-contain" :style="{ width: `${size}px`, height: `${size}px` }" />
  <component :is="icon" v-else-if="icon" :size="size" />
</template>
