<script lang="ts">
import type { AvailableUpdate } from '../../api/types'

/** Automatic update checks per `instance:kind`, so switching tabs does not query the catalogs again. */
const updateCache = new Map<string, { at: number; updates: AvailableUpdate[] }>()
const UPDATE_CHECK_TTL_MS = 10 * 60 * 1000
</script>

<script setup lang="ts">
import {
  ArrowDown,
  ArrowUp,
  ArrowUpCircle,
  ArrowUpFromLine,
  Ban,
  CircleCheck,
  Download,
  ExternalLink,
  FilePlus,
  FolderCog,
  FolderOpen,
  History,
  ListOrdered,
  Lock,
  LockOpen,
  RefreshCw,
  Repeat,
  Search,
  ShieldCheck,
  Trash2,
  TriangleAlert,
  Wrench,
} from '@lucide/vue'
import { open } from '@tauri-apps/plugin-dialog'
import { computed, onMounted, ref, watch } from 'vue'
import * as api from '../../api/commands'
import { isTauri } from '../../api/commands'
import type { BlockedFile, JarScan, LocalResource, LocalResourceKind } from '../../api/types'
import { t } from '../../i18n'
import { useInstancesStore } from '../../stores/instances'
import { usePacksStore, type PackResult } from '../../stores/packs'
import { useModCompatStore } from '../../stores/modCompat'
import { badgeOf, useRatscanStore } from '../../stores/ratscan'
import { useTasksStore } from '../../stores/tasks'
import { openExternal } from '../../utils/external'
import { errorMessage, formatBytes, minecraftVersionOf, formatRelative, originProject, plainText, PLATFORM_LABELS } from '../../utils/format'
import { addFilesToInstance } from '../../utils/addFiles'
import { useMarquee } from '../../utils/useMarquee'
import { useViewMode } from '../../utils/useViewMode'
import ActionPanel from '../ActionPanel.vue'
import type { PanelAction } from '../ActionPanel.vue'
import ChangeVersionDialog from '../ChangeVersionDialog.vue'
import ConfirmDialog from '../ConfirmDialog.vue'
import ContentDialog from '../ContentDialog.vue'
import ContextMenu from '../ContextMenu.vue'
import MarqueeBox from '../MarqueeBox.vue'
import ModalDialog from '../ModalDialog.vue'
import ModListDialog from '../ModListDialog.vue'
import ScanDetailsDialog from '../ScanDetailsDialog.vue'
import ToggleSwitch from '../ToggleSwitch.vue'
import ViewToggle from '../ViewToggle.vue'

const props = defineProps<{ instanceId: string; kind: LocalResourceKind; sharedNote?: string }>()
const showContent = ref(false)
const showList = ref(false)
const changingVersion = ref<LocalResource | null>(null)
const tasks = useTasksStore()
const ratscan = useRatscanStore()
const packs = usePacksStore()

const resources = ref<LocalResource[]>([])
const loading = ref(true)
const filter = ref('')
const updates = ref<Map<string, AvailableUpdate>>(new Map())
const checking = ref(false)
const toDelete = ref<LocalResource[] | null>(null)
/** Progress of "update all"; `null` when it is not running. */
const bulk = ref<{ done: number; total: number } | null>(null)

const displayName = (r: LocalResource) => plainText(r.metadata?.name ?? '') || plainText(r.fileName)
const projectOf = (r: LocalResource) => (r.origin ? originProject(r.origin) : undefined)

type SortKey = 'name' | 'modified'
const sortKey = ref<SortKey>('name')
const sortAsc = ref(true)
function sortBy(key: SortKey) {
  if (sortKey.value === key) sortAsc.value = !sortAsc.value
  else {
    sortKey.value = key
    sortAsc.value = key !== 'modified'
  }
}
const sortValue: Record<SortKey, (r: LocalResource) => string> = {
  name: displayName,
  modified: (r) => r.modified ?? '',
}

const visible = computed(() => {
  const needle = filter.value.trim().toLowerCase()
  const value = sortValue[sortKey.value]
  return resources.value
    .filter((r) => !needle || r.fileName.toLowerCase().includes(needle) || displayName(r).toLowerCase().includes(needle))
    .sort((a, b) => value(a).localeCompare(value(b), undefined, { numeric: true, sensitivity: 'base' }) * (sortAsc.value ? 1 : -1))
})

