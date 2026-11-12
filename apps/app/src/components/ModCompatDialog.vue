<script setup lang="ts">
import { Download, LoaderCircle } from '@lucide/vue'
import { computed } from 'vue'
import { t } from '../i18n'
import { needsOf, useModCompatStore } from '../stores/modCompat'
import ModalDialog from './ModalDialog.vue'

const modCompat = useModCompatStore()
const hasDependencies = computed(() => modCompat.prompt?.mods.some((mod) => mod.dependencies.length))
const title = computed(() => modCompat.prompt?.mode === 'add' ? t.modCompat.addTitle : modCompat.prompt?.mode === 'repair' ? t.modCompat.repairTitle : t.modCompat.title)

function close() {
  if (!modCompat.prompt?.downloading) modCompat.prompt?.answer('cancel')
}
</script>

<template>
  <ModalDialog v-if="modCompat.prompt" :title="title" size="md" @close="close">
    <div class="flex flex-col gap-3" :aria-busy="modCompat.prompt.downloading">
      <p class="text-muted">{{ (modCompat.prompt.mode === 'add' ? t.modCompat.addText : modCompat.prompt.mode === 'repair' ? t.modCompat.repairText : t.modCompat.text)(modCompat.prompt.minecraftVersion) }}</p>
      <ul v-if="modCompat.prompt.mods.length" class="selectable flex max-h-64 flex-col gap-2 overflow-y-auto text-[12px]">
        <li v-for="mod in modCompat.prompt.mods" :key="mod.fileName" class="rounded-lg border border-line bg-raised p-3">
          <span class="break-all font-mono font-semibold">{{ mod.fileName }}</span>
          <p class="mt-1 break-words text-warn">{{ needsOf(mod) }}</p>
        </li>
      </ul>
      <p v-if="hasDependencies" class="rounded-lg border border-accent/30 bg-accent/10 p-3 text-[12px] text-muted">{{ t.modCompat.downloadHint }}</p>
      <p v-if="modCompat.prompt.downloading" role="status" class="flex items-center gap-2 text-accent"><LoaderCircle :size="15" class="animate-spin" />{{ t.modCompat.downloading }}</p>
      <p v-if="modCompat.prompt.message" role="status" class="text-[12px] text-muted">{{ modCompat.prompt.message }}</p>
      <p v-if="modCompat.prompt.error" role="alert" class="selectable break-words text-[12px] text-danger">{{ modCompat.prompt.error }}</p>
    </div>
    <template #footer>
      <button class="btn" :disabled="modCompat.prompt.downloading" @click="close">{{ modCompat.prompt.mode === 'repair' ? t.common.close : t.common.cancel }}</button>
      <template v-if="modCompat.prompt.mode !== 'repair'">
        <button class="btn" :class="!modCompat.prompt.mods.length ? 'btn-primary' : ''" :disabled="modCompat.prompt.downloading" @click="modCompat.prompt.answer('launch')">{{ modCompat.prompt.mode === 'add' ? t.modCompat.addAnyway : modCompat.prompt.mods.length ? t.modCompat.launchAnyway : t.library.play }}</button>
        <button v-if="modCompat.prompt.mods.length" class="btn" :class="!hasDependencies ? 'btn-primary' : ''" :disabled="modCompat.prompt.downloading" @click="modCompat.prompt.answer('disable')">{{ modCompat.prompt.mode === 'add' ? t.modCompat.skipThese : t.modCompat.disableAndLaunch }}</button>
      </template>
      <button v-if="hasDependencies" class="btn btn-primary" :disabled="modCompat.prompt.downloading" @click="modCompat.downloadDependencies">
        <Download v-if="!modCompat.prompt.downloading" :size="15" /><LoaderCircle v-else :size="15" class="animate-spin" />{{ modCompat.prompt.downloading ? t.modCompat.downloading : t.modCompat.downloadDependencies }}
      </button>
    </template>
  </ModalDialog>
</template>
