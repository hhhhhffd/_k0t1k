<script setup lang="ts">
import { Check, Copy, ExternalLink } from '@lucide/vue'
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { cancelMicrosoftLogin, elybyLogin, finishMicrosoftLogin, startMicrosoftLogin } from '../api/commands'
import type { DeviceCodeInfo } from '../api/types'
import { t } from '../i18n'
import { useSettingsStore } from '../stores/settings'
import { openExternal } from '../utils/external'
import ModalDialog from './ModalDialog.vue'

const props = defineProps<{ kind: 'microsoft' | 'elyby' }>()
const emit = defineEmits<{ close: [] }>()

const store = useSettingsStore()
const error = ref('')

// Microsoft: the backend returns the device code at once and then waits until it is entered.
const device = ref<DeviceCodeInfo | null>(null)
const copied = ref(false)
let closed = false

async function copyCode() {
  if (!device.value) return
  try {
    await navigator.clipboard.writeText(device.value.userCode)
    copied.value = true
    setTimeout(() => (copied.value = false), 1500)
  } catch {
    // no clipboard access: the code stays visible for typing
  }
}

async function signInMicrosoft() {
  try {
    const code = await startMicrosoftLogin()
    if (closed) {
      // The dialog went away while the code was requested; its cancel found nothing to stop.
      await cancelMicrosoftLogin()
      return
    }
    device.value = code
    void openExternal(device.value.verificationUri).catch(() => {})
    await finishMicrosoftLogin()
    await store.load()
    emit('close')
  } catch (e) {
    if (!closed) error.value = String(e)
  }
}

// ely.by: login and password, then the two-factor code when the account has one.
const username = ref('')
const password = ref('')
const totp = ref('')
const needsTotp = ref(false)
const busy = ref(false)

async function signInElyby() {
  error.value = ''
  busy.value = true
  try {
    const result = await elybyLogin(username.value, password.value, needsTotp.value ? totp.value : null)
    if (result.twoFactorRequired) {
      needsTotp.value = true
      return
    }
    await store.load()
    emit('close')
  } catch (e) {
    error.value = String(e)
  } finally {
    busy.value = false
  }
}

onMounted(() => {
  if (props.kind === 'microsoft') void signInMicrosoft()
})
onBeforeUnmount(() => {
  closed = true
  if (props.kind === 'microsoft') void cancelMicrosoftLogin().catch(() => {})
})
</script>

<template>
  <ModalDialog :title="kind === 'microsoft' ? t.account.msTitle : t.account.elybyTitle" :width="420" @close="emit('close')">
    <div v-if="kind === 'microsoft'" class="flex flex-col items-center gap-3 text-center">
      <p v-if="error" class="text-danger">{{ error }}</p>
      <template v-else-if="device">
        <p class="text-muted">{{ t.account.msEnterCode }}</p>
        <div class="flex items-center gap-2">
          <span class="selectable font-mono text-3xl font-bold tracking-widest">{{ device.userCode }}</span>
          <button class="btn btn-ghost btn-icon" :aria-label="t.account.copyCode" @click="copyCode">
            <Check v-if="copied" :size="16" /><Copy v-else :size="16" />
          </button>
        </div>
        <button class="btn" @click="openExternal(device.verificationUri)"><ExternalLink :size="15" />{{ t.account.openPage }}</button>
        <p class="text-faint">{{ t.account.msWaiting }}</p>
      </template>
      <p v-else class="text-muted">{{ t.account.msStarting }}</p>
    </div>

    <form v-else class="flex flex-col gap-3" @submit.prevent="signInElyby">
      <input v-model="username" class="input" :placeholder="t.account.elybyLogin" :aria-label="t.account.elybyLogin" autocomplete="username" :disabled="needsTotp || busy" />
      <input v-model="password" class="input" type="password" :placeholder="t.account.elybyPassword" :aria-label="t.account.elybyPassword" autocomplete="current-password" :disabled="needsTotp || busy" />
      <template v-if="needsTotp">
        <p class="text-muted">{{ t.account.elybyTotpHint }}</p>
        <input v-model="totp" class="input" inputmode="numeric" maxlength="8" :placeholder="t.account.elybyTotp" :aria-label="t.account.elybyTotp" autocomplete="one-time-code" :disabled="busy" />
      </template>
      <p v-if="error" class="text-danger">{{ error }}</p>
      <button class="btn btn-primary self-end" type="submit" :disabled="busy || !username.trim() || !password || (needsTotp && !totp.trim())">{{ t.account.signIn }}</button>
    </form>
  </ModalDialog>
</template>
