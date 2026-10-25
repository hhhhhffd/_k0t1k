<script setup lang="ts">
import { t } from '../../i18n'
defineProps<{ title: string }>()
/** true = this instance overrides the global value. */
const overridden = defineModel<boolean>({ required: true })
</script>

<template>
  <div class="flex flex-col gap-3 py-4 first:pt-0 last:pb-0">
    <header class="flex items-center gap-2">
      <h3 class="min-w-0 flex-1 text-[14px] font-semibold">{{ title }}</h3>
      <span class="badge" :class="overridden ? 'badge-accent' : ''">{{ overridden ? t.iset.ownValue : t.iset.globalValue }}</span>
      <button class="btn h-7" @click="overridden = !overridden">{{ overridden ? t.iset.resetToGlobal : t.iset.overrideHere }}</button>
    </header>
    <div :class="overridden ? '' : 'opacity-60'"><slot :disabled="!overridden" /></div>
  </div>
</template>
