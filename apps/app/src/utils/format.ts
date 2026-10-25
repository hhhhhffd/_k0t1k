import { CLIENT_UIDS, COMPONENT_MINECRAFT, LOADER_UIDS } from '../api/types'
import { language, t } from '../i18n'
import type { ClientId, Instance, LoaderId, Platform, PwMod } from '../api/types'

export const LOADER_LABELS: Record<LoaderId, string> = {
  fabric: 'Fabric',
  quilt: 'Quilt',
  forge: 'Forge',
  neoForge: 'NeoForge',
}

export const CLIENT_LABELS: Record<ClientId, string> = { labymod4: 'LabyMod 4' }

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

export function clientOf(inst: Instance): { id: ClientId; label: string; version: string } | undefined {
  for (const id of Object.keys(CLIENT_UIDS) as ClientId[]) {
    const comp = inst.components.find((c) => c.uid === CLIENT_UIDS[id])
    if (comp) return { id, label: CLIENT_LABELS[id], version: comp.version }
  }
  return undefined
}

/** Platform a local file was installed from, according to its packwiz entry. */
export function originPlatform(origin: PwMod): Platform | undefined {
  if (origin.update.modrinth) return 'modrinth'
  if (origin.update.curseforge) return 'curseForge'
  return undefined
}

const dateLocale = () => (language.value === 'ru' ? 'ru-RU' : 'en-US')

export function formatPlaytime(secs: number): string {
  const { min, hour, lessThanMinute } = t.format
  if (secs < 60) return secs > 0 ? lessThanMinute : '—'
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  return h > 0 ? `${h} ${hour} ${m} ${min}` : `${m} ${min}`
}

export function formatRelative(iso?: string | null): string {
  if (!iso) return t.format.never
  const { min: minLabel, hour, day, justNow, ago } = t.format
  const diff = Date.now() - new Date(iso).getTime()
  const min = Math.floor(diff / 60_000)
  if (min < 1) return justNow
  if (min < 60) return ago(`${min} ${minLabel}`)
  const h = Math.floor(min / 60)
  if (h < 24) return ago(`${h} ${hour}`)
  const d = Math.floor(h / 24)
  if (d < 30) return ago(`${d} ${day}`)
  return new Date(iso).toLocaleDateString(dateLocale())
}

export function formatDate(iso?: string | null): string {
  return iso ? new Date(iso).toLocaleDateString(dateLocale()) : '—'
}

export function formatBytes(bytes: number): string {
  const { byte, kilobyte, megabyte, gigabyte } = t.format
  if (bytes < 1024) return `${bytes} ${byte}`
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(0)} ${kilobyte}`
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(1)} ${megabyte}`
  return `${(bytes / 1024 ** 3).toFixed(2)} ${gigabyte}`
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
