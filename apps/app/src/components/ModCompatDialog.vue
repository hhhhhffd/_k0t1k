<script setup lang="ts">
import { t } from '../i18n'
import { needsOf, useModCompatStore } from '../stores/modCompat'
import ModalDialog from './ModalDialog.vue'

const modCompat = useModCompatStore()
</script>

<template>
  <ModalDialog v-if="modCompat.prompt" :title="t.modCompat.title" :width="480" @close="modCompat.prompt.answer('cancel')">
    <div class="flex flex-col gap-2">
      <p class="text-muted">{{ t.modCompat.text(modCompat.prompt.minecraftVersion) }}</p>
      <ul class="selectable max-h-48 list-disc overflow-y-auto pl-5 text-[12px]">
        <li v-for="mod in modCompat.prompt.mods" :key="mod.fileName">
          <span class="font-mono">{{ mod.fileName }}</span> <span class="text-warn">— {{ needsOf(mod) }}</span>
        </li>
      </ul>
    </div>
    <template #footer>
      <button class="btn" @click="modCompat.prompt.answer('cancel')">{{ t.common.cancel }}</button>
      <button class="btn" @click="modCompat.prompt.answer('launch')">{{ t.modCompat.launchAnyway }}</button>
      <button class="btn btn-primary" @click="modCompat.prompt.answer('disable')">{{ t.modCompat.disableAndLaunch }}</button>
    </template>
  </ModalDialog>
</template>
