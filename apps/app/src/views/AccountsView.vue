<script setup lang="ts">
import { Check, FileDown, FileUp, Trash2, UserPlus } from '@lucide/vue'
import { ref } from 'vue'
import AccountLoginDialog from '../components/AccountLoginDialog.vue'
import AccountTransferDialog from '../components/AccountTransferDialog.vue'
import type { Account } from '../api/types'
import { t } from '../i18n'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'

/** The accounts page: the signed-in accounts, which one plays, and the ways to add one. */
const store = useSettingsStore()
const tasks = useTasksStore()

const newName = ref('')
const loginKind = ref<'microsoft' | 'elyby' | null>(null)
/** The account being exported, or `'import'` while the import dialog is open. */
const transfer = ref<Account | 'import' | null>(null)

async function addAccount() {
  const name = newName.value.trim()
  if (!name) return
  newName.value = ''
  await tasks.guard(() => store.addAccount(name))
}
</script>

<template>
  <div v-if="store.settings" class="mx-auto flex w-full max-w-3xl flex-col gap-4 p-6">
    <h1 class="text-xl font-bold">{{ t.account.title }}</h1>
    <section class="card p-4">
      <ul class="flex flex-col gap-1.5">
        <li v-for="acc in store.settings.accounts" :key="acc.id" class="flex items-center gap-3 rounded-lg bg-raised px-3 py-2">
          <span class="min-w-0 flex-1 truncate font-medium">{{ acc.name }}</span>
          <span class="badge">{{ t.account.kinds[acc.kind] }}</span>
          <span v-if="store.settings.selectedAccountId === acc.id" class="badge badge-accent"><Check :size="12" />{{ t.settings.selected }}</span>
          <button v-else class="btn h-7" @click="tasks.guard(() => store.selectAccount(acc.id))">{{ t.settings.select }}</button>
          <button v-if="acc.kind !== 'offline'" class="btn btn-ghost btn-icon h-7 text-muted" :aria-label="t.account.exportAction" :title="t.account.exportAction" @click="transfer = acc"><FileDown :size="15" /></button>
          <button class="btn btn-ghost btn-icon h-7 text-muted hover:!text-danger" :aria-label="t.common.delete" @click="tasks.guard(() => store.removeAccount(acc.id))"><Trash2 :size="15" /></button>
        </li>
      </ul>
      <div class="mt-3 flex flex-wrap gap-2">
        <button class="btn" @click="loginKind = 'microsoft'"><UserPlus :size="15" />{{ t.account.signInMicrosoft }}</button>
        <button class="btn" @click="loginKind = 'elyby'"><UserPlus :size="15" />{{ t.account.signInElyby }}</button>
        <button class="btn" @click="transfer = 'import'"><FileUp :size="15" />{{ t.account.importAction }}</button>
      </div>
      <form class="mt-3 flex gap-2" @submit.prevent="addAccount">
        <input v-model="newName" class="input" :placeholder="t.settings.nickname" maxlength="16" :aria-label="t.settings.nickname" />
        <button class="btn btn-primary" type="submit" :disabled="!newName.trim()"><UserPlus :size="15" />{{ t.settings.addAccount }}</button>
      </form>
    </section>
    <AccountLoginDialog v-if="loginKind" :kind="loginKind" @close="loginKind = null" />
    <AccountTransferDialog v-if="transfer" :account="transfer === 'import' ? undefined : transfer" @close="transfer = null" />
  </div>
</template>
