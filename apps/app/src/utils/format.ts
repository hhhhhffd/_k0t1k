import { COMPONENT_MINECRAFT, LOADER_UIDS } from '../api/types'
import type { Instance, LoaderId } from '../api/types'

export const LOADER_LABELS: Record<LoaderId, string> = {
  fabric: 'Fabric',
  quilt: 'Quilt',
  forge: 'Forge',
  neoforge: 'NeoForge',
}

export function minecraftVersionOf(inst: Instance): string {
  return inst.components.find((c) => c.uid === COMPONENT_MINECRAFT)?.version ?? '?'
}

export function loaderOf(inst: Instance): { id: LoaderId; label: string; version: string } | undefined {
  for (const id of Object.keys(LOADER_UIDS) as LoaderId[]) {
    const comp = inst.components.find((c) => c.uid === LOADER_UIDS[id])
    if (comp) return { id, label: LOADER_LABELS[id], version: comp.version }
  }
  return undefined
}

export function formatPlaytime(secs: number): string {
  if (secs < 60) return secs > 0 ? '<1 мин' : '—'
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  return h > 0 ? `${h} ч ${m} мин` : `${m} мин`
}

export function formatRelative(iso?: string): string {
  if (!iso) return 'никогда'
  const diff = Date.now() - new Date(iso).getTime()
  const min = Math.floor(diff / 60_000)
  if (min < 1) return 'только что'
  if (min < 60) return `${min} мин назад`
  const h = Math.floor(min / 60)
  if (h < 24) return `${h} ч назад`
  const d = Math.floor(h / 24)
  if (d < 30) return `${d} дн назад`
  return new Date(iso).toLocaleDateString('ru-RU')
}

export function formatDate(iso?: string): string {
  return iso ? new Date(iso).toLocaleDateString('ru-RU') : '—'
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} Б`
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(0)} КБ`
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} МБ`
  return `${(bytes / 1024 ** 3).toFixed(2)} ГБ`
}

export function formatCount(n: number): string {
  if (n >= 1e6) return `${(n / 1e6).toFixed(n >= 1e7 ? 0 : 1)}M`
  if (n >= 1e3) return `${(n / 1e3).toFixed(0)}K`
  return String(n)
}

/** Deep copy through JSON; structuredClone throws on Vue reactive proxies. */
export const clone = <T,>(v: T): T => JSON.parse(JSON.stringify(v)) as T

export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e)
}
