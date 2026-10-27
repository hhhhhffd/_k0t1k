import { addLocalFiles } from '../api/commands'
import type { AddFilesReport, LocalResourceKind } from '../api/types'
import { t } from '../i18n'
import { useModCompatStore } from '../stores/modCompat'
import { usePacksStore } from '../stores/packs'
import { useRatscanStore } from '../stores/ratscan'
import { useTasksStore } from '../stores/tasks'
import { baseName } from './useFileDrop'

/**
 * Copies files into an instance (`kind` null: identified per file), reports the result in toasts and
 * starts the follow-ups: a scan of new mods, the offer to convert new resource packs.
 */
export async function addFilesToInstance(instanceId: string, kind: LocalResourceKind | null, paths: string[]): Promise<AddFilesReport | undefined> {
  const tasks = useTasksStore()
  if (kind === 'mod' || kind === null) {
    const checked = await useModCompatStore().confirmAdd(instanceId, paths)
    if (!checked?.length) return undefined
    paths = checked
  }
  const report = await tasks.guard(() => addLocalFiles(instanceId, kind, paths))
  if (!report) return undefined
  if (report.added.length) tasks.toast(t.transfer.addedFiles(report.added.length), 'ok')
  for (const name of report.fixedNested) tasks.toast(t.packs.fixedOnAdd(name), 'ok')
  for (const skipped of report.skipped) {
    tasks.toast(t.transfer.skippedFile(baseName(skipped.path), t.transfer.skipped[skipped.reason]), 'error')
  }
  useRatscanStore().scanAdded(instanceId, report.added.filter((a) => a.kind === 'mod').map((a) => a.fileName))
  void usePacksStore().offerConversion(instanceId, report.added.filter((a) => a.kind === 'resourcePack').map((a) => a.fileName))
  return report
}