const selected = ref(new Set<string>())
let anchor: string | null = null
const { box, onPointerDown } = useMarquee(selected)
const selectedItems = computed(() => resources.value.filter((r) => selected.value.has(r.fileName)))
const single = computed(() => (selectedItems.value.length === 1 ? selectedItems.value[0]! : null))

function select(r: LocalResource, e: MouseEvent) {
  const next = new Set(e.ctrlKey || e.metaKey || e.shiftKey ? selected.value : [])
  if (e.shiftKey && anchor) {
    const names = visible.value.map((x) => x.fileName)
    const [from, to] = [names.indexOf(anchor), names.indexOf(r.fileName)].sort((a, b) => a - b)
    if (from! >= 0) names.slice(from, to! + 1).forEach((name) => next.add(name))
  } else if ((e.ctrlKey || e.metaKey) && next.has(r.fileName)) {
    next.delete(r.fileName)
  } else {
    next.add(r.fileName)
  }
  if (!e.shiftKey) anchor = r.fileName
  selected.value = next
}
function onKey(e: KeyboardEvent) {
  if (e.key === 'a' && (e.ctrlKey || e.metaKey)) {
    e.preventDefault()
    selected.value = new Set(visible.value.map((r) => r.fileName))
  } else if (e.key === 'Delete' && selectedItems.value.length) {
    toDelete.value = selectedItems.value
  } else if (e.key === 'Escape') {
    selected.value = new Set()
  }
}

async function load() {
  loading.value = true
  resources.value = (await tasks.guard(() => api.listLocalResources(props.instanceId, props.kind))) ?? []
  // Keep the selection of files that still exist (a toggle renames `x` to `x.disabled`, so match the base name too).
  const base = (name: string) => name.replace(/\.disabled$/, '')
  const kept = new Set([...selected.value].map(base))
  selected.value = new Set(resources.value.filter((r) => kept.has(base(r.fileName))).map((r) => r.fileName))
  const present = new Set(resources.value.map((r) => r.fileName))
  updates.value = new Map([...updates.value].filter(([name]) => present.has(name)))
  loading.value = false
  // Badges are an extra: a failure to read them must not hide the list.
  const extras = props.kind === 'mod' ? Promise.all([ratscan.refresh(props.instanceId), modCompat.refresh(props.instanceId)]) : props.kind === 'resourcePack' ? packs.refresh(props.instanceId) : undefined
  extras?.catch((e: unknown) => console.warn('cannot read the plugin results', e))
}
async function openTab() {
  updates.value = new Map()
  await load()
  const cached = updateCache.get(cacheKey())
  if (cached && Date.now() - cached.at < UPDATE_CHECK_TTL_MS) updates.value = new Map(cached.updates.map((u) => [u.fileName, u]))
  else if (resources.value.length) await checkUpdates(true)
}
onMounted(openTab)
watch(() => [props.instanceId, props.kind], openTab)
watch(() => packs.changed, load)

// ---- RatScan (mods) ----
const modCompat = useModCompatStore()
const instances = useInstancesStore()
const view = useViewMode('resources')
/** The mod check result for the status line; `null` until the check has finished. */
const compatSummary = computed(() => {
  if (props.kind !== 'mod') return null
  const found = modCompat.byFile[props.instanceId]
  const instance = instances.instances.find((i) => i.id === props.instanceId)
  if (!found || !instance) return null
  const mismatched = Object.keys(found).length
  return mismatched ? { ok: false, text: t.modCompat.someMismatch(mismatched) } : { ok: true, text: t.modCompat.allFit(minecraftVersionOf(instance)) }
})
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
const packBusy = ref(false)
const packResult = ref<(PackResult & { title: string }) | null>(null)
const isZipPack = (r: LocalResource) => props.kind === 'resourcePack' && (r.isDir || /\.zip(\.disabled)?$/i.test(r.fileName))
const mismatchOf = (r: LocalResource) => packs.compat[props.instanceId]?.[r.fileName]
async function runPackTool(action: () => Promise<PackResult & { title: string }>) {
  packBusy.value = true
  const result = await tasks.guard(action)
  packBusy.value = false
  if (result) packResult.value = result
  await load()
}
const convertPack = (r: LocalResource) =>
  runPackTool(async () => {
    const target = await packs.targetFor(props.instanceId)
    if (!target) throw new Error(t.packs.noTarget)
    return { ...(await packs.convert(props.instanceId, r.fileName, target)), title: t.packs.convertTo(target) }
  })
