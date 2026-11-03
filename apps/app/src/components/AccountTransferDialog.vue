<script setup lang="ts">
import { open, save } from '@tauri-apps/plugin-dialog'
import { Check, Copy, Dices, Eye, EyeOff, TriangleAlert } from '@lucide/vue'
import { computed, ref } from 'vue'
import { exportAccount, importAccount, isTauri } from '../api/commands'
import type { Account } from '../api/types'
import { t } from '../i18n'
import { useSettingsStore } from '../stores/settings'
import ModalDialog from './ModalDialog.vue'

/** Export of one account's sign-in to a password-encrypted file (`account` given), or import of such a file. */
const props = defineProps<{ account?: Account }>()
const emit = defineEmits<{ close: [] }>()

// Mirrors MIN_PASSWORD_LEN in the core, which enforces it.
const MIN_PASSWORD_LENGTH = 10
const EXTENSION = 'k0t1k-auth'
const filters = [{ name: t.account.transferFilter, extensions: [EXTENSION] }]

const store = useSettingsStore()
const password = ref('')
const shown = ref(false)
const copied = ref(false)
const file = ref('')
const error = ref('')
const busy = ref(false)
const done = ref('')
/** The imported account is already signed in: waiting for the user to confirm the replacement. */
const replacing = ref<Account | null>(null)

const canSubmit = computed(() => !busy.value && (props.account ? password.value.length >= MIN_PASSWORD_LENGTH : !!file.value && !!password.value))

function submit() {
  if (!canSubmit.value) return
  if (props.account) void doExport()
  else void doImport(false)
}

function generatePassword() {
  // No look-alike characters (0/O, 1/l/I): the password is often retyped on the other PC.
  const alphabet = 'ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789'
  const limit = 256 - (256 % alphabet.length)
  const chars: string[] = []
  while (chars.length < 20) {
    for (const byte of crypto.getRandomValues(new Uint8Array(32))) {
      if (byte < limit && chars.length < 20) chars.push(alphabet[byte % alphabet.length])
    }
  }
  password.value = chars.join('')
  shown.value = true
}

async function copyPassword() {
  try {
    await navigator.clipboard.writeText(password.value)
    copied.value = true
    setTimeout(() => (copied.value = false), 1500)
  } catch {
    // no clipboard access: the password stays visible for copying by hand
  }
}

async function chooseFile() {
  const chosen = isTauri ? await open({ multiple: false, filters }) : 'C:\\Users\\Player\\Desktop\\account.k0t1k-auth'
  if (typeof chosen === 'string') file.value = chosen
}

async function run(action: () => Promise<void>) {
  error.value = ''
  busy.value = true
  try {
    await action()
  } catch (e) {
    error.value = String(e)
  } finally {
    busy.value = false
  }
}

function doExport() {
  const account = props.account
  if (!account) return
  return run(async () => {
    const destination = isTauri ? await save({ defaultPath: `${account.name}.${EXTENSION}`, filters }) : `C:\\Exports\\${account.name}.${EXTENSION}`
    if (!destination) return
    await exportAccount(account.id, password.value, destination)
    done.value = t.account.exportDone
  })
}

function doImport(overwrite: boolean) {
  return run(async () => {
    const result = await importAccount(password.value, file.value, overwrite)
    if (result.alreadyExists) {
      replacing.value = result.account
      return
    }
    await store.load()
    replacing.value = null
    done.value = t.account.importDone(result.account.name)
  })
}
</script>

<template>
  <ModalDialog :title="account ? t.account.exportTitle(account.name) : t.account.importTitle" size="sm" @close="emit('close')">
    <p v-if="done" class="anim-fade flex items-center justify-center gap-2 font-medium text-ok"><Check :size="16" />{{ done }}</p>
    <p v-else-if="replacing" class="text-muted">{{ t.account.importConfirm(replacing.name) }}</p>
    <form v-else class="flex flex-col gap-3" @submit.prevent="submit">
      <p v-if="account" class="flex items-start gap-2 rounded-lg bg-raised p-3 text-sm text-muted">
        <TriangleAlert :size="16" class="mt-0.5 shrink-0 text-warn" />{{ t.account.transferWarning }}
      </p>
      <div v-if="!account" class="flex items-center gap-2">
        <button class="btn shrink-0" type="button" :disabled="busy" @click="chooseFile">{{ t.account.chooseFile }}</button>
        <span class="selectable min-w-0 flex-1 truncate text-sm" :class="file ? '' : 'text-faint'" :title="file">{{ file || t.account.noFileChosen }}</span>
      </div>
      <div class="flex items-center gap-1">
        <input
          v-model="password"
          class="input min-w-0 flex-1"
          :class="shown ? 'selectable font-mono' : ''"
          :type="shown ? 'text' : 'password'"
          :placeholder="account ? t.account.transferPassword(MIN_PASSWORD_LENGTH) : t.account.importPassword"
          :aria-label="account ? t.account.transferPassword(MIN_PASSWORD_LENGTH) : t.account.importPassword"
          autocomplete="off"
          :disabled="busy"
        />
        <button class="btn btn-ghost btn-icon" type="button" :aria-label="shown ? t.account.hidePassword : t.account.showPassword" @click="shown = !shown">
          <EyeOff v-if="shown" :size="16" /><Eye v-else :size="16" />
        </button>
        <template v-if="account">
          <button class="btn btn-ghost btn-icon" type="button" :aria-label="t.account.generatePassword" :title="t.account.generatePassword" @click="generatePassword"><Dices :size="16" /></button>
          <button class="btn btn-ghost btn-icon" type="button" :aria-label="t.account.copyPassword" :disabled="!password" @click="copyPassword">
            <Check v-if="copied" :size="16" /><Copy v-else :size="16" />
          </button>
        </template>
      </div>
      <p v-if="account" class="text-sm text-faint">{{ t.account.exportSaveHint }}</p>
      <button hidden type="submit"></button>
    </form>
    <p v-if="error" class="selectable mt-3 text-danger">{{ error }}</p>
    <template #footer>
      <template v-if="done">
        <button class="btn btn-primary" @click="emit('close')">{{ t.common.close }}</button>
      </template>
      <template v-else-if="replacing">
        <button class="btn" :disabled="busy" @click="emit('close')">{{ t.common.cancel }}</button>
        <button class="btn btn-danger" :disabled="busy" @click="doImport(true)">{{ t.account.replace }}</button>
      </template>
      <template v-else>
        <button class="btn" @click="emit('close')">{{ t.common.cancel }}</button>
        <button v-if="account" class="btn btn-primary" :disabled="!canSubmit" @click="submit">{{ t.account.exportAction }}</button>
        <button v-else class="btn btn-primary" :disabled="!canSubmit" @click="submit">{{ t.account.importAction }}</button>
      </template>
    </template>
  </ModalDialog>
</template>
