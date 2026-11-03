<script setup lang="ts">
import { UserPlus } from '@lucide/vue'
import { ref } from 'vue'
import { t } from '../i18n'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'
import AccountLoginDialog from './AccountLoginDialog.vue'

/** The ways to get a first account: Microsoft, ely.by or an offline nickname. `added` fires once an account exists. */
const emit = defineEmits<{ added: [] }>()
const store = useSettingsStore()
const tasks = useTasksStore()

const loginKind = ref<'microsoft' | 'elyby' | null>(null)
const nickname = ref('')

function loginClosed() {
  loginKind.value = null
  // The dialog also closes on cancel or failure; only a signed-in account counts.
  if (store.settings?.selectedAccountId) emit('added')
}

async function addOffline() {
  const name = nickname.value.trim()
  if (!name) return
  if ((await tasks.guard(() => store.addAccount(name))) !== undefined) emit('added')
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex flex-wrap gap-2">
      <button class="btn btn-primary" @click="loginKind = 'microsoft'"><UserPlus :size="15" />{{ t.account.signInMicrosoft }}</button>
      <button class="btn" @click="loginKind = 'elyby'"><UserPlus :size="15" />{{ t.account.signInElyby }}</button>
    </div>
    <div>
      <p class="mb-1.5 text-muted">{{ t.account.offlineHint }}</p>
      <form class="flex gap-2" @submit.prevent="addOffline">
        <input v-model="nickname" class="input" :placeholder="t.settings.nickname" maxlength="16" :aria-label="t.settings.nickname" />
        <button class="btn" type="submit" :disabled="!nickname.trim()">{{ t.account.playOffline }}</button>
      </form>
    </div>
    <AccountLoginDialog v-if="loginKind" :kind="loginKind" @close="loginClosed" />
  </div>
</template>
