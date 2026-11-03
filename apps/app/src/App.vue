<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { RouterView } from 'vue-router'
import { getOnboarding, getRelocationReport } from './api/commands'
import type { OnboardingInfo, RelocationReport } from './api/types'
import BlockedFilesDialog from './components/BlockedFilesDialog.vue'
import CrashDialog from './components/CrashDialog.vue'
import LaunchScanDialog from './components/LaunchScanDialog.vue'
import ModCompatDialog from './components/ModCompatDialog.vue'
import MainToolbar from './components/MainToolbar.vue'
import RelocationNotice from './components/RelocationNotice.vue'
import SizeGrip from './components/SizeGrip.vue'
import StatusBar from './components/StatusBar.vue'
import SteamNav from './components/SteamNav.vue'
import TitleBar from './components/TitleBar.vue'
import TaskBar from './components/TaskBar.vue'
import ToastHost from './components/ToastHost.vue'
import UpdateDialog from './components/UpdateDialog.vue'
import { language, t } from './i18n'
import { useCustomizationStore } from './stores/customization'
import { useInstancesStore } from './stores/instances'
import { useLayoutStore } from './stores/layout'
import { useSettingsStore } from './stores/settings'
import { useTasksStore } from './stores/tasks'
import { useUpdateStore } from './stores/update'
import OnboardingView from './views/OnboardingView.vue'
import WelcomeView from './views/WelcomeView.vue'
import SteamLibraryView from './views/SteamLibraryView.vue'

const settings = useSettingsStore()
const instances = useInstancesStore()
const tasks = useTasksStore()
const layout = useLayoutStore()
const update = useUpdateStore()
const customization = useCustomizationStore()

// `undefined` until the backend has said whether the launcher still has to ask where to live
const onboarding = ref<OnboardingInfo | null | undefined>(undefined)
const relocationReport = ref<RelocationReport | null>(null)
// Decided once after loading: adding an account or an instance during the welcome must not close it.
const welcome = ref(false)

onMounted(async () => {
  // tasks first so no progress event emitted during startup is missed
  await tasks.init()
  const pending = (await tasks.guard(getOnboarding)) ?? null
  if (pending) {
    onboarding.value = pending
    return
  }
  await tasks.guard(() => Promise.all([settings.load(), instances.init()]))
  // after the settings: theme files and the chosen cat or icon set apply on top of them
  await customization.refresh()
  const loaded = settings.settings
  welcome.value = !!loaded && !loaded.onboarding.welcomeDone && !loaded.accounts.length && !instances.instances.length
  // only now: showing the library first would flash it (and start its requests) before the welcome replaces it
  onboarding.value = null
  if (settings.settings?.updates.checkOnStartup) void update.check(true, true, true)
  const report = await tasks.guard(getRelocationReport)
  if (!report) return
  if (report.problems.length) relocationReport.value = report
  else if (report.installedTo) tasks.toast(`${t.onboarding.reportOk}: ${report.installedTo}`, 'ok')
})
</script>

<template>
  <!-- remounted on a language change, so values computed once in setup pick up the new dictionary -->
  <div :key="language" class="flex h-full flex-col">
    <TitleBar />
    <OnboardingView v-if="onboarding" :info="onboarding" />
    <WelcomeView v-else-if="onboarding === null && welcome" @done="welcome = false" />
    <template v-else-if="onboarding === null">
      <SteamNav v-if="layout.mode === 'steam'" />
      <MainToolbar v-else-if="layout.toolbar === 'top'" />
      <main class="min-h-0 flex-1 overflow-y-auto">
        <RouterView v-slot="{ Component, route }">
          <!-- the Steam-like library shows the instance pages inside itself -->
          <SteamLibraryView v-if="layout.mode === 'steam' && route.name !== 'settings' && route.name !== 'accounts'" />
          <component :is="Component" v-else />
        </RouterView>
      </main>
      <MainToolbar v-if="layout.mode !== 'steam' && layout.toolbar === 'bottom'" />
      <TaskBar />
      <StatusBar />
    </template>
    <RelocationNotice v-if="relocationReport" :report="relocationReport" @close="relocationReport = null" />
    <BlockedFilesDialog v-if="tasks.blocked.length" :files="tasks.blocked" :instance-id="tasks.blockedInstanceId" @close="tasks.blocked = []" />
    <CrashDialog v-if="onboarding === null" />
    <ModCompatDialog />
    <LaunchScanDialog />
    <UpdateDialog v-if="update.prompting && update.release" />
    <ToastHost />
    <SizeGrip />
  </div>
</template>
