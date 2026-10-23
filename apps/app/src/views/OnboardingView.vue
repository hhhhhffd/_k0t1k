<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { FolderOpen, HardDriveDownload, Pin } from '@lucide/vue'
import { ref } from 'vue'
import { isTauri, keepPortableHere, relocate } from '../api/commands'
import type { OnboardingInfo } from '../api/types'
import mascot from '../assets/mascot.png'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import { t } from '../i18n/ru'
import { useTasksStore } from '../stores/tasks'

const props = defineProps<{ info: OnboardingInfo }>()
const tasks = useTasksStore()

const desktopShortcut = ref(true)
const startMenuShortcut = ref(true)
const dontAsk = ref(false)
// set once a command succeeded: the launcher is about to restart and the window to close
const busy = ref<'install' | 'keep' | null>(null)

async function install(target: string) {
  busy.value = 'install'
  const done = await tasks.guard(async () => {
    await relocate(target, desktopShortcut.value, startMenuShortcut.value)
    return true
  })
  if (!done) busy.value = null
}

async function pickFolder() {
  // Outside Tauri there is no native dialog; the mock accepts a fake path.
  const picked = isTauri ? await open({ directory: true, multiple: false, title: t.onboarding.pickTitle }) : 'D:\\Games'
  if (typeof picked === 'string') await install(picked)
}

async function keepHere() {
  busy.value = 'keep'
  const done = await tasks.guard(async () => {
    await keepPortableHere(dontAsk.value)
    return true
  })
  if (!done) busy.value = null
}
</script>

<template>
  <div class="flex min-h-0 flex-1 items-center justify-center overflow-y-auto p-6">
    <div class="flex w-full max-w-xl flex-col gap-5">
      <div class="flex items-center gap-4">
        <img :src="mascot" alt="" class="pixelated size-20 shrink-0" />
        <div class="flex flex-col gap-1">
          <h1 class="font-mono text-2xl font-bold tracking-tight">{{ t.onboarding.title }}</h1>
          <p class="text-muted">{{ props.info.reason === 'temp' ? t.onboarding.whyTemp : t.onboarding.why }}</p>
        </div>
      </div>

      <section class="card flex flex-col gap-4 p-4" :aria-busy="busy !== null">
        <div class="flex flex-col gap-1">
          <div class="flex items-center gap-2 font-semibold">
            <HardDriveDownload :size="16" />{{ t.onboarding.install }}
            <span class="badge badge-accent">{{ t.onboarding.recommended }}</span>
          </div>
          <p class="text-[12px] text-faint">{{ t.onboarding.installHint }}</p>
          <p class="selectable font-mono text-[12px] break-all">{{ props.info.defaultTarget }}</p>
        </div>
        <label class="flex items-center gap-3">
          <ToggleSwitch v-model="desktopShortcut" :label="t.onboarding.desktopShortcut" :disabled="busy !== null" />{{ t.onboarding.desktopShortcut }}
        </label>
        <label class="flex items-center gap-3">
          <ToggleSwitch v-model="startMenuShortcut" :label="t.onboarding.startMenuShortcut" :disabled="busy !== null" />{{ t.onboarding.startMenuShortcut }}
        </label>
        <div class="flex flex-wrap items-center gap-3">
          <button class="btn btn-primary" :disabled="busy !== null" @click="install(props.info.defaultTarget)">
            <HardDriveDownload :size="15" />{{ busy === 'install' ? t.onboarding.working : t.onboarding.install }}
          </button>
          <button class="btn" :disabled="busy !== null" @click="pickFolder"><FolderOpen :size="15" />{{ t.onboarding.pickFolder }}</button>
        </div>
        <p class="text-[12px] text-faint">{{ t.onboarding.pickFolderHint }}</p>
      </section>

      <section v-if="props.info.reason === 'folder'" class="card flex flex-col gap-3 p-4">
        <div class="flex items-center gap-2 font-semibold"><Pin :size="16" />{{ t.onboarding.keepHere }}</div>
        <p class="text-[12px] text-faint">{{ t.onboarding.keepHereHint }}</p>
        <p class="selectable font-mono text-[12px] break-all">{{ props.info.exeDir }}</p>
        <label class="flex items-center gap-3">
          <ToggleSwitch v-model="dontAsk" :label="t.onboarding.dontAsk" :disabled="busy !== null" />{{ t.onboarding.dontAsk }}
        </label>
        <div>
          <button class="btn" :disabled="busy !== null" @click="keepHere">{{ busy === 'keep' ? t.onboarding.restarting : t.onboarding.keepHere }}</button>
        </div>
      </section>
    </div>
  </div>
</template>
