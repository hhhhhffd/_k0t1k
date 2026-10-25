<script setup lang="ts">
import { ArrowUpCircle, ArrowUpFromLine, Lock, LockOpen, Plus, RefreshCw, Search, ShieldCheck, Trash2, Wrench } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import * as api from '../../api/commands'
import type { AvailableUpdate, BlockedFile, JarScan, LocalResource, LocalResourceKind } from '../../api/types'
import { t } from '../../i18n'
import { usePacksStore, type PackResult } from '../../stores/packs'
import { badgeOf, useRatscanStore } from '../../stores/ratscan'
import { useTasksStore } from '../../stores/tasks'
import { errorMessage, formatBytes, originPlatform } from '../../utils/format'
import ConfirmDialog from '../ConfirmDialog.vue'
import ContentDialog from '../ContentDialog.vue'
import ModalDialog from '../ModalDialog.vue'
import ScanDetailsDialog from '../ScanDetailsDialog.vue'
import ToggleSwitch from '../ToggleSwitch.vue'

const props = defineProps<{ instanceId: string; kind: LocalResourceKind; sharedNote?: string }>()
const showContent = ref(false)
const tasks = useTasksStore()
const ratscan = useRatscanStore()
const packs = usePacksStore()

const resources = ref<LocalResource[]>([])
const loading = ref(true)
const filter = ref('')
const updates = ref<Map<string, AvailableUpdate>>(new Map())
const checking = ref(false)
const toDelete = ref<LocalResource | null>(null)
/** Progress of "update all"; `null` when it is not running. */
const bulk = ref<{ done: number; total: number } | null>(null)

const visible = computed(() => {
  const needle = filter.value.trim().toLowerCase()
  return resources.value.filter(
    (r) => !needle || r.fileName.toLowerCase().includes(needle) || r.metadata?.name.toLowerCase().includes(needle),
  )
})

async function load() {
  loading.value = true
  updates.value = new Map()
  resources.value = (await tasks.guard(() => api.listLocalResources(props.instanceId, props.kind))) ?? []
  loading.value = false
  // Badges are an extra: a failure to read them must not hide the list.
  const extras = props.kind === 'mod' ? ratscan.refresh(props.instanceId) : props.kind === 'resourcePack' ? packs.refresh(props.instanceId) : undefined
  extras?.catch((e: unknown) => console.warn('cannot read the plugin results', e))
}
onMounted(load)
watch(() => [props.instanceId, props.kind], load)
watch(() => packs.changed, load)

// ---- RatScan (mods) ----
const jarsByName = computed(() => new Map((ratscan.jars[props.instanceId] ?? []).map((jar) => [jar.fileName, jar])))
const scanning = computed(() => ratscan.progress?.instanceId === props.instanceId)
const scanDetails = ref<JarScan | null>(null)
async function scanMods() {
  const scanned = await tasks.guard(() => ratscan.scan(props.instanceId))
  if (scanned !== undefined) tasks.toast(scanned ? t.scan.done(scanned) : t.scan.nothingNew, 'ok')
}
const badgeClass = { clean: 'badge-ok', suspicious: 'badge-warn', dangerous: 'badge-danger', unknown: '' } as const
const badgeText = computed(() => ({ clean: t.scan.clean, suspicious: t.scan.suspicious, dangerous: t.scan.dangerous, unknown: t.scan.unknown }))

// ---- packconv (resource packs) ----
const packBusy = ref<string | null>(null)
const packResult = ref<(PackResult & { title: string }) | null>(null)
const isZipPack = (r: LocalResource) => props.kind === 'resourcePack' && /\.zip(\.disabled)?$/i.test(r.fileName)
async function convertPack(r: LocalResource) {
  packBusy.value = r.fileName
  const result = await tasks.guard(async () => {
    const target = await packs.targetFor(props.instanceId)
    if (!target) throw new Error(t.packs.noTarget)
    return { ...(await packs.convert(props.instanceId, r.fileName, target)), title: t.packs.convertTo(target) }
  })
  packBusy.value = null
  if (result) packResult.value = result
  await load()
}
async function repairPack(r: LocalResource) {
  packBusy.value = r.fileName
  const result = await tasks.guard(async () => ({ ...(await packs.repair(props.instanceId, r.fileName)), title: t.packs.repair }))
  packBusy.value = null
  if (result) packResult.value = result
  await load()
}
const targetVersion = ref<string | null>(null)
watch(
  () => [props.instanceId, props.kind],
  async () => {
    targetVersion.value = props.kind === 'resourcePack' ? await packs.targetFor(props.instanceId).catch(() => null) : null
  },
  { immediate: true },
)

async function toggle(r: LocalResource, enabled: boolean) {
  const ok = await tasks.guard(async () => {
    await api.toggleResource(props.instanceId, props.kind, r.fileName, enabled)
    return true
  })
  if (ok) await load()
}

