// Mock of the plugin commands. The plugins themselves are the real bundled wasm, run in the same worker
// as in the app; only the file access is faked: jars and packs live in memory.
import { callPlugin } from '../plugins/host'
import type { JarScan, LocalResource, LocalResourceKind, PackCompat, PluginId, PluginInfo, ScanResult } from './types'

type Args = Record<string, unknown>
export interface PluginMockContext {
  resourcesFor(instanceId: string, kind: LocalResourceKind): LocalResource[]
  minecraftVersion(instanceId: string): string
}

const PLUGIN_IDS: PluginId[] = ['ratscan', 'packconv']
const encoder = new TextEncoder()
const lastChecks = new Map<PluginId, string>()
const files = new Map<string, Uint8Array>()
const caches = new Map<string, Map<string, { pluginVersion: string; scannedAt: string; result: ScanResult }>>()
const fileKey = (instanceId: string, kind: string, fileName: string) => `${instanceId}:${kind}:${fileName}`

// ---- a minimal stored (uncompressed) zip writer, enough for a tiny jar or resource pack ----
const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
  return c >>> 0
})
function crc32(bytes: Uint8Array) {
  let c = 0xffffffff
  for (const b of bytes) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8)
  return (c ^ 0xffffffff) >>> 0
}
export function makeZip(entries: Record<string, string | Uint8Array>): Uint8Array {
  const chunks: Uint8Array[] = []
  const central: Uint8Array[] = []
  let offset = 0
  for (const [name, content] of Object.entries(entries)) {
    const data = typeof content === 'string' ? encoder.encode(content) : content
    const nameBytes = encoder.encode(name)
    const header = new DataView(new ArrayBuffer(30))
    header.setUint32(0, 0x04034b50, true)
    header.setUint16(4, 20, true)
    header.setUint32(14, crc32(data), true)
    header.setUint32(18, data.length, true)
    header.setUint32(22, data.length, true)
    header.setUint16(26, nameBytes.length, true)
    const entry = new DataView(new ArrayBuffer(46))
    entry.setUint32(0, 0x02014b50, true)
    entry.setUint16(4, 20, true)
    entry.setUint16(6, 20, true)
    entry.setUint32(16, crc32(data), true)
    entry.setUint32(20, data.length, true)
    entry.setUint32(24, data.length, true)
    entry.setUint16(28, nameBytes.length, true)
    entry.setUint32(42, offset, true)
    chunks.push(new Uint8Array(header.buffer), nameBytes, data)
    central.push(new Uint8Array(entry.buffer), nameBytes)
    offset += 30 + nameBytes.length + data.length
  }
  const size = central.reduce((sum, part) => sum + part.length, 0)
  const end = new DataView(new ArrayBuffer(22))
  end.setUint32(0, 0x06054b50, true)
  end.setUint16(8, Object.keys(entries).length, true)
  end.setUint16(10, Object.keys(entries).length, true)
  end.setUint32(12, size, true)
  end.setUint32(16, offset, true)
  return concat([...chunks, ...central, new Uint8Array(end.buffer)])
}
function concat(parts: Uint8Array[]) {
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0))
  let at = 0
  for (const part of parts) {
    out.set(part, at)
    at += part.length
  }
  return out
}

/** Bytes of a mock file; seeded mods and packs are generated on first use. */
function bytesOf(ctx: PluginMockContext, instanceId: string, kind: LocalResourceKind, fileName: string): Uint8Array {
  const key = fileKey(instanceId, kind, fileName)
  let bytes = files.get(key)
  if (!bytes) {
    const index = ctx.resourcesFor(instanceId, kind).findIndex((r) => r.fileName === fileName)
    if (index < 0) throw new Error(`mock: no such file ${fileName}`)
    bytes =
      kind === 'resourcePack'
        ? makeZip({ 'pack.mcmeta': JSON.stringify({ pack: { pack_format: index % 2 ? 15 : 46, description: fileName } }) })
        : makeZip({ 'META-INF/MANIFEST.MF': 'Manifest-Version: 1.0\n', 'fabric.mod.json': JSON.stringify({ schemaVersion: 1, id: `mock${index}`, version: '1.0.0' }) })
    files.set(key, bytes)
  }
  return bytes
}

async function sha1Hex(bytes: Uint8Array) {
  const digest = await crypto.subtle.digest('SHA-1', bytes as BufferSource)
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
}

async function bundledInfo(id: PluginId): Promise<PluginInfo> {
  const manifest = (await (await fetch(`${import.meta.env.BASE_URL}plugins/${id}/plugin.json`)).json()) as { version: string; files: Record<string, string> }
  return { id, version: manifest.version, source: 'bundled', files: Object.keys(manifest.files), lastCheck: lastChecks.get(id) ?? null, error: null, delayed: false }
}

