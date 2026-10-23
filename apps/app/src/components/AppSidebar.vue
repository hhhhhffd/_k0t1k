<script setup lang="ts">
import { Check, ChevronsUpDown, Library, Compass, Settings, User } from '@lucide/vue'
import { computed, ref } from 'vue'
import { RouterLink } from 'vue-router'
import mascot from '../assets/mascot.png'
import { t } from '../i18n/ru'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'

const settings = useSettingsStore()
const tasks = useTasksStore()
const open = ref(false)

const nav = [
  { to: '/', label: t.nav.library, icon: Library },
  { to: '/browse', label: t.nav.browse, icon: Compass },
  { to: '/settings', label: t.nav.settings, icon: Settings },
]
const accounts = computed(() => settings.settings?.accounts ?? [])
const current = computed(() => accounts.value.find((a) => a.id === settings.settings?.selectedAccountId))

async function pick(id: string) {
  open.value = false
  await tasks.guard(() => settings.selectAccount(id))
}
</script>

<template>
  <aside class="dot-grid relative flex w-56 shrink-0 flex-col border-r border-line bg-surface">
    <div class="flex items-center gap-2.5 px-4 pt-5 pb-4">
      <img :src="mascot" alt="" class="pixelated pix anim-bob size-11 bg-black" />
      <div class="leading-tight">
        <div class="font-mono text-[19px] font-bold tracking-tight">_k0t1k</div>
        <div class="text-[11px] text-faint">{{ t.app.tagline }}</div>
      </div>
    </div>

    <nav class="flex flex-1 flex-col gap-1 px-2.5">
      <RouterLink
        v-for="item in nav"
        :key="item.to"
        :to="item.to"
        class="flex h-9 items-center gap-2.5 rounded-lg px-3 font-medium text-muted transition-colors hover:bg-hover hover:text-fg"
        active-class="!bg-accent/15 !text-accent"
        :exact-active-class="item.to === '/' ? '!bg-accent/15 !text-accent' : ''"
      >
        <component :is="item.icon" :size="17" />
        {{ item.label }}
      </RouterLink>
    </nav>

    <div class="relative border-t border-line p-2.5">
      <div v-if="open" class="card anim-pop absolute right-2.5 bottom-[calc(100%-4px)] left-2.5 p-1 shadow-[var(--shadow-pop)]">
        <button v-for="acc in accounts" :key="acc.id" class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-hover" @click="pick(acc.id)">
          <span class="flex-1 truncate">{{ acc.name }}</span>
          <Check v-if="acc.id === current?.id" :size="14" class="text-accent" />
        </button>
        <RouterLink to="/settings#accounts" class="flex h-8 items-center rounded-md px-2 text-faint hover:bg-hover" @click="open = false">{{ t.account.add }}</RouterLink>
      </div>
      <button class="flex h-11 w-full items-center gap-2.5 rounded-lg px-2 text-left hover:bg-hover" :aria-label="t.account.switch" @click="open = !open">
        <span class="pix flex size-8 shrink-0 items-center justify-center bg-accent/20 text-accent"><User :size="16" /></span>
        <span class="min-w-0 flex-1 leading-tight">
          <span class="block truncate font-medium">{{ current?.name ?? t.account.none }}</span>
          <span class="block text-[11px] text-faint">{{ t.account.offline }}</span>
        </span>
        <ChevronsUpDown :size="14" class="text-faint" />
      </button>
    </div>
  </aside>
</template>
