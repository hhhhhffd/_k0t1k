import { Copy, FileArchive, Flame, FolderOpen, Package, Pencil, Play, Puzzle, RefreshCw, Square, Tag, TextCursorInput, Trash2, Upload } from '@lucide/vue'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { openInstanceFolder } from '../api/commands'
import type { ExportFormat, Instance } from '../api/types'
import type { MenuItem } from '../components/ContextMenu.vue'
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import { useLayoutStore } from '../stores/layout'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'

/**
 * The actions on an instance offered by the library's panel and context menus; `InstanceActionDialogs` shows
 * the dialogs they open. `targetsOf` gives what a group change or delete started on an instance applies to.
 */
export function useInstanceActions(targetsOf: (inst: Instance) => Instance[] = (inst) => [inst]) {
  const store = useInstancesStore()
  const settings = useSettingsStore()
  const tasks = useTasksStore()
  const layout = useLayoutStore()
  const router = useRouter()

  /** `targets`: all instances a group change applies to (several when picked together). */
  const dialog = ref<{ kind: 'rename' | 'copy' | 'group'; instance: Instance; targets?: Instance[] } | null>(null)
  const deleting = ref<Instance[]>([])
  const exportMenu = ref<{ x: number; y: number; instance: Instance } | null>(null)
  const exporting = ref<{ format: ExportFormat; instance: Instance } | null>(null)
  const packUpdate = ref<Instance | null>(null)

  const launch = (inst: Instance) => void tasks.guard(() => store.launch(inst.id, settings.settings?.selectedAccountId ?? undefined))

  // The export formats open at the pointer, wherever "Export" was picked (panel or context menu).
  let lastPointer = { x: 0, y: 0 }
  const rememberPointer = (e: PointerEvent) => (lastPointer = { x: e.clientX, y: e.clientY })

  /** One action list for the side panel and the context menu, in Prism's order. */
  function actionsFor(inst: Instance): MenuItem[] {
    const running = store.stateOf(inst.id) !== 'stopped'
    return [
      { label: running ? t.library.playAgain : t.panel.launch, icon: Play, iconKey: 'launch', action: () => launch(inst) },
      { label: t.library.stop, icon: Square, iconKey: 'stop', disabled: !running, action: () => void tasks.guard(() => store.kill(inst.id)) },
      { label: t.panel.edit, icon: Pencil, iconKey: 'edit', separatorBefore: true, action: () => router.push({ name: 'instance', params: { id: inst.id } }) },
      { label: t.panel.versions, icon: Puzzle, iconKey: 'versions', action: () => router.push({ name: 'instance', params: { id: inst.id }, query: { tab: 'version' } }) },
      { label: t.library.changeGroup, icon: Tag, iconKey: 'group', action: () => (dialog.value = { kind: 'group', instance: inst, targets: targetsOf(inst) }) },
      { label: t.library.rename, icon: TextCursorInput, iconKey: 'rename', action: () => (dialog.value = { kind: 'rename', instance: inst }) },
      { label: t.panel.folder, icon: FolderOpen, iconKey: 'folder', action: () => void tasks.guard(() => openInstanceFolder(inst.id)) },
      { label: t.transfer.export, icon: Upload, iconKey: 'export', action: () => (exportMenu.value = { ...lastPointer, instance: inst }) },
      ...(inst.managedPack ? [{ label: t.pack.check, icon: RefreshCw, iconKey: 'update', action: () => (packUpdate.value = inst) }] : []),
      { label: t.library.copy, icon: Copy, iconKey: 'copy', action: () => (dialog.value = { kind: 'copy', instance: inst }) },
      { label: t.library.delete, icon: Trash2, iconKey: 'delete', danger: true, separatorBefore: true, action: () => (deleting.value = targetsOf(inst)) },
    ]
  }

  const exportItems = computed<MenuItem[]>(() => {
    const inst = exportMenu.value?.instance
    if (!inst) return []
    const pick = (format: ExportFormat) => () => (exporting.value = { format, instance: inst })
    return [
      { label: t.transfer.exportInstance, icon: FileArchive, iconKey: 'export-instance', action: pick('instance') },
      { label: t.transfer.exportModrinth, icon: Package, iconKey: 'export-modrinth', action: pick('modrinth') },
      { label: t.transfer.exportCurseForge, icon: Flame, iconKey: 'export-curseforge', action: pick('curseForge') },
    ]
  })

  async function submitPrompt(value: string) {
    const d = dialog.value
    if (!d) return
    dialog.value = null
    if (d.kind === 'rename') await tasks.guard(() => store.rename(d.instance.id, value))
    else if (d.kind === 'group') {
      await tasks.guard(async () => {
        for (const target of d.targets ?? [d.instance]) await store.setGroup(target.id, value || null)
      })
    }
    else {
      const copy = await tasks.guard(() => store.copy(d.instance.id, value))
      if (copy) layout.selectedId = copy.id
    }
  }

  async function confirmDelete() {
    const doomed = deleting.value
    deleting.value = []
    // The store drops each instance as it is deleted, so a failure midway leaves the list correct.
    await tasks.guard(async () => {
      for (const inst of doomed) await store.remove(inst.id)
    })
  }

  return { dialog, deleting, exportMenu, exporting, packUpdate, launch, rememberPointer, actionsFor, exportItems, submitPrompt, confirmDelete }
}

export type InstanceActions = ReturnType<typeof useInstanceActions>