const copyOf = (bytes: Uint8Array) => bytes.slice().buffer

/** Lets a test put real files into the mock instance (a jar to scan, a pack to convert). */
export function installTestHook(ctx: PluginMockContext) {
  ;(window as unknown as Record<string, unknown>).__k0t1kMock = {
    putFile(instanceId: string, kind: LocalResourceKind, fileName: string, bytes: Uint8Array) {
      files.set(fileKey(instanceId, kind, fileName), bytes)
      const list = ctx.resourcesFor(instanceId, kind)
      if (!list.some((r) => r.fileName === fileName)) {
        list.unshift({ fileName, enabled: true, size: bytes.length, metadata: null, origin: null })
      }
    },
  }
}

export async function pluginCommand(cmd: string, a: Args, ctx: PluginMockContext, body?: Uint8Array, headers?: Record<string, string>): Promise<{ value: unknown } | undefined> {
  switch (cmd) {
    case 'list_plugins':
      return { value: await Promise.all(PLUGIN_IDS.map(bundledInfo)) }
    case 'check_plugin_updates': {
      const now = new Date().toISOString()
      for (const id of PLUGIN_IDS) lastChecks.set(id, now)
      return { value: await Promise.all(PLUGIN_IDS.map(bundledInfo)) }
    }
    case 'read_plugin_file':
      throw new Error('mock: only bundled plugins exist')
    case 'ratscan_status': {
      const instanceId = a.instanceId as string
      const version = (await bundledInfo('ratscan')).version
      const cache = caches.get(instanceId)
      const jars: JarScan[] = []
      for (const r of ctx.resourcesFor(instanceId, 'mod')) {
        if (!/\.jar(\.disabled)?$/i.test(r.fileName)) continue
        const sha1 = await sha1Hex(bytesOf(ctx, instanceId, 'mod', r.fileName))
        const hit = cache?.get(sha1)
        const report = hit?.pluginVersion === version ? hit.result.report : null
        jars.push({
          fileName: r.fileName,
          enabled: r.enabled,
          size: r.size,
          sha1,
          scan: hit && report ? { verdict: report.verdict, riskScore: report.risk_score, summary: hit.result.summary, scannedAt: hit.scannedAt } : null,
        })
      }
      return { value: jars }
    }
    case 'ratscan_store': {
      const instanceId = a.instanceId as string
      if (!caches.has(instanceId)) caches.set(instanceId, new Map())
      caches.get(instanceId)!.set(a.sha1 as string, { pluginVersion: a.pluginVersion as string, scannedAt: new Date().toISOString(), result: a.result as ScanResult })
      return { value: undefined }
    }
    case 'ratscan_report':
      return { value: caches.get(a.instanceId as string)?.get(a.sha1 as string)?.result ?? null }
    case 'read_resource_file':
      return { value: copyOf(bytesOf(ctx, a.instanceId as string, a.kind as LocalResourceKind, a.fileName as string)) }
    case 'save_resource_file': {
      if (!body || !headers?.['x-args']) throw new Error('mock: save_resource_file needs a body and x-args')
      const args = JSON.parse(atob(headers['x-args'])) as { instanceId: string; kind: LocalResourceKind; fileName: string }
      const list = ctx.resourcesFor(args.instanceId, args.kind)
      let name = args.fileName
      for (let n = 2; list.some((r) => r.fileName === name); n++) name = args.fileName.replace(/(\.[^.]+)?$/, ` (${n})$1`)
      files.set(fileKey(args.instanceId, args.kind, name), body)
      list.unshift({ fileName: name, enabled: true, size: body.length, metadata: null, origin: null })
      return { value: name }
    }
    case 'resourcepack_compat': {
      // The real backend reads pack.mcmeta in Rust; the mock asks the converter whether `from` equals `to`.
      const instanceId = a.instanceId as string
      const version = ctx.minecraftVersion(instanceId)
      const packs: PackCompat[] = []
      for (const r of ctx.resourcesFor(instanceId, 'resourcePack')) {
        if (!/\.zip(\.disabled)?$/i.test(r.fileName)) continue
        try {
          const bytes = bytesOf(ctx, instanceId, 'resourcePack', r.fileName).slice()
          const { report } = await callPlugin<{ report: { from: string; to: string } }>('packconv', 'convert', [bytes, version], [bytes.buffer])
          packs.push({ fileName: r.fileName, madeFor: report.from, mismatch: report.from !== report.to })
        } catch {
          packs.push({ fileName: r.fileName, madeFor: null, mismatch: false })
        }
      }
      return { value: packs }
    }
    default:
      return undefined
  }
}
