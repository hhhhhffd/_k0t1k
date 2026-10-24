<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { RouterView } from 'vue-router'
import { getOnboarding, getRelocationReport } from './api/commands'
import type { OnboardingInfo, RelocationReport } from './api/types'
import AppSidebar from './components/AppSidebar.vue'
import BlockedFilesDialog from './components/BlockedFilesDialog.vue'
import CrashDialog from './components/CrashDialog.vue'
import RelocationNotice from './components/RelocationNotice.vue'
import TitleBar from './components/TitleBar.vue'
import TaskBar from './components/TaskBar.vue'
import ToastHost from './components/ToastHost.vue'
import { t } from './i18n/ru'
import { useInstancesStore } from './stores/instances'
import { useSettingsStore } from './stores/settings'
import { useTasksStore } from './stores/tasks'
import OnboardingView from './views/OnboardingView.vue'

const settings = useSettingsStore()
const instances = useInstancesStore()
const tasks = useTasksStore()

// `undefined` until the backend has said whether the launcher still has to ask where to live
const onboarding = ref<OnboardingInfo | null | undefined>(undefined)
const relocationReport = ref<RelocationReport | null>(null)

onMounted(async () => {
  // tasks first so no progress event emitted during startup is missed
  await tasks.init()
  onboarding.value = (await tasks.guard(getOnboarding)) ?? null
  if (onboarding.value) return
  await tasks.guard(() => Promise.all([settings.load(), instances.init()]))
  const report = await tasks.guard(getRelocationReport)
  if (!report) return
  if (report.problems.length) relocationReport.value = report
  else if (report.installedTo) tasks.toast(`${t.onboarding.reportOk}: ${report.installedTo}`, 'ok')
})
</script>

<template>
  <div class="flex h-full flex-col">
    <TitleBar />
    <OnboardingView v-if="onboarding" :info="onboarding" />
    <div v-else-if="onboarding === null" class="flex min-h-0 flex-1">
      <AppSidebar />
      <main class="min-w-0 flex-1 overflow-y-auto">
        <RouterView />
      </main>
    </div>
    <TaskBar v-if="onboarding === null" />
    <RelocationNotice v-if="relocationReport" :report="relocationReport" @close="relocationReport = null" />
    <BlockedFilesDialog v-if="tasks.blocked.length" :files="tasks.blocked" :instance-id="tasks.blockedInstanceId" @close="tasks.blocked = []" />
    <CrashDialog v-if="onboarding === null" />
    <ToastHost />
  </div>
</template>
