<script setup lang="ts">
import { ArrowDown, ArrowUp, Pencil, Play, Plus, RefreshCw, Server as ServerIcon, Trash2 } from '@lucide/vue'
import { computed, onMounted, reactive, ref, watch } from 'vue'
import * as api from '../../api/commands'
import type { ResourcePackPolicy, Server, ServerStatus } from '../../api/types'
import { t } from '../../i18n'
import { useInstancesStore } from '../../stores/instances'
import { useSettingsStore } from '../../stores/settings'
import { useTasksStore } from '../../stores/tasks'
import { errorMessage } from '../../utils/format'
import { useMarquee } from '../../utils/useMarquee'
import { useSelection } from '../../utils/useSelection'
import ActionPanel from '../ActionPanel.vue'
import type { PanelAction } from '../ActionPanel.vue'
import ConfirmDialog from '../ConfirmDialog.vue'
import ContextMenu from '../ContextMenu.vue'
import MarqueeBox from '../MarqueeBox.vue'
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
const deleting = ref<number[]>([])
/** Indexes of the selected servers, as strings: a server has no id of its own. */
const selected = ref(new Set<string>())
const selection = useSelection(selected)
const { box, onPointerDown } = useMarquee(selected)
const selectedIndexes = computed(() => [...selected.value].map(Number).sort((a, b) => a - b))
/** The server the single-server actions apply to: only when exactly one is selected. */
const current = computed(() => (selectedIndexes.value.length === 1 ? selectedIndexes.value[0]! : null))
const order = () => servers.value.map((_, i) => String(i))
function onKey(e: KeyboardEvent) {
  if (selection.onKey(e, order())) return
  if (e.key === 'Delete' && selectedIndexes.value.length) deleting.value = selectedIndexes.value
}

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
  selected.value = new Set([...selected.value].filter((i) => Number(i) < servers.value.length))
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

async function move(from: number, to: number) {
  await change(() => api.moveServer(props.instanceId, from, to))
  selected.value = new Set([String(to)])
}

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
  // From the last to the first, so the indexes still to delete stay valid.
  const doomed = [...deleting.value].sort((a, b) => b - a)
  deleting.value = []
  if (!doomed.length) return
  selected.value = new Set()
  await tasks.guard(async () => {
    for (const index of doomed) await api.removeServer(props.instanceId, index)
  })
  // Reload even after a failure: the servers removed before it must leave the list.
  await load()
}

function serverActions(index: number | null, doomed: number[]): PanelAction[] {
  const server = index === null ? undefined : servers.value[index]
  return [
    { label: t.servers.play, icon: Play, title: t.servers.playOn, disabled: !server, action: () => server && play(server.address) },
    { label: t.servers.edit, icon: Pencil, disabled: index === null, action: () => index !== null && startEdit(index) },
    { label: t.common.delete, icon: Trash2, danger: true, disabled: !doomed.length, action: () => (deleting.value = doomed) },
    { label: t.servers.moveUp, icon: ArrowUp, separatorBefore: true, disabled: !index, action: () => index && void move(index, index - 1) },
    {
      label: t.servers.moveDown,
      icon: ArrowDown,
      disabled: index === null || index >= servers.value.length - 1,
      action: () => index !== null && void move(index, index + 1),
    },
  ]
}
const panelActions = computed<PanelAction[]>(() => [
  { label: t.servers.add, icon: Plus, primary: true, action: () => startEdit(null) },
  ...serverActions(current.value, selectedIndexes.value),
])
const footerActions = computed<PanelAction[]>(() => [{ label: t.servers.refresh, icon: RefreshCw, disabled: !servers.value.length, action: pingAll }])

const menu = ref<{ x: number; y: number; items: PanelAction[] } | null>(null)
function openMenu(e: MouseEvent, index: number) {
  if (!selected.value.has(String(index))) selected.value = new Set([String(index)])
  // Only deleting makes sense for several servers at once.
  const items = selectedIndexes.value.length > 1 ? serverActions(null, selectedIndexes.value).filter((a) => a.danger) : serverActions(index, [index])
  menu.value = { x: e.clientX, y: e.clientY, items }
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex items-start gap-3">
      <div class="min-w-0 flex-1">
        <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
        <p v-else-if="!servers.length" class="py-10 text-center text-faint">{{ t.servers.empty }}</p>
        <div v-else class="overflow-hidden rounded-xl border border-line !outline-none" tabindex="-1" @pointerdown="onPointerDown" @dragstart.prevent @keydown="onKey">
          <div class="flex items-center gap-3 border-b border-line px-3 py-2 text-[12px] font-semibold text-faint">
            <span class="w-10 shrink-0" />
            <span class="min-w-0 flex-1">{{ t.servers.name }} · {{ servers.length }}</span>
            <span class="w-24 shrink-0">{{ t.servers.players }}</span>
            <span class="w-32 shrink-0">{{ t.servers.versionColumn }}</span>
            <span class="w-16 shrink-0 text-right">{{ t.servers.pingColumn }}</span>
          </div>
          <ul role="listbox" aria-multiselectable="true">
            <li
              v-for="(s, i) in servers"
              :key="`${i}-${s.address}`"
              role="option"
              :data-select-key="String(i)"
              :aria-selected="selected.has(String(i))"
              class="flex cursor-default items-center gap-3 border-b border-line/60 px-3 py-2 last:border-b-0"
              :class="selected.has(String(i)) ? 'bg-accent/12' : 'hover:bg-hover'"
              @click="selection.click(String(i), $event, order())"
              @dblclick="play(s.address)"
              @contextmenu.prevent="openMenu($event, i)"
            >
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
                <span class="w-24 shrink-0 truncate text-[12.5px] text-muted">{{ pings[i].status.online }}/{{ pings[i].status.max }}</span>
                <span class="w-32 shrink-0 truncate text-[12.5px] text-muted" :title="pings[i].status.version">{{ pings[i].status.version }}</span>
                <span class="w-16 shrink-0 text-right text-[12.5px] text-accent">{{ pings[i].status.latencyMs }} {{ t.common.ms }}</span>
              </template>
              <template v-else>
                <span class="w-24 shrink-0 text-faint">—</span>
                <span class="w-32 shrink-0 text-faint">—</span>
                <span class="w-16 shrink-0 text-right text-faint">—</span>
              </template>
            </li>
          </ul>
        </div>
        <p v-if="servers.length" class="mt-2 text-[12px] text-faint">{{ t.servers.selectionHint }}</p>
      </div>
      <MarqueeBox :box="box" />
      <ActionPanel :actions="panelActions" :footer="footerActions" />
    </div>

    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" :items="menu.items" @close="menu = null" />

    <ModalDialog v-if="editing" :title="t.servers.editTitle" size="sm" @close="editing = null">
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

    <ConfirmDialog
      v-if="deleting.length"
      :title="deleting.length > 1 ? t.servers.deleteMany(deleting.length) : t.servers.delete"
      :text="deleting.map((i) => servers[i]?.name ?? '').join(', ')"
      @confirm="confirmDelete"
      @close="deleting = []"
    />
  </div>
</template>
