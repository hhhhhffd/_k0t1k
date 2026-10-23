<script setup lang="ts">
import { onMounted } from 'vue'
import { RouterView } from 'vue-router'
import AppSidebar from './components/AppSidebar.vue'
import TaskBar from './components/TaskBar.vue'
import ToastHost from './components/ToastHost.vue'
import { useInstancesStore } from './stores/instances'
import { useSettingsStore } from './stores/settings'
import { useTasksStore } from './stores/tasks'

const settings = useSettingsStore()
const instances = useInstancesStore()
const tasks = useTasksStore()

onMounted(async () => {
  // tasks first so no progress event emitted during startup is missed
  await tasks.init()
  await tasks.guard(() => Promise.all([settings.load(), instances.init()]))
})
</script>

<template>
  <div class="flex h-full flex-col">
    <div class="flex min-h-0 flex-1">
      <AppSidebar />
      <main class="min-w-0 flex-1 overflow-y-auto">
        <RouterView />
      </main>
    </div>
    <TaskBar />
    <ToastHost />
  </div>
</template>
