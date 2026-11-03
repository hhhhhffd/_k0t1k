<script setup lang="ts">
import { Check, Circle, X } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { language, t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'
import CreateInstanceDialog from './CreateInstanceDialog.vue'

/** A dismissible list of the first things to do; each undone step is a shortcut to doing it. */
const settings = useSettingsStore()
const store = useInstancesStore()
const tasks = useTasksStore()
const router = useRouter()
const showCreate = ref(false)

const items = computed(() => {
  const s = settings.settings
  const played = store.instances.find((i) => i.lastPlayed || i.playtime.totalSecs > 0)
  return [
    { id: 'language', label: t.checklist.language, done: !!s?.onboarding.welcomeDone || language.value !== 'en', act: () => router.push('/settings') },
    { id: 'account', label: t.checklist.account, done: !!s?.accounts.length, act: () => router.push('/accounts') },
    { id: 'instance', label: t.checklist.instance, done: store.instances.length > 0, act: () => (showCreate.value = true) },
    {
      id: 'launch',
      label: t.checklist.launch,
      done: !!played,
      // the first instance's page has the Play button
      act: () => store.instances[0] && router.push({ name: 'instance', params: { id: store.instances[0].id } }),
    },
  ]
})
const doneCount = computed(() => items.value.filter((i) => i.done).length)
// Only for people who came through the welcome: an existing user with instances is not new and needs no checklist.
const visible = computed(
  () =>
    !!settings.settings &&
    store.loaded &&
    settings.settings.onboarding.welcomeDone &&
    !settings.settings.onboarding.checklistHidden &&
    doneCount.value < items.value.length,
)
</script>

<template>
  <div v-if="visible" class="card m-2 flex flex-col gap-1 p-2.5 text-[13px]">
    <div class="mb-1 flex items-center gap-2">
      <span class="font-mono font-bold">{{ t.checklist.title }}</span>
      <span class="badge">{{ t.checklist.progress(doneCount, items.length) }}</span>
      <button class="btn btn-ghost btn-icon ml-auto !size-6" :title="t.checklist.hide" :aria-label="t.checklist.hide" @click="tasks.guard(() => settings.markOnboarding('checklist'))"><X :size="14" /></button>
    </div>
    <template v-for="item in items" :key="item.id">
      <div v-if="item.done" class="flex items-center gap-2 px-1 py-0.5 text-faint line-through"><Check :size="14" class="text-ok" />{{ item.label }}</div>
      <button v-else class="flex items-center gap-2 rounded px-1 py-0.5 text-left hover:bg-hover" @click="item.act()"><Circle :size="14" class="text-accent" />{{ item.label }}</button>
    </template>
    <CreateInstanceDialog v-if="showCreate" @close="showCreate = false" />
  </div>
</template>
