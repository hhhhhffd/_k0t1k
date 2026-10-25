<script setup lang="ts">
import { Check, Copy, ExternalLink, X } from '@lucide/vue'
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { cancelMicrosoftLogin, elybyLogin, finishMicrosoftLogin, onMicrosoftLoginStage, startMicrosoftLogin } from '../api/commands'
import type { DeviceCodeInfo, LoginStage } from '../api/types'
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

const STEPS = ['waiting', 'microsoft', 'xbox', 'minecraft', 'profile'] as const
const stage = ref<LoginStage>('waiting')
const signedInName = ref('')
const codeConfirmed = computed(() => stage.value !== 'waiting')
const stepIndex = computed(() => (stage.value === 'done' ? STEPS.length : STEPS.indexOf(stage.value)))

function stepState(index: number): 'done' | 'active' | 'failed' | 'pending' {
  if (index < stepIndex.value) return 'done'
  if (index > stepIndex.value) return 'pending'
  return error.value ? 'failed' : 'active'
}

// Elapsed time since the code was shown; stops when the sign-in ends either way.
const elapsed = ref(0)
const clock = computed(() => `${String(Math.floor(elapsed.value / 60)).padStart(2, '0')}:${String(elapsed.value % 60).padStart(2, '0')}`)
let timer: ReturnType<typeof setInterval> | undefined
let unlisten: (() => void) | undefined
function stopClock() {
  clearInterval(timer)
  timer = undefined
}

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
    const started = Date.now()
    timer = setInterval(() => (elapsed.value = Math.floor((Date.now() - started) / 1000)), 1000)
    void openExternal(device.value.verificationUri).catch(() => {})
    const account = await finishMicrosoftLogin()
    stopClock()
    await store.load()
    if (closed) return
    stage.value = 'done'
    signedInName.value = account.name
    // Long enough to read the result before the dialog goes away.
    await new Promise((resolve) => setTimeout(resolve, 800))
    emit('close')
  } catch (e) {
    stopClock()
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

onMounted(async () => {
  if (props.kind !== 'microsoft') return
  // Subscribed before the sign-in starts so no stage is missed.
  const off = await onMicrosoftLoginStage((next) => (stage.value = next))
  if (closed) off()
  else unlisten = off
  void signInMicrosoft()
})
onBeforeUnmount(() => {
  closed = true
  stopClock()
  unlisten?.()
  if (props.kind === 'microsoft') void cancelMicrosoftLogin().catch(() => {})
})
</script>

<template>
  <ModalDialog :title="kind === 'microsoft' ? t.account.msTitle : t.account.elybyTitle" :width="420" @close="emit('close')">
    <div v-if="kind === 'microsoft'" class="flex flex-col gap-3">
      <p v-if="error && !device" class="text-center text-danger">{{ error }}</p>
      <p v-else-if="!device" class="text-center text-muted">{{ t.account.msStarting }}</p>
      <template v-else>
        <div class="fold" :data-folded="codeConfirmed">
          <div>
            <div class="flex flex-col items-center gap-3 pb-1 text-center">
              <p class="text-muted">{{ t.account.msEnterCode }}</p>
              <div class="flex items-center gap-2">
                <span class="selectable font-mono text-3xl font-bold tracking-widest">{{ device.userCode }}</span>
                <button class="btn btn-ghost btn-icon" :aria-label="t.account.copyCode" @click="copyCode">
                  <Check v-if="copied" :size="16" /><Copy v-else :size="16" />
                </button>
              </div>
              <button class="btn" @click="openExternal(device.verificationUri)"><ExternalLink :size="15" />{{ t.account.openPage }}</button>
            </div>
          </div>
        </div>

        <ol class="flex flex-col gap-2" role="list">
          <li v-for="(step, index) in STEPS" :key="step" class="flex items-start gap-2.5" :class="stepState(index) === 'pending' ? 'text-faint' : ''">
            <span class="mt-0.5 flex h-4 w-4 shrink-0 items-center justify-center">
              <Check v-if="stepState(index) === 'done'" :size="16" class="step-check text-ok" />
              <X v-else-if="stepState(index) === 'failed'" :size="16" class="step-check text-danger" />
              <span v-else-if="stepState(index) === 'active'" class="step-ring h-3.5 w-3.5" aria-hidden="true"></span>
              <span v-else class="h-1.5 w-1.5 rounded-full bg-current opacity-50" aria-hidden="true"></span>
            </span>
            <div class="min-w-0 flex-1">
              <div class="flex items-baseline justify-between gap-2">
                <span :class="stepState(index) === 'failed' ? 'text-danger' : stepState(index) === 'active' ? 'text-fg' : ''">{{ t.account.msSteps[step] }}</span>
                <span v-if="index === 0 && stepState(0) === 'active'" class="font-mono text-xs text-faint tabular-nums">{{ clock }}</span>
              </div>
              <p v-if="stepState(index) === 'failed'" class="selectable anim-fade mt-0.5 text-sm text-danger">{{ error }}</p>
            </div>
          </li>
        </ol>

        <p v-if="stage === 'done'" class="anim-fade flex items-center justify-center gap-2 font-medium text-ok">
          <Check :size="16" />{{ t.account.msSignedIn(signedInName) }}
        </p>
        <p v-else-if="!error && !codeConfirmed" class="text-center text-faint">{{ t.account.msWaiting }}</p>
      </template>
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