const repairPack = (r: LocalResource) => runPackTool(async () => ({ ...(await packs.repair(props.instanceId, r.fileName)), title: t.packs.repair }))
const targetVersion = ref<string | null>(null)
watch(
  () => [props.instanceId, props.kind],
  async () => {
    targetVersion.value = props.kind === 'resourcePack' ? await packs.targetFor(props.instanceId).catch(() => null) : null
  },
  { immediate: true },
)

/** The first failure stops the rest. */
async function forEach(items: LocalResource[], action: (r: LocalResource) => Promise<void>) {
  await tasks.guard(async () => {
    for (const r of items) await action(r)
  })
  await load()
}
const setEnabled = (items: LocalResource[], enabled: boolean) =>
  forEach(
    items.filter((r) => r.enabled !== enabled),
    (r) => api.toggleResource(props.instanceId, props.kind, r.fileName, enabled),
  )

async function confirmDelete() {
  const items = toDelete.value
  toDelete.value = null
  if (items) await forEach(items, (r) => api.deleteResource(props.instanceId, props.kind, r.fileName))
}

const cacheKey = () => `${props.instanceId}:${props.kind}`
const forgetCheck = () => updateCache.delete(cacheKey())

/** `silent`: the automatic check, which reports nothing when all is up to date. */
async function checkUpdates(silent = false) {
  if (checking.value) return
  checking.value = true
  const found = await (silent
    ? api.checkUpdates(props.instanceId, props.kind).catch((e: unknown) => {
        console.warn('automatic update check failed', e)
        return undefined
      })
    : tasks.guard(() => api.checkUpdates(props.instanceId, props.kind)))
  checking.value = false
  if (!found) return
  updateCache.set(cacheKey(), { at: Date.now(), updates: found })
  updates.value = new Map(found.map((u) => [u.fileName, u]))
  // files identified by the check now have catalog metadata
  await load()
  if (!silent && !found.length) tasks.toast(t.instance.noUpdates, 'ok')
}

async function applyUpdate(r: LocalResource) {
  const update = updates.value.get(r.fileName)
  if (!update) return
  const report = await tasks.guard(() => api.installVersion(props.instanceId, update.latest, props.kind))
  if (!report) return
  tasks.showBlocked(report.blocked, props.instanceId)
  // core replaces the old file of an already installed project, so nothing is deleted here
  tasks.toast(t.instance.updated, 'ok')
  forgetCheck()
  updates.value = new Map([...updates.value].filter(([name]) => name !== r.fileName))
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
  forgetCheck()
  updates.value = new Map()
  tasks.showBlocked(blocked, props.instanceId)
  if (updated) tasks.toast(t.updates.updatedAll(updated), 'ok')
  await load()
}

/** New files: scan jars in the background, offer to convert packs made for another version. */
function noteAdded(fileNames: string[]) {
  forgetCheck()
  if (props.kind === 'mod') ratscan.scanAdded(props.instanceId, fileNames)
  else if (props.kind === 'resourcePack') void packs.offerConversion(props.instanceId, fileNames)
}

const fileExtensions: Record<LocalResourceKind, string[]> = {
  mod: ['jar', 'litemod', 'zip'],
  resourcePack: ['zip'],
  shaderPack: ['zip'],
  dataPack: ['zip'],
}
async function addFiles() {
  const picked = isTauri
    ? await open({ multiple: true, filters: [{ name: t.instance.fileFilter, extensions: fileExtensions[props.kind] }] })
    : ['C:\\Downloads\\sodium-fabric-0.6.0.jar']
  const paths = Array.isArray(picked) ? picked : picked ? [picked] : []
  if (!paths.length) return
  if (!(await addFilesToInstance(props.instanceId, props.kind, paths))) return
  forgetCheck()
  await load()
}

function openHomepages(items: LocalResource[]) {
  for (const url of items.map((r) => projectOf(r)?.url).filter((url) => url !== undefined)) void tasks.guard(() => openExternal(url))
}
async function versionChanged(fileNames: string[]) {
  noteAdded(fileNames)
  await load()
}

const isLocked = (r: LocalResource) => r.origin?.['x-prismlauncher-lock-update'] === true
async function setLocked(items: LocalResource[], locked: boolean) {
  await tasks.guard(async () => {
    for (const r of items) {
      await api.setUpdateLock(props.instanceId, props.kind, r.fileName, locked)
      if (r.origin) r.origin['x-prismlauncher-lock-update'] = locked
    }
  })
  if (locked) updates.value = new Map([...updates.value].filter(([name]) => !items.some((r) => r.fileName === name)))
}

