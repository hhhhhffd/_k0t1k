import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api/commands'
import type { ConvertReport, PackCompat, RepairReport } from '../api/types'
import { t } from '../i18n'
import { callPlugin } from '../plugins/host'
import { minecraftVersionOf } from '../utils/format'
import { useInstancesStore } from './instances'
import { useTasksStore } from './tasks'

const version = (text: string) => {
  const parts = text.split('.').map(Number)
  return parts.length >= 2 && parts.length <= 3 && parts.every(Number.isInteger) ? ([parts[0], parts[1], parts[2] ?? 0] as const) : null
}
const compare = (a: readonly number[], b: readonly number[]) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]

/** The newest version the converter supports that is not newer than `instanceVersion`; null when there is none (a snapshot, say). */
export function conversionTarget(instanceVersion: string, supported: string[]): string | null {
  const wanted = version(instanceVersion)
  if (!wanted) return null
  let best: string | null = null
  let bestVersion: readonly number[] | null = null
  for (const name of supported) {
    const candidate = version(name)
    if (!candidate || compare(candidate, wanted) > 0) continue
    if (!bestVersion || compare(candidate, bestVersion) > 0) {
      best = name
      bestVersion = candidate
    }
  }
  return best
}

/** `Faithful.zip` and `Faithful.zip.disabled` both give `Faithful`. */
const stemOf = (fileName: string) => fileName.replace(/\.disabled$/i, '').replace(/\.zip$/i, '')

export interface PackResult {
  savedAs: string
  summary: string
  warnings: string[]
}

export const usePacksStore = defineStore('packs', () => {
  const tasks = useTasksStore()
  const instances = useInstancesStore()
  const compat = ref<Record<string, Record<string, PackCompat>>>({})
  /** Bumped when a conversion or repair added a file, so open pack lists reload. */
  const changed = ref(0)
  let supported: Promise<string[]> | undefined

  const supportedVersions = () => (supported ??= callPlugin<string[]>('packconv', 'supportedVersions', []).catch((e: unknown) => {
    supported = undefined
    throw e
  }))

  async function refresh(instanceId: string) {
    compat.value[instanceId] = Object.fromEntries((await api.resourcepackCompat(instanceId)).map((pack) => [pack.fileName, pack]))
  }

  /** The version a pack would be converted to for this instance; null when the instance's version is not a release. */
  async function targetFor(instanceId: string): Promise<string | null> {
    const instance = instances.instances.find((i) => i.id === instanceId)
    return instance ? conversionTarget(minecraftVersionOf(instance), await supportedVersions()) : null
  }

  async function transform(instanceId: string, fileName: string, outName: string, run: (bytes: Uint8Array) => Promise<{ zip: Uint8Array; report: unknown }>) {
    const bytes = new Uint8Array(await api.readResourceFile(instanceId, 'resourcePack', fileName))
    const result = await run(bytes)
    const savedAs = await api.saveResourceFile(instanceId, 'resourcePack', outName, result.zip)
    return { savedAs, report: result.report }
  }

  async function convert(instanceId: string, fileName: string, target: string): Promise<PackResult> {
    const { savedAs, report } = await transform(instanceId, fileName, `${stemOf(fileName)} (${target}).zip`, (bytes) =>
      callPlugin('packconv', 'convert', [bytes, target], [bytes.buffer]),
    )
    const r = report as ConvertReport
    return { savedAs, summary: t.packs.convertSummary(r.from, r.to, r.steps.length, r.files_out), warnings: r.warnings }
  }

  async function repair(instanceId: string, fileName: string): Promise<PackResult> {
    const { savedAs, report } = await transform(instanceId, fileName, `${stemOf(fileName)} (repaired).zip`, (bytes) =>
      callPlugin('packconv', 'repair', [bytes], [bytes.buffer]),
    )
    const r = report as RepairReport
    return { savedAs, summary: r.repaired.length ? t.packs.repairSummary(r.repaired.length) : t.packs.nothingToRepair, warnings: r.warnings }
  }

  /** After packs were added: offers to convert those made for another version, without blocking anything. */
  async function offerConversion(instanceId: string, fileNames: string[]) {
    const names = fileNames.filter((name) => /\.zip(\.disabled)?$/i.test(name))
    if (!names.length) return
    try {
      await refresh(instanceId)
      const target = await targetFor(instanceId)
      if (!target) return
      for (const name of names) {
        const pack = compat.value[instanceId]?.[name]
        if (!pack?.mismatch) continue
        tasks.toast(t.packs.offer(name, pack.madeFor ?? '?', target), 'info', {
          label: t.packs.convertTo(target),
          run: () => {
            void tasks.guard(async () => {
              const result = await convert(instanceId, name, target)
              changed.value++
              tasks.toast(`${t.packs.saved(result.savedAs)} ${result.summary}`, 'ok')
            })
          },
        })
      }
    } catch (e) {
      console.warn('cannot check the added resource packs', e)
    }
  }

  return { compat, changed, refresh, targetFor, convert, repair, offerConversion }
})