async function confirmDelete() {
  const r = toDelete.value
  toDelete.value = null
  if (!r) return
  const ok = await tasks.guard(async () => {
    await api.deleteResource(props.instanceId, props.kind, r.fileName)
    return true
  })
  if (ok) await load()
}

async function checkUpdates() {
  checking.value = true
  const found = await tasks.guard(() => api.checkUpdates(props.instanceId, props.kind))
  checking.value = false
  if (!found) return
  updates.value = new Map(found.map((u) => [u.fileName, u]))
  if (!found.length) tasks.toast(t.instance.noUpdates, 'ok')
}

async function applyUpdate(r: LocalResource) {
  const update = updates.value.get(r.fileName)
  if (!update) return
  const report = await tasks.guard(() => api.installVersion(props.instanceId, update.latest, props.kind))
  if (!report) return
  tasks.showBlocked(report.blocked, props.instanceId)
  // core replaces the old file of an already installed project, so nothing is deleted here
  tasks.toast(t.instance.updated, 'ok')
  noteAdded(report.installed.map((i) => i.fileName))
  await load()
}

async function updateAll() {
  const pending = [...updates.value.values()]
  bulk.value = { done: 0, total: pending.length }
  const blocked: BlockedFile[] = []
  let updated = 0
  for (const update of pending) {
    try {
      const report = await api.installVersion(props.instanceId, update.latest, props.kind)
      blocked.push(...report.blocked)
      noteAdded(report.installed.map((i) => i.fileName))
      updated++
    } catch (e) {
      tasks.toast(t.updates.updateFailed(update.name, errorMessage(e)), 'error')
    }
    bulk.value.done++
  }
  bulk.value = null
  tasks.showBlocked(blocked, props.instanceId)
  if (updated) tasks.toast(t.updates.updatedAll(updated), 'ok')
  await load()
}

/** New files: scan jars in the background, offer to convert packs made for another version. */
function noteAdded(fileNames: string[]) {
  if (props.kind === 'mod') ratscan.scanAdded(props.instanceId, fileNames)
  else if (props.kind === 'resourcePack') void packs.offerConversion(props.instanceId, fileNames)
}

const isLocked = (r: LocalResource) => r.origin?.['x-prismlauncher-lock-update'] === true

async function toggleLock(r: LocalResource) {
  const locked = !isLocked(r)
  const ok = await tasks.guard(async () => {
    await api.setUpdateLock(props.instanceId, props.kind, r.fileName, locked)
    return true
  })
  if (!ok || !r.origin) return
  r.origin['x-prismlauncher-lock-update'] = locked
  if (locked) {
    const next = new Map(updates.value)
    next.delete(r.fileName)
    updates.value = next
  }
}

function closeContent() {
  showContent.value = false
  void load()
}
</script>

