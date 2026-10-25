import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api/commands'
import type { JarScan, ScanBadge, ScanResult, ScanVerdict } from '../api/types'
import { t } from '../i18n'
import { callPlugin } from '../plugins/host'
import { errorMessage } from '../utils/format'
import { usePluginsStore } from './plugins'
import { useTasksStore } from './tasks'

/** RatScan calls its worst verdict "malicious"; the launcher says "dangerous". */
export const badgeOf = (verdict: ScanVerdict): ScanBadge => (verdict === 'malicious' ? 'dangerous' : verdict)

export interface ScanProgress {
  instanceId: string
  done: number
  total: number
}

/** The question before launching an instance that has a dangerous enabled mod. */
export interface LaunchPrompt {
  instanceId: string
  mods: JarScan[]
  answer: (choice: 'launch' | 'disable' | 'cancel') => void
}

export const useRatscanStore = defineStore('ratscan', () => {
  const tasks = useTasksStore()
  const plugins = usePluginsStore()
  /** Jars of every instance seen so far, with cached results. */
  const jars = ref<Record<string, JarScan[]>>({})
  const progress = ref<ScanProgress | null>(null)
  const launchPrompt = ref<LaunchPrompt | null>(null)
  // Scans run one after another: each jar is a synchronous call in the single worker anyway.
  let queue: Promise<unknown> = Promise.resolve()

  async function refresh(instanceId: string) {
    jars.value[instanceId] = await api.ratscanStatus(instanceId)
    return jars.value[instanceId]
  }

  async function scanJar(instanceId: string, jar: JarScan, pluginVersion: string) {
    const bytes = new Uint8Array(await api.readResourceFile(instanceId, 'mod', jar.fileName))
    const result = await callPlugin<ScanResult>('ratscan', 'scan', [bytes], [bytes.buffer])
    await api.ratscanStore(instanceId, jar.sha1, pluginVersion, result)
  }

  async function run(instanceId: string, only: string[] | null, silent: boolean) {
    const pluginVersion = (await plugins.infoOf('ratscan')).version
    const all = await refresh(instanceId)
    const seen = new Set<string>()
    const targets = all.filter((jar) => {
      if (jar.scan || (only && !only.includes(jar.fileName)) || seen.has(jar.sha1)) return false
      seen.add(jar.sha1)
      return true
    })
    progress.value = { instanceId, done: 0, total: targets.length }
    let failed = 0
    let done = 0
    for (const jar of targets) {
      try {
        await scanJar(instanceId, jar, pluginVersion)
      } catch (e) {
        failed++
        console.warn(`scan of ${jar.fileName} failed`, e)
        if (!silent || failed === 1) tasks.toast(t.scan.failed(jar.fileName, errorMessage(e)), 'error')
      }
      progress.value = { instanceId, done: ++done, total: targets.length }
    }
    progress.value = null
    await refresh(instanceId)
    return targets.length - failed
  }

  /** Scans the jars without a result for the running plugin version (`only` limits it to some file names). */
  function scan(instanceId: string, only: string[] | null = null, silent = false) {
    const next = queue.then(() => run(instanceId, only, silent))
    queue = next.catch(() => undefined)
    return next
  }

  /** Background scan of jars that were just added; never throws and never blocks the caller. */
  function scanAdded(instanceId: string, fileNames: string[]) {
    const jarNames = fileNames.filter((name) => /\.jar(\.disabled)?$/i.test(name))
    if (!jarNames.length) return
    scan(instanceId, jarNames, true).then(
      () => announceDangerous(instanceId, jarNames),
      (e: unknown) => console.warn('background scan failed', e),
    )
  }

  function announceDangerous(instanceId: string, fileNames: string[]) {
    for (const jar of jars.value[instanceId] ?? []) {
      if (fileNames.includes(jar.fileName) && jar.scan && badgeOf(jar.scan.verdict) === 'dangerous') {
        tasks.toast(t.scan.dangerousFound(jar.fileName), 'error')
      }
    }
  }

  const report = (instanceId: string, sha1: string) => api.ratscanReport(instanceId, sha1)

  /**
   * Asks what to do when an enabled mod is dangerous. Resolves to false when the launch is cancelled.
   * Unscanned mods are not a reason to stop: the answer is based on the results that exist.
   */
  async function confirmLaunch(instanceId: string): Promise<boolean> {
    let dangerous: JarScan[]
    try {
      dangerous = (await api.ratscanStatus(instanceId)).filter((jar) => jar.enabled && jar.scan && badgeOf(jar.scan.verdict) === 'dangerous')
    } catch (e) {
      // A broken scan cache must not stop the game.
      console.warn('cannot read the mod scan results', e)
      return true
    }
    if (!dangerous.length) return true
    const choice = await new Promise<'launch' | 'disable' | 'cancel'>((resolve) => {
      launchPrompt.value = { instanceId, mods: dangerous, answer: resolve }
    })
    launchPrompt.value = null
    if (choice === 'cancel') return false
    if (choice === 'disable') {
      for (const jar of dangerous) await api.toggleResource(instanceId, 'mod', jar.fileName, false)
      await refresh(instanceId)
    }
    return true
  }

  return { jars, progress, launchPrompt, refresh, scan, scanAdded, report, confirmLaunch }
})
