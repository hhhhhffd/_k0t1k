<script setup lang="ts">
import { t } from '../i18n'
import AccountSetup from './AccountSetup.vue'
import ModalDialog from './ModalDialog.vue'

/** Shown instead of launching when there is no account; `added` means one now exists and the launch can go on. */
const emit = defineEmits<{ close: []; added: [] }>()
</script>

<template>
  <ModalDialog :title="t.account.needTitle" size="sm" @close="emit('close')">
    <p class="mb-4 text-muted">{{ t.account.needHint }}</p>
    <AccountSetup @added="emit('added')" />
    <template #footer>
      <button class="btn" @click="emit('close')">{{ t.common.cancel }}</button>
    </template>
  </ModalDialog>
</template>
