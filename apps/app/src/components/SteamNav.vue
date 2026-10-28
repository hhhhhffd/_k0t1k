<script setup lang="ts">
import { ArrowLeft, ArrowRight } from '@lucide/vue'
import { computed } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { t } from '../i18n'
import { useSettingsStore } from '../stores/settings'

/** The Steam-like mode's top bar: history arrows and big section links instead of the toolbar. */
const settings = useSettingsStore()
const route = useRoute()
const router = useRouter()

const account = computed(() => settings.settings?.accounts.find((a) => a.id === settings.settings?.selectedAccountId))
const links = computed(() => [
  { label: t.nav.library, active: route.name === 'library' || route.name === 'instance', go: () => router.push('/') },
  { label: t.nav.settings, active: route.name === 'settings' && route.hash !== '#accounts', go: () => router.push('/settings') },
  { label: account.value?.name ?? t.account.none, active: route.name === 'settings' && route.hash === '#accounts', go: () => router.push('/settings#accounts') },
])
</script>

<template>
  <div class="flex h-12 shrink-0 items-center gap-1 border-b border-line bg-surface px-3">
    <button class="btn btn-ghost btn-icon h-8 w-8" :title="t.nav.back" :aria-label="t.nav.back" @click="router.back()"><ArrowLeft :size="18" class="text-icon" /></button>
    <button class="btn btn-ghost btn-icon mr-3 h-8 w-8" :title="t.nav.forward" :aria-label="t.nav.forward" @click="router.forward()"><ArrowRight :size="18" class="text-icon" /></button>
    <button
      v-for="link in links"
      :key="link.label"
      class="nav-link h-full px-3 text-[18px] font-bold tracking-wide uppercase transition-colors"
      :class="link.active ? 'nav-link-active text-accent' : 'text-fg hover:text-accent'"
      :aria-current="link.active ? 'page' : undefined"
      @click="link.go()"
    >
      <span class="max-w-60 truncate">{{ link.label }}</span>
    </button>
  </div>
</template>

<style scoped>
.nav-link {
  display: flex;
  align-items: center;
  border-bottom: 3px solid transparent;
  padding-top: 3px;
}
.nav-link-active {
  border-bottom-color: var(--c-accent);
}
</style>