function closeContent() {
  showContent.value = false
  void load()
}

/** Shared by the side panel and the right-click menu. */
const selectionActions = computed<PanelAction[]>(() => {
  const items = selectedItems.value
  const one = single.value
  const fromCatalog = items.filter((r) => r.origin)
  const allLocked = fromCatalog.length > 0 && fromCatalog.every(isLocked)
  const list: PanelAction[] = [
    { label: t.instance.changeVersion, icon: History, disabled: !one?.origin, title: one && !one.origin ? t.instance.localSourceHint : undefined, action: () => (changingVersion.value = one) },
    { label: t.instance.enable, icon: CircleCheck, disabled: !items.some((r) => !r.enabled), action: () => void setEnabled(items, true) },
    { label: t.instance.disable, icon: Ban, disabled: !items.some((r) => r.enabled), action: () => void setEnabled(items, false) },
    { label: t.common.delete, icon: Trash2, danger: true, disabled: !items.length, action: () => (toDelete.value = items) },
    {
      label: t.instance.openHomepage,
      icon: ExternalLink,
      separatorBefore: true,
      disabled: !fromCatalog.length || fromCatalog.length > 10,
      action: () => openHomepages(fromCatalog),
    },
    {
      label: allLocked ? t.updates.unlock : t.updates.lock,
      icon: allLocked ? LockOpen : Lock,
      disabled: !fromCatalog.length,
      action: () => void setLocked(fromCatalog, !allLocked),
    },
  ]
  if (props.kind === 'resourcePack') {
    const zip = one && isZipPack(one) ? one : null
    list.push(
      {
        label: targetVersion.value ? t.packs.convertTo(targetVersion.value) : t.packs.convert,
        icon: Repeat,
        disabled: packBusy.value || !zip || !mismatchOf(zip)?.mismatch || !targetVersion.value,
        title: t.packs.mismatchHint,
        action: () => zip && void convertPack(zip),
      },
      { label: t.packs.repair, icon: Wrench, disabled: packBusy.value || !zip, title: t.packs.repairHint, action: () => zip && void repairPack(zip) },
    )
  }
  return list
})

const panelActions = computed<PanelAction[]>(() => {
  const list: PanelAction[] = [
    { label: t.instance.download[props.kind], icon: Download, primary: true, action: () => (showContent.value = true) },
    { label: checking.value ? t.instance.checking : t.instance.checkUpdates, icon: RefreshCw, busy: checking.value, disabled: checking.value || !!bulk.value || !resources.value.length, action: () => void checkUpdates() },
  ]
  if (updates.value.size) {
    list.push({
      label: bulk.value ? t.updates.updatingAll(bulk.value.done, bulk.value.total) : `${t.updates.updateAll} (${updates.value.size})`,
      icon: ArrowUpFromLine,
      disabled: !!bulk.value,
      action: () => void updateAll(),
    })
  }
  list.push({ label: t.instance.addFile, icon: FilePlus, title: t.instance.addFileHint, action: () => void addFiles() })
  const selection = selectionActions.value
  selection[0] = { ...selection[0]!, separatorBefore: true }
  list.push(...selection)
  list.push({ label: t.instance.exportList, icon: ListOrdered, separatorBefore: true, disabled: !resources.value.length, action: () => (showList.value = true) })
  if (props.kind === 'mod') {
    list.push({
      label: scanning.value ? t.scan.scanning(ratscan.progress!.done, ratscan.progress!.total) : t.scan.scanMods,
      icon: ShieldCheck,
      disabled: scanning.value || !resources.value.length,
      action: () => void scanMods(),
    })
  }
  return list
})
const footerActions = computed<PanelAction[]>(() => [
  { label: t.instance.openFolder, icon: FolderOpen, iconClass: 'text-warn', title: t.instance.folderOf[props.kind], action: () => void tasks.guard(() => api.openContentFolder(props.instanceId, props.kind)) },
  ...(props.kind === 'mod' ? [{ label: t.instance.openConfigs, icon: FolderCog, iconClass: 'text-warn', action: () => void tasks.guard(() => api.openConfigFolder(props.instanceId)) }] : []),
])

