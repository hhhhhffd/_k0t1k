<script setup lang="ts">
import { ArrowDown, ArrowUp, Pencil, Play, Plus, RefreshCw, Server as ServerIcon, Trash2 } from '@lucide/vue'
import { onMounted, reactive, ref, watch } from 'vue'
import * as api from '../../api/commands'
import type { ResourcePackPolicy, Server, ServerStatus } from '../../api/types'
import { t } from '../../i18n/ru'
import { useInstancesStore } from '../../stores/instances'
import { useSettingsStore } from '../../stores/settings'
import { useTasksStore } from '../../stores/tasks'
import { errorMessage } from '../../utils/format'
import ConfirmDialog from '../ConfirmDialog.vue'
import ModalDialog from '../ModalDialog.vue'
import SelectMenu from '../SelectMenu.vue'

type Ping = { state: 'pending' } | { state: 'ok'; status: ServerStatus } | { state: 'error'; message: string }

const props = defineProps<{ instanceId: string }>()
const tasks = useTasksStore()
const instances = useInstancesStore()
const settings = useSettingsStore()

const servers = ref<Server[]>([])
const pings = reactive<Record<number, Ping>>({})
const loading = ref(true)
/** `index` null adds a new server. */
const editing = ref<{ index: number | null; server: Server } | null>(null)
const deleting = ref<number | null>(null)

const policyOptions = (Object.keys(t.servers.policies) as ResourcePackPolicy[]).map((value) => ({ value, label: t.servers.policies[value] }))

// A newer round of pings supersedes an older one: indexes shift when the list changes.
let round = 0
function pingAll() {
  const current = ++round
  for (const key of Object.keys(pings)) delete pings[Number(key)]
  servers.value.forEach((server, index) => {
    pings[index] = { state: 'pending' }
    api.pingServer(server.address).then(
      (status) => {
        if (current === round) pings[index] = { state: 'ok', status }
      },
      (error: unknown) => {
        if (current === round) pings[index] = { state: 'error', message: errorMessage(error) }
      },
    )
  })
}

async function load() {
  loading.value = true
  servers.value = (await tasks.guard(() => api.listServers(props.instanceId))) ?? []
  loading.value = false
  pingAll()
}
onMounted(load)
watch(() => props.instanceId, load)

async function change(action: () => Promise<void>) {
  const done = await tasks.guard(async () => {
    await action()
    return true
  })
  if (done) await load()
}

const play = (address: string) =>
  void tasks.guard(() => instances.launch(props.instanceId, settings.settings?.selectedAccountId ?? undefined, address))

const move = (from: number, to: number) => change(() => api.moveServer(props.instanceId, from, to))

function startEdit(index: number | null) {
  editing.value = { index, server: index === null ? { name: '', address: '', resourcePack: 'prompt' } : { ...servers.value[index]! } }
}
async function saveEdit() {
  const edit = editing.value
  if (!edit || !edit.server.address.trim()) return
  editing.value = null
  const server = { ...edit.server, name: edit.server.name.trim() || t.servers.noName, address: edit.server.address.trim() }
  await change(() => (edit.index === null ? api.addServer(props.instanceId, server) : api.updateServer(props.instanceId, edit.index, server)))
}

