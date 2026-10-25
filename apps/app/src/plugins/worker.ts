/*
 * The sandbox of a wasm plugin: a dedicated module worker owned by the launcher. A worker has no Tauri IPC
 * and no access to the page, so a plugin only sees the bytes it is called with. The host (host.ts) sends the
 * wasm-bindgen glue and the wasm; the glue is imported from a blob URL and initialised with the wasm bytes.
 */

/** The export contract of plugin api 1. */
export type PluginFunction = 'scan' | 'convert' | 'repair' | 'supportedVersions'

export type WorkerRequest =
  | { type: 'init'; glue: string; wasm: ArrayBuffer }
  | { type: 'call'; id: number; fn: PluginFunction; args: unknown[] }

export type WorkerResponse =
  | { type: 'ready' }
  | { type: 'fatal'; message: string }
  | { type: 'result'; id: number; value: unknown }
  | { type: 'failure'; id: number; message: string }

interface ConvertResult {
  zip: Uint8Array
  report: string
  free(): void
}
interface PluginModule {
  default: (init: { module_or_path: ArrayBuffer }) => Promise<unknown>
  scan(bytes: Uint8Array): string
  convert(bytes: Uint8Array, to: string, from?: string): ConvertResult
  repair(bytes: Uint8Array): ConvertResult
  supportedVersions(): string[]
}

const scope = self as unknown as {
  postMessage(message: WorkerResponse, transfer?: Transferable[]): void
  onmessage: ((event: MessageEvent<WorkerRequest>) => void) | null
}

let plugin: PluginModule | undefined

function finish(result: ConvertResult) {
  const zip = result.zip
  const report: unknown = JSON.parse(result.report)
  result.free()
  return { zip, report }
}

function run(module: PluginModule, fn: PluginFunction, args: unknown[]) {
  switch (fn) {
    case 'scan':
      return JSON.parse(module.scan(args[0] as Uint8Array)) as unknown
    case 'convert':
      return finish(module.convert(args[0] as Uint8Array, args[1] as string, (args[2] as string | undefined) ?? undefined))
    case 'repair':
      return finish(module.repair(args[0] as Uint8Array))
    case 'supportedVersions':
      return module.supportedVersions()
  }
}

const messageOf = (error: unknown) => (error instanceof Error ? error.message : String(error))

scope.onmessage = async ({ data }) => {
  if (data.type === 'init') {
    const url = URL.createObjectURL(new Blob([data.glue], { type: 'text/javascript' }))
    try {
      const module = (await import(/* @vite-ignore */ url)) as PluginModule
      await module.default({ module_or_path: data.wasm })
      plugin = module
      scope.postMessage({ type: 'ready' })
    } catch (error) {
      scope.postMessage({ type: 'fatal', message: messageOf(error) })
    } finally {
      URL.revokeObjectURL(url)
    }
    return
  }
  if (!plugin) {
    scope.postMessage({ type: 'failure', id: data.id, message: 'the plugin is not loaded' })
    return
  }
  try {
    const value = run(plugin, data.fn, data.args)
    const transfer = value && typeof value === 'object' && 'zip' in value ? [(value as { zip: Uint8Array }).zip.buffer] : []
    scope.postMessage({ type: 'result', id: data.id, value }, transfer)
  } catch (error) {
    scope.postMessage({ type: 'failure', id: data.id, message: messageOf(error) })
  }
}