const menu = ref<{ x: number; y: number } | null>(null)
function openMenu(r: LocalResource, e: MouseEvent) {
  if (!selected.value.has(r.fileName)) {
    selected.value = new Set([r.fileName])
    anchor = r.fileName
  }
  menu.value = { x: e.clientX, y: e.clientY }
}

const columns: { key: SortKey; label: () => string; class: string }[] = [
  { key: 'name', label: () => t.instance.columns.name, class: '' },
  { key: 'modified', label: () => t.instance.columns.modified, class: 'w-28' },
]
</script>

<template>
  <div class="flex flex-col gap-3">
    <p v-if="sharedNote" class="flex items-center gap-2 text-muted">
      <span class="badge badge-accent">{{ t.sharedFolder.badge }}</span>{{ sharedNote }}
    </p>
    <div class="flex items-start gap-3">
      <div class="flex min-w-0 flex-1 flex-col gap-2">
        <div class="flex items-center gap-2">
          <div class="relative max-w-sm flex-1">
            <Search :size="15" class="pointer-events-none absolute top-2.5 left-2.5 text-faint" />
            <input v-model="filter" class="input !pl-8" :placeholder="t.instance.filterPlaceholder" :aria-label="t.instance.filterPlaceholder" />
          </div>
          <span class="text-faint">
            {{ t.instance.counted(resources.length) }}<template v-if="selected.size"> · {{ t.instance.selectedCount(selected.size) }}</template>
          </span>
          <ViewToggle v-model="view" class="ml-auto" />
        </div>

        <p v-if="loading" class="py-10 text-center text-faint">{{ t.common.loading }}</p>
        <p v-else-if="!visible.length" class="py-10 text-center text-faint">{{ t.instance.noResources }}</p>
        <div
          v-else
          class="!outline-none"
          :class="view === 'list' ? 'overflow-hidden rounded-xl border border-line' : ''"
          tabindex="0"
          role="listbox"
          aria-multiselectable="true"
          @pointerdown="onPointerDown"
          @dragstart.prevent
          @keydown="onKey"
        >
          <div v-if="view === 'list'" class="flex items-center gap-3 border-b border-line px-3 py-2 text-[12px] font-semibold text-faint">
            <span class="w-9 shrink-0" />
            <span class="w-9 shrink-0" />
            <button
              v-for="c in columns"
              :key="c.key"
              class="flex shrink-0 items-center gap-1 text-left hover:text-fg"
              :class="c.class || 'min-w-0 flex-1'"
              @click="sortBy(c.key)"
            >
              {{ c.label() }}
              <component :is="sortAsc ? ArrowUp : ArrowDown" v-if="sortKey === c.key" :size="12" />
            </button>
          </div>
          <ul :class="view === 'tiles' ? 'grid grid-cols-[repeat(auto-fill,minmax(250px,1fr))] gap-2.5' : ''">
            <li
              v-for="r in visible"
              :key="r.fileName"
              :data-select-key="r.fileName"
              role="option"
              :aria-selected="selected.has(r.fileName)"
              class="relative flex cursor-default"
              :class="[
                view === 'list' ? 'items-center gap-3 border-b border-line/60 px-3 py-1.5 last:border-b-0' : 'flex-col gap-2 rounded-xl border border-line p-3',
                selected.has(r.fileName) ? (view === 'list' ? 'bg-accent/12' : 'border-accent bg-accent/12') : 'hover:bg-hover',
              ]"
              @click="select(r, $event)"
              @contextmenu.prevent="openMenu(r, $event)"
            >
              <span class="flex shrink-0" :class="view === 'list' ? 'w-9' : 'absolute top-3 right-3'" @click.stop>
                <ToggleSwitch :model-value="r.enabled" :label="t.common.enabled" @update:model-value="setEnabled([r], $event)" />
              </span>
              <img v-if="r.metadata?.icon" :src="r.metadata.icon" alt="" class="pixelated size-9 shrink-0 rounded-md" :class="r.enabled ? '' : 'opacity-50'" />
              <span v-else class="pix flex size-9 shrink-0 items-center justify-center bg-accent/15 font-bold text-accent" :class="r.enabled ? '' : 'opacity-50'">
                {{ displayName(r).charAt(0).toUpperCase() }}
              </span>
              <div class="min-w-0 flex-1" :class="r.enabled ? '' : 'opacity-60'">
                <div class="flex items-center gap-1.5" :class="view === 'tiles' ? 'flex-wrap' : ''">
                  <span class="truncate font-semibold" :class="view === 'tiles' ? 'max-w-full' : ''">{{ displayName(r) }}</span>
                  <span v-if="r.metadata?.version" class="badge shrink-0">{{ r.metadata.version }}</span>
                  <span v-if="projectOf(r)" class="badge badge-accent shrink-0">{{ PLATFORM_LABELS[projectOf(r)!.platform] }}</span>
                  <span v-else class="badge shrink-0" :title="t.instance.localSourceHint">{{ t.instance.localSource }}</span>
                  <Lock v-if="isLocked(r)" :size="12" class="shrink-0 text-warn" :title="t.updates.lock" />
                  <button
                    v-if="kind === 'mod' && jarsByName.get(r.fileName)?.scan"
                    class="badge cursor-pointer"
                    :class="badgeClass[badgeOf(jarsByName.get(r.fileName)!.scan!.verdict)]"
                    :title="t.scan.details"
                    @click.stop="scanDetails = jarsByName.get(r.fileName)!"
                  >{{ badgeText[badgeOf(jarsByName.get(r.fileName)!.scan!.verdict)] }}</button>
                  <span v-if="kind === 'mod' && modCompat.byFile[instanceId]?.[r.fileName]" class="badge badge-warn" :title="modCompat.hint(modCompat.byFile[instanceId][r.fileName])">
                    {{ modCompat.label(modCompat.byFile[instanceId][r.fileName]) }}
                  </span>
                  <span v-if="kind === 'resourcePack' && mismatchOf(r)?.mismatch" class="badge badge-warn" :title="t.packs.mismatchHint">
                    {{ t.packs.madeFor(mismatchOf(r)!.madeFor ?? '?') }}
                  </span>
                  <button v-if="updates.has(r.fileName)" class="badge badge-accent cursor-pointer" :disabled="!!bulk" :title="t.instance.updateTo" @click.stop="applyUpdate(r)">
                    <ArrowUpCircle :size="12" />{{ updates.get(r.fileName)!.latest.versionNumber }}
                  </button>
                </div>
                <div class="truncate text-[12px] text-faint">
                  {{ plainText(r.fileName) }} · {{ r.isDir ? t.instance.folderItem : formatBytes(r.size) }}<template v-if="r.metadata?.authors.length"> · {{ r.metadata.authors.join(', ') }}</template>
                </div>
              </div>
              <span v-if="view === 'list'" class="w-28 shrink-0 truncate text-[12.5px] text-faint">{{ formatRelative(r.modified) }}</span>
            </li>
          </ul>
        </div>
        <p class="text-[12px] text-faint">{{ t.instance.selectionHint }}</p>
        <p v-if="compatSummary" class="flex items-center gap-1.5 text-[12px]" :class="compatSummary.ok ? 'text-ok' : 'text-warn'">
          <component :is="compatSummary.ok ? CircleCheck : TriangleAlert" :size="13" />{{ compatSummary.text }}
        </p>
      </div>

      <MarqueeBox :box="box" />
      <ActionPanel :actions="panelActions" :footer="footerActions" />
    </div>

    <ContextMenu
      v-if="menu"
      :x="menu.x"
      :y="menu.y"
      :items="selectionActions.map((a) => ({ label: a.label, icon: a.icon, disabled: a.disabled, danger: a.danger, separatorBefore: a.separatorBefore, action: a.action }))"
      @close="menu = null"
    />
    <ConfirmDialog
      v-if="toDelete"
      :title="toDelete.length === 1 ? t.instance.deleteResource : t.instance.deleteResources(toDelete.length)"
      :text="toDelete.map((r) => r.fileName).slice(0, 8).join(', ') + (toDelete.length > 8 ? '…' : '')"
      @confirm="confirmDelete"
      @close="toDelete = null"
    />
    <ScanDetailsDialog v-if="scanDetails" :instance-id="instanceId" :jar="scanDetails" @close="scanDetails = null" />
    <ModalDialog v-if="packResult" :title="packResult.title" size="sm" @close="packResult = null">
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
    <ModListDialog v-if="showList" :resources="resources" :title="t.instance.exportList" :file-name="`${instanceId}-${kind}`" @close="showList = false" />
    <ChangeVersionDialog
      v-if="changingVersion"
      :instance-id="instanceId"
      :kind="kind"
      :resource="changingVersion"
      @changed="versionChanged"
      @close="changingVersion = null"
    />
  </div>
</template>

