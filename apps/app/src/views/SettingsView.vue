<script setup lang="ts">
import { Check, RefreshCw, Trash2, UserPlus } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { detectJava, getSystemInfo, listManagedJava } from '../api/commands'
import type { JavaInstall, SystemInfo } from '../api/types'
import mascot from '../assets/mascot.png'
import CommandFields from '../components/fields/CommandFields.vue'
import MemoryFields from '../components/fields/MemoryFields.vue'
import WindowFields from '../components/fields/WindowFields.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import { t } from '../i18n/ru'
import { THEMES, useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'

const store = useSettingsStore()
const tasks = useTasksStore()
const route = useRoute()

const sectionIds = ['java', 'memory', 'jvm', 'window', 'commands', 'appearance', 'accounts', 'about'] as const
type Section = (typeof sectionIds)[number]
const hashSection = route.hash.slice(1) as Section
const section = ref<Section>(sectionIds.includes(hashSection) ? hashSection : 'java')

const system = ref<SystemInfo | null>(null)
const detected = ref<JavaInstall[]>([])
const managed = ref<JavaInstall[]>([])
const scanning = ref(false)
const newName = ref('')

const s = computed(() => store.settings)

async function rescan() {
  scanning.value = true
  detected.value = (await tasks.guard(detectJava)) ?? []
  scanning.value = false
}

onMounted(async () => {
  system.value = (await tasks.guard(getSystemInfo)) ?? null
  managed.value = (await tasks.guard(listManagedJava)) ?? []
  await rescan()
})

// Autosave: every edit of the settings object is persisted after a short pause.
let timer: ReturnType<typeof setTimeout> | undefined
watch(
  () => JSON.stringify(store.settings),
  (now, before) => {
    if (before === 'null' || now === before) return
    clearTimeout(timer)
    timer = setTimeout(() => void tasks.guard(store.save), 400)
  },
)

async function addAccount() {
  const name = newName.value.trim()
  if (!name) return
  newName.value = ''
  await tasks.guard(() => store.addAccount(name))
}
</script>

<template>
  <div v-if="s" class="mx-auto flex max-w-5xl gap-6 p-6">
    <nav class="sticky top-6 flex h-fit w-44 shrink-0 flex-col gap-1" :aria-label="t.settings.title">
      <h1 class="mb-2 px-3 text-xl font-bold">{{ t.settings.title }}</h1>
      <button
        v-for="id in sectionIds"
        :key="id"
        class="h-8 rounded-lg px-3 text-left font-medium transition-colors"
        :class="section === id ? 'bg-accent/15 text-accent' : 'text-muted hover:bg-hover hover:text-fg'"
        @click="section = id"
      >
        {{ t.settings.sections[id] }}
      </button>
    </nav>

    <div class="flex min-w-0 max-w-2xl flex-1 flex-col gap-4">
      <h2 class="text-[17px] font-bold">{{ t.settings.sections[section] }}</h2>

      <template v-if="section === 'java'">
        <section class="card flex flex-col gap-4 p-4">
          <label class="flex items-center gap-3">
            <ToggleSwitch v-model="s.java.autoSelect" :label="t.settings.autoSelect" />
            <span><span class="block font-medium">{{ t.settings.autoSelect }}</span><span class="text-[12px] text-faint">{{ t.settings.autoSelectHint }}</span></span>
          </label>
          <label class="flex items-center gap-3">
            <ToggleSwitch v-model="s.java.autoDownload" :label="t.settings.autoDownload" />
            <span><span class="block font-medium">{{ t.settings.autoDownload }}</span><span class="text-[12px] text-faint">{{ t.settings.autoDownloadHint }}</span></span>
          </label>
          <label class="flex flex-col gap-1">
            <span class="label">{{ t.settings.javaPath }}</span>
            <input v-model="s.java.path" class="input font-mono" placeholder="C:\Program Files\Java\...\javaw.exe" />
          </label>
        </section>
        <section class="card p-4">
          <div class="mb-2 flex items-center justify-between">
            <h3 class="section-title">{{ t.settings.detected }}</h3>
            <button class="btn h-7" :disabled="scanning" @click="rescan"><RefreshCw :size="14" :class="scanning ? 'animate-spin' : ''" />{{ scanning ? t.settings.scanning : t.settings.rescan }}</button>
          </div>
          <ul class="flex flex-col gap-1.5">
            <li v-for="j in detected" :key="j.path" class="flex items-center gap-3 rounded-lg bg-raised px-3 py-2">
              <span class="badge badge-accent">Java {{ j.major }}</span>
              <div class="min-w-0 flex-1">
                <div class="font-medium">{{ j.vendor }} {{ j.version }} <span class="text-faint">· {{ j.arch }}</span></div>
                <div class="truncate font-mono text-[11.5px] text-faint">{{ j.path }}</div>
              </div>
              <button class="btn h-7" :disabled="s.java.path === j.path" @click="s.java.path = j.path"><Check v-if="s.java.path === j.path" :size="14" />{{ t.settings.use }}</button>
            </li>
            <li v-if="!detected.length && !scanning" class="text-faint">{{ t.common.nothingFound }}</li>
          </ul>
        </section>
        <section class="card p-4">
          <h3 class="section-title mb-2">{{ t.settings.managed }}</h3>
          <ul class="flex flex-col gap-1.5">
            <li v-for="j in managed" :key="j.path" class="flex items-center gap-3 rounded-lg bg-raised px-3 py-2">
              <span class="badge">Java {{ j.major }}</span>
              <div class="min-w-0 flex-1">
                <div class="font-medium">{{ j.vendor }} {{ j.version }}</div>
                <div class="truncate font-mono text-[11.5px] text-faint">{{ j.path }}</div>
              </div>
            </li>
            <li v-if="!managed.length" class="text-faint">{{ t.common.none }}</li>
          </ul>
        </section>
      </template>

      <section v-else-if="section === 'memory'" class="card p-4"><MemoryFields v-model="s.memory" :total-ram-mb="system?.totalRamMb" /></section>

      <section v-else-if="section === 'jvm'" class="card p-4">
        <textarea v-model="s.jvmArgs" class="input font-mono" rows="4" :aria-label="t.settings.sections.jvm" />
      </section>

      <section v-else-if="section === 'window'" class="card p-4"><WindowFields v-model="s.window" /></section>

      <section v-else-if="section === 'commands'" class="card p-4"><CommandFields v-model="s.commands" /></section>

      <template v-else-if="section === 'appearance'">
        <section class="card p-4">
          <h3 class="section-title mb-3">{{ t.settings.theme }}</h3>
          <div class="grid grid-cols-2 gap-3">
            <button
              v-for="th in THEMES"
              :key="th.id"
              :data-theme="th.id"
              class="overflow-hidden rounded-xl border-2 text-left transition-colors"
              :class="s.ui.theme === th.id ? 'border-accent' : 'border-line hover:border-faint'"
              :aria-pressed="s.ui.theme === th.id"
              style="background: var(--c-base); color: var(--c-fg)"
              @click="store.setUi({ theme: th.id })"
            >
              <div class="flex h-20">
                <div class="w-1/4" style="background: var(--c-surface); border-right: 1px solid var(--c-line)">
                  <div class="m-2 h-1.5 rounded" style="background: var(--c-accent)" />
                  <div class="m-2 h-1.5 rounded" style="background: var(--c-line)" />
                  <div class="m-2 h-1.5 rounded" style="background: var(--c-line)" />
                </div>
                <div class="flex-1 p-2">
                  <div class="mb-1.5 h-2 w-1/2 rounded" style="background: var(--c-fg)" />
                  <div class="flex gap-1.5">
                    <div class="h-9 flex-1 rounded" style="background: var(--c-raised); border: 1px solid var(--c-line)" />
                    <div class="h-9 flex-1 rounded" style="background: var(--c-raised); border: 1px solid var(--c-line)" />
                  </div>
                  <div class="mt-1.5 h-3 w-1/3 rounded" style="background: var(--c-accent)" />
                </div>
              </div>
              <div class="flex items-center justify-between px-3 py-2" style="background: var(--c-surface)">
                <span class="font-medium">{{ th.label }}</span>
                <Check v-if="s.ui.theme === th.id" :size="15" style="color: var(--c-accent)" />
              </div>
            </button>
          </div>
        </section>
        <section class="card flex items-center gap-3 p-4">
          <ToggleSwitch :model-value="s.ui.liteMode" :label="t.settings.lite" @update:model-value="store.setUi({ liteMode: $event })" />
          <span><span class="block font-medium">{{ t.settings.lite }}</span><span class="text-[12px] text-faint">{{ t.settings.liteHint }}</span></span>
        </section>
      </template>

      <template v-else-if="section === 'accounts'">
        <section class="card p-4">
          <ul class="flex flex-col gap-1.5">
            <li v-for="acc in s.accounts" :key="acc.id" class="flex items-center gap-3 rounded-lg bg-raised px-3 py-2">
              <span class="min-w-0 flex-1 truncate font-medium">{{ acc.name }}</span>
              <span class="badge">{{ t.account.offline }}</span>
              <span v-if="s.selectedAccountId === acc.id" class="badge badge-accent"><Check :size="12" />{{ t.settings.selected }}</span>
              <button v-else class="btn h-7" @click="tasks.guard(() => store.selectAccount(acc.id))">{{ t.settings.select }}</button>
              <button class="btn btn-ghost btn-icon h-7 text-muted hover:!text-danger" :aria-label="t.common.delete" @click="tasks.guard(() => store.removeAccount(acc.id))"><Trash2 :size="15" /></button>
            </li>
          </ul>
          <form class="mt-3 flex gap-2" @submit.prevent="addAccount">
            <input v-model="newName" class="input" :placeholder="t.settings.nickname" maxlength="16" :aria-label="t.settings.nickname" />
            <button class="btn btn-primary" type="submit" :disabled="!newName.trim()"><UserPlus :size="15" />{{ t.settings.addAccount }}</button>
          </form>
        </section>
        <p class="rounded-lg border border-dashed border-line px-4 py-3 text-muted">{{ t.settings.msSoon }}</p>
      </template>

      <section v-else class="card flex items-start gap-4 p-4">
        <img :src="mascot" alt="" class="pixelated size-20" />
        <div class="flex flex-col gap-1">
          <div class="font-mono text-xl font-bold">_k0t1k</div>
          <p class="text-muted">{{ t.settings.aboutText }}</p>
          <dl v-if="system" class="mt-2 grid grid-cols-[110px_1fr] gap-y-1 text-muted">
            <dt class="text-faint">{{ t.settings.version }}</dt><dd>{{ system.appVersion }}</dd>
            <dt class="text-faint">{{ t.settings.system }}</dt><dd>{{ system.os }}</dd>
            <dt class="text-faint">{{ t.settings.dataDir }}</dt><dd class="selectable font-mono text-[12px]">{{ system.dataDir }}</dd>
          </dl>
        </div>
      </section>
    </div>
  </div>
</template>
