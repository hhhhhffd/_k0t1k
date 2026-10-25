<script setup lang="ts">
import { Download } from '@lucide/vue'
import { computed } from 'vue'
import { useRouter } from 'vue-router'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useLayoutStore } from '../stores/layout'
import { useUpdateStore } from '../stores/update'
import { formatPlaytime, formatRelative, loaderOf, minecraftVersionOf } from '../utils/format'

const store = useInstancesStore()
const layout = useLayoutStore()
const update = useUpdateStore()
const router = useRouter()

const selected = computed(() => (layout.selectedId ? store.byId(layout.selectedId) : undefined))
const totalSecs = computed(() => store.instances.reduce((sum, i) => sum + i.playtime.totalSecs, 0))
const summary = computed(() => {
  const inst = selected.value
  if (!inst) return ''
  const loader = loaderOf(inst)
  const parts = [`Minecraft ${minecraftVersionOf(inst)}${loader ? ` · ${loader.label} ${loader.version}` : ''}`]
  parts.push(inst.lastPlayed ? t.status.lastPlayed(formatRelative(inst.lastPlayed), formatPlaytime(inst.playtime.lastSecs)) : t.status.neverPlayed)
  parts.push(t.status.instanceTotal(formatPlaytime(inst.playtime.totalSecs)))
  return parts.join(', ')
})
</script>

<template>
  <footer class="flex h-7 shrink-0 items-center gap-4 border-t border-line bg-surface px-3 text-[12px] text-muted">
    <span class="min-w-0 flex-1 truncate">{{ summary }}</span>
    <button v-if="update.release" class="flex shrink-0 items-center gap-1.5 text-accent hover:underline" @click="router.push({ path: '/settings', hash: '#about' })">
      <Download :size="13" />{{ t.update.available(update.release.version) }}
    </button>
    <span class="shrink-0">{{ t.status.total(formatPlaytime(totalSecs)) }}</span>
  </footer>
</template>