<template>
  <div class="flex flex-col gap-3">
    <p v-if="sharedNote" class="flex items-center gap-2 text-muted">
      <span class="badge badge-accent">{{ t.sharedFolder.badge }}</span>{{ sharedNote }}
    </p>
    <div class="flex items-center gap-2">
      <div class="relative max-w-xs flex-1">
        <Search :size="15" class="pointer-events-none absolute top-2.5 left-2.5 text-faint" />
        <input v-model="filter" class="input !pl-8" :placeholder="t.instance.filterPlaceholder" :aria-label="t.instance.filterPlaceholder" />
      </div>
      <span class="mr-auto text-faint">{{ resources.length }}</span>
      <button v-if="updates.size" class="btn btn-primary" :disabled="!!bulk" @click="updateAll">
        <ArrowUpFromLine :size="15" />{{ bulk ? t.updates.updatingAll(bulk.done, bulk.total) : `${t.updates.updateAll} (${updates.size})` }}
      </button>
      <button v-if="kind === 'mod'" class="btn" :disabled="scanning || !resources.length" @click="scanMods">
        <ShieldCheck :size="15" :class="scanning ? 'animate-pulse' : ''" />{{ scanning ? t.scan.scanning(ratscan.progress!.done, ratscan.progress!.total) : t.scan.scanMods }}
      </button>
      <button class="btn" :disabled="checking || !!bulk || !resources.length" @click="checkUpdates">
        <RefreshCw :size="15" :class="checking ? 'animate-spin' : ''" />{{ checking ? t.instance.checking : t.instance.checkUpdates }}
      </button>
      <button class="btn btn-primary" @click="showContent = true"><Plus :size="16" />{{ t.instance.addContent }}</button>
    </div>

    <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
    <p v-else-if="!visible.length" class="py-10 text-center text-faint">{{ t.instance.noResources }}</p>
    <ul v-else class="flex flex-col gap-1.5">
      <li v-for="r in visible" :key="r.fileName" class="card flex items-center gap-3 px-3 py-2" :class="r.enabled ? '' : 'opacity-60'">
        <ToggleSwitch :model-value="r.enabled" :label="t.common.enabled" @update:model-value="toggle(r, $event)" />
        <img v-if="r.metadata?.icon" :src="r.metadata.icon" alt="" class="pixelated size-9 rounded-md" />
        <span v-else class="pix flex size-9 shrink-0 items-center justify-center bg-accent/15 font-bold text-accent">
          {{ (r.metadata?.name ?? r.fileName).charAt(0).toUpperCase() }}
        </span>
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <span class="truncate font-semibold">{{ r.metadata?.name ?? r.fileName }}</span>
            <span v-if="r.metadata?.version" class="badge">{{ r.metadata.version }}</span>
            <span v-if="!r.origin" class="badge" :title="t.instance.unknownOrigin">?</span>
            <span v-else class="badge badge-accent">{{ originPlatform(r.origin) === 'modrinth' ? 'Modrinth' : 'CurseForge' }}</span>
            <button
              v-if="kind === 'mod' && jarsByName.get(r.fileName)?.scan"
              class="badge cursor-pointer"
              :class="badgeClass[badgeOf(jarsByName.get(r.fileName)!.scan!.verdict)]"
              :title="t.scan.details"
              @click="scanDetails = jarsByName.get(r.fileName)!"
            >{{ badgeText[badgeOf(jarsByName.get(r.fileName)!.scan!.verdict)] }}</button>
            <span
              v-if="kind === 'resourcePack' && packs.compat[instanceId]?.[r.fileName]?.mismatch"
              class="badge badge-warn"
              :title="t.packs.mismatchHint"
            >{{ t.packs.madeFor(packs.compat[instanceId][r.fileName].madeFor ?? '?') }}</span>
          </div>
          <div class="truncate text-[12px] text-faint">{{ r.fileName }} · {{ formatBytes(r.size) }}<template v-if="r.metadata?.authors"> · {{ r.metadata.authors.join(', ') }}</template></div>
        </div>
        <button
          v-if="r.origin"
          class="btn btn-ghost btn-icon"
          :class="isLocked(r) ? 'text-warn' : 'text-faint'"
          :title="isLocked(r) ? t.updates.unlock : t.updates.lock"
          :aria-label="isLocked(r) ? t.updates.unlock : t.updates.lock"
          :aria-pressed="isLocked(r)"
          @click="toggleLock(r)"
        >
          <component :is="isLocked(r) ? Lock : LockOpen" :size="15" />
        </button>
        <template v-if="isZipPack(r)">
          <button
            v-if="packs.compat[instanceId]?.[r.fileName]?.mismatch && targetVersion"
            class="btn h-7"
            :disabled="packBusy !== null"
            @click="convertPack(r)"
          >{{ packBusy === r.fileName ? t.packs.working : t.packs.convertTo(targetVersion) }}</button>
          <button class="btn btn-ghost btn-icon text-muted" :disabled="packBusy !== null" :title="t.packs.repairHint" :aria-label="t.packs.repair" @click="repairPack(r)">
            <Wrench :size="15" />
          </button>
        </template>
        <button v-if="updates.has(r.fileName)" class="btn btn-primary h-7" :disabled="!!bulk" @click="applyUpdate(r)">
          <ArrowUpCircle :size="14" />{{ t.instance.updateTo }} {{ updates.get(r.fileName)!.latest.versionNumber }}
        </button>
        <button class="btn btn-ghost btn-icon text-muted hover:!text-danger" :aria-label="t.common.delete" @click="toDelete = r"><Trash2 :size="15" /></button>
      </li>
    </ul>

    <ConfirmDialog
      v-if="toDelete"
      :title="t.instance.deleteResource"
      :text="toDelete.fileName"
      @confirm="confirmDelete"
      @close="toDelete = null"
    />
    <ScanDetailsDialog v-if="scanDetails" :instance-id="instanceId" :jar="scanDetails" @close="scanDetails = null" />
    <ModalDialog v-if="packResult" :title="packResult.title" :width="480" @close="packResult = null">
      <div class="flex flex-col gap-2">
        <p>{{ t.packs.saved(packResult.savedAs) }}</p>
        <p class="text-muted">{{ packResult.summary }}</p>
        <ul v-if="packResult.warnings.length" class="selectable max-h-48 list-disc overflow-y-auto pl-5 text-[12px] text-warn">
          <li v-for="(warning, i) in packResult.warnings" :key="i">{{ warning }}</li>
        </ul>
      </div>
      <template #footer><button class="btn btn-primary" @click="packResult = null">{{ t.common.close }}</button></template>
    </ModalDialog>
    <ContentDialog v-if="showContent" :instance-id="instanceId" :kind="kind" @close="closeContent" />
  </div>
</template>
