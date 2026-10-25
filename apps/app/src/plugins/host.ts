import { readPluginFile } from '../api/commands'
import type { PluginId, PluginInfo } from '../api/types'
import { t } from '../i18n'
import { usePluginsStore } from '../stores/plugins'
import type { PluginFunction, WorkerRequest, WorkerResponse } from './worker'

/** A call that runs longer than this terminates the worker. */
export const CALL_TIMEOUT_MS = 120_000

interface Pending {
  resolve: (value: unknown) => void
  reject: (error: Error) => void
  timer: ReturnType<typeof setTimeout>
}

/** One worker per plugin, created on first use and kept alive. */
interface Slot {
  worker: Worker
  version: string
  ready: Promise<void>
  pending: Map<number, Pending>
}

const slots = new Map<PluginId, Slot>()
let nextCallId = 0

async function loadFiles(info: PluginInfo) {
  const read = async (name: string): Promise<ArrayBuffer> => {
    if (info.source === 'installed') return readPluginFile(info.id, info.version, name)
    // The bundled baseline is part of the frontend assets, signed together with the exe.
    const response = await fetch(`${import.meta.env.BASE_URL}plugins/${info.id}/${name}`)
    if (!response.ok) throw new Error(`${name}: HTTP ${response.status}`)
    return response.arrayBuffer()
  }
  const glueName = info.files.find((name) => name.endsWith('.js'))
  const wasmName = info.files.find((name) => name.endsWith('.wasm'))
  if (!glueName || !wasmName) throw new Error(`plugin ${info.id} has no glue or wasm file`)
  const [glue, wasm] = await Promise.all([read(glueName), read(wasmName)])
  return { glue: new TextDecoder().decode(glue), wasm }
}

function kill(id: PluginId, slot: Slot, error: Error) {
  slot.worker.terminate()
  if (slots.get(id) === slot) slots.delete(id)
  for (const call of slot.pending.values()) {
    clearTimeout(call.timer)
    call.reject(error)
  }
  slot.pending.clear()
}

function start(info: PluginInfo): Slot {
  const worker = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module' })
  const slot: Slot = { worker, version: info.version, ready: Promise.resolve(), pending: new Map() }
  slot.ready = new Promise<void>((resolve, reject) => {
    const fail = (error: Error) => {
      kill(info.id, slot, error)
      reject(error)
    }
    worker.onmessage = ({ data }: MessageEvent<WorkerResponse>) => {
      if (data.type === 'ready') return resolve()
      if (data.type === 'fatal') return fail(new Error(data.message))
      const call = slot.pending.get(data.id)
      if (!call) return
      slot.pending.delete(data.id)
      clearTimeout(call.timer)
      if (data.type === 'result') call.resolve(data.value)
      else call.reject(new Error(data.message))
    }
    worker.onerror = (event) => fail(new Error(event.message || 'the plugin worker failed'))
    worker.onmessageerror = () => fail(new Error('the plugin worker sent an unreadable message'))
    loadFiles(info)
      .then(({ glue, wasm }) => worker.postMessage({ type: 'init', glue, wasm } satisfies WorkerRequest, [wasm]))
      .catch((error: unknown) => fail(error instanceof Error ? error : new Error(String(error))))
  })
  slots.set(info.id, slot)
  return slot
}

/**
 * Runs `fn` of plugin `id` in its worker. `transfer` lists buffers moved to the worker (the caller's
 * copy becomes unusable). Rejects with the plugin's error; a call that exceeds [`CALL_TIMEOUT_MS`]
 * terminates the worker, and the next call starts a fresh one.
 */
export async function callPlugin<T>(id: PluginId, fn: PluginFunction, args: unknown[], transfer: Transferable[] = []): Promise<T> {
  const info = await usePluginsStore().infoOf(id)
  let slot = slots.get(id)
  // A newer copy was installed since the worker started: use it for the next call once the old one is idle.
  if (slot && slot.version !== info.version && slot.pending.size === 0) {
    kill(id, slot, new Error('the plugin was updated'))
    slot = undefined
  }
  slot ??= start(info)
  const running = slot
  await running.ready
  return new Promise<T>((resolve, reject) => {
    const callId = ++nextCallId
    const timer = setTimeout(() => kill(id, running, new Error(t.plugins.timeout(Math.round(CALL_TIMEOUT_MS / 1000)))), CALL_TIMEOUT_MS)
    running.pending.set(callId, { resolve: resolve as (value: unknown) => void, reject, timer })
    running.worker.postMessage({ type: 'call', id: callId, fn, args } satisfies WorkerRequest, transfer)
  })
}