async function confirmDelete() {
  const index = deleting.value
  deleting.value = null
  if (index !== null) await change(() => api.removeServer(props.instanceId, index))
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex items-center gap-2">
      <span class="mr-auto text-faint">{{ servers.length }}</span>
      <button class="btn" :disabled="!servers.length" @click="pingAll"><RefreshCw :size="15" />{{ t.servers.refresh }}</button>
      <button class="btn btn-primary" @click="startEdit(null)"><Plus :size="16" />{{ t.servers.add }}</button>
    </div>

    <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
    <p v-else-if="!servers.length" class="py-10 text-center text-faint">{{ t.servers.empty }}</p>
    <ul v-else class="flex flex-col gap-1.5">
      <li v-for="(s, i) in servers" :key="`${i}-${s.address}`" class="card flex items-center gap-3 px-3 py-2.5">
        <div class="flex flex-col">
          <button class="btn btn-ghost btn-icon !h-5 text-muted" :disabled="i === 0" :title="t.servers.moveUp" :aria-label="t.servers.moveUp" @click="move(i, i - 1)"><ArrowUp :size="13" /></button>
          <button class="btn btn-ghost btn-icon !h-5 text-muted" :disabled="i === servers.length - 1" :title="t.servers.moveDown" :aria-label="t.servers.moveDown" @click="move(i, i + 1)"><ArrowDown :size="13" /></button>
        </div>
        <img v-if="pings[i]?.state === 'ok' && pings[i].status.favicon" :src="pings[i].status.favicon" alt="" class="pixelated size-10 shrink-0 rounded-md" />
        <span v-else class="pix flex size-10 shrink-0 items-center justify-center bg-accent/15 text-accent"><ServerIcon :size="18" /></span>
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <span class="truncate font-semibold">{{ s.name }}</span>
            <span class="truncate text-[12px] text-faint">{{ s.address }}</span>
          </div>
          <div class="truncate text-[12px] text-muted">
            <template v-if="!pings[i] || pings[i].state === 'pending'">{{ t.servers.pinging }}</template>
            <span v-else-if="pings[i].state === 'error'" class="text-danger" :title="pings[i].message">{{ t.servers.unreachable }}: {{ pings[i].message }}</span>
            <template v-else>{{ pings[i].status.motd }}</template>
          </div>
        </div>
        <template v-if="pings[i]?.state === 'ok'">
          <span class="badge">{{ pings[i].status.online }}/{{ pings[i].status.max }} {{ t.servers.online }}</span>
          <span class="badge max-w-40 truncate" :title="pings[i].status.version">{{ pings[i].status.version }}</span>
          <span class="badge badge-accent">{{ pings[i].status.latencyMs }} мс</span>
        </template>
        <button class="btn" :title="t.servers.playOn" @click="play(s.address)"><Play :size="13" fill="currentColor" />{{ t.servers.play }}</button>
        <button class="btn btn-ghost btn-icon text-muted" :title="t.servers.edit" :aria-label="t.servers.edit" @click="startEdit(i)"><Pencil :size="15" /></button>
        <button class="btn btn-ghost btn-icon text-muted hover:!text-danger" :aria-label="t.common.delete" @click="deleting = i"><Trash2 :size="15" /></button>
      </li>
    </ul>
    <p class="text-[12px] text-faint">{{ t.servers.srvNote }}</p>

    <ModalDialog v-if="editing" :title="t.servers.editTitle" :width="440" @close="editing = null">
      <div class="flex flex-col gap-3">
        <label class="flex flex-col gap-1">
          <span class="label">{{ t.servers.name }}</span>
          <input v-model="editing.server.name" class="input" :placeholder="t.servers.noName" />
        </label>
        <label class="flex flex-col gap-1">
          <span class="label">{{ t.servers.address }}</span>
          <input v-model="editing.server.address" class="input" autofocus @keydown.enter="saveEdit" />
        </label>
        <div class="flex flex-col gap-1">
          <span class="label">{{ t.servers.resourcePack }}</span>
          <SelectMenu v-model="editing.server.resourcePack" :options="policyOptions" :aria-label="t.servers.resourcePack" />
        </div>
      </div>
      <template #footer>
        <button class="btn" @click="editing = null">{{ t.common.cancel }}</button>
        <button class="btn btn-primary" :disabled="!editing.server.address.trim()" @click="saveEdit">{{ t.common.save }}</button>
      </template>
    </ModalDialog>

    <ConfirmDialog v-if="deleting !== null" :title="t.servers.delete" :text="servers[deleting]?.name ?? ''" @confirm="confirmDelete" @close="deleting = null" />
  </div>
</template>
