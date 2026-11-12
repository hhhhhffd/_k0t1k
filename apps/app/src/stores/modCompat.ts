import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api/commands'
import type { MissingDependency, ModCompat } from '../api/types'
import { t } from '../i18n'
import { errorMessage, LOADER_LABELS, minecraftVersionOf } from '../utils/format'
import { baseName } from '../utils/useFileDrop'
import { useInstancesStore } from './instances'
import { useRatscanStore } from './ratscan'
import { useTasksStore } from './tasks'

/** Compatibility advice when adding, launching, or repairing installed mods. */
export interface ModCompatPrompt {
  instanceId: string
  /** In `add` mode, 'disable' means skip the incoming jars. `repair` never launches the game. */
  mode: 'launch' | 'add' | 'repair'
  paths: string[]
  minecraftVersion: string
  mods: ModCompat[]
  downloading: boolean
  message: string | null
  error: string | null
  answer: (choice: Choice) => void
}

type Choice = 'launch' | 'disable' | 'cancel'

const loaderNames = (mod: ModCompat) => mod.madeForLoaders.map((id) => LOADER_LABELS[id]).join(' / ')

const dependencyText = (dependency: MissingDependency) =>
  dependency.installed
    ? t.modCompat.depWrongVersion(dependency.id, dependency.wanted ?? '?', dependency.installed)
    : dependency.wanted
      ? t.modCompat.depMissingVersion(dependency.id, dependency.wanted)
      : t.modCompat.depMissing(dependency.id)

/** What the mod needs, in words: the loader or Minecraft version it is made for, then the mods it misses. */
export const needsOf = (mod: ModCompat) =>
  [mod.loaderMismatch ? t.modCompat.needsLoader(loaderNames(mod)) : mod.minecraftMismatch ? t.modCompat.needsVersion[mod.source ?? 'descriptor'](mod.needs ?? '?') : '', ...mod.dependencies.map(dependencyText)]
    .filter(Boolean)
    .join('; ')

export const useModCompatStore = defineStore('modCompat', () => {
  /** Mismatched mods of every instance seen so far, by file name. */
  const byFile = ref<Record<string, Record<string, ModCompat>>>({})
  const prompt = ref<ModCompatPrompt | null>(null)
  /** Resource lists reload after dependency downloads, including a partially failed install. */
  const changed = ref(0)

  async function refresh(instanceId: string) {
    const found = await api.modCompat(instanceId)
    byFile.value[instanceId] = Object.fromEntries(found.map((mod) => [mod.fileName, mod]))
    return found
  }

  /** The badge text: the loader or version problem first, else the missing mod (or how many). */
  const label = (mod: ModCompat) => {
    if (mod.loaderMismatch) return t.modCompat.madeForLoader(loaderNames(mod))
    if (mod.minecraftMismatch) return t.modCompat.madeFor[mod.source ?? 'descriptor'](mod.needs ?? '?')
    return mod.dependencies.length === 1 ? dependencyText(mod.dependencies[0]) : t.modCompat.depsLabel(mod.dependencies.length)
  }
  /** The badge tooltip: every problem, and who says so. */
  const hint = (mod: ModCompat) => {
    if (mod.loaderMismatch) return t.modCompat.loaderHint(loaderNames(mod))
    const parts = mod.dependencies.length ? [t.modCompat.depsHint(mod.dependencies.map(dependencyText).join('; '))] : []
    if (mod.minecraftMismatch) parts.unshift(t.modCompat.versionHint[mod.source ?? 'descriptor'](mod.needs ?? '?'))
    return parts.join(' ')
  }

  async function ask(instanceId: string, mode: ModCompatPrompt['mode'], mods: ModCompat[], paths: string[] = []): Promise<Choice> {
    const instance = useInstancesStore().instances.find((i) => i.id === instanceId)
    // A second question while one is open cancels the first one instead of leaving it pending forever.
    prompt.value?.answer('cancel')
    let current: ModCompatPrompt | undefined
    const choice = await new Promise<Choice>((resolve) => {
      current = { instanceId, mode, paths, minecraftVersion: instance ? minecraftVersionOf(instance) : '?', mods, downloading: false, message: null, error: null, answer: resolve }
      prompt.value = current
    })
    // Close the dialog only if it still shows this question; compared by `answer` because the ref holds a proxy.
    if (prompt.value && prompt.value.answer === current?.answer) prompt.value = null
    return choice
  }

  /** Opens the same repair flow from a mod badge or a notification; it never starts the game. */
  async function openDependencies(instanceId: string) {
    const mods = await useTasksStore().guard(() => refresh(instanceId))
    if (!mods) return
    const affected = mods.filter((mod) => mod.enabled && mod.dependencies.length)
    if (affected.length) await ask(instanceId, 'repair', affected)
    else useTasksStore().toast(t.modCompat.dependenciesReady, 'ok')
  }

  async function downloadDependencies() {
    const current = prompt.value
    if (!current || current.downloading) return
    const tasks = useTasksStore()
    current.downloading = true
    current.message = null
    current.error = null
    try {
      const report = await api.installMissingModDependencies(current.instanceId, current.paths)
      tasks.showBlocked(report.blocked, current.instanceId)
      const added = report.installed.map((mod) => mod.fileName)
      if (added.length) {
        useRatscanStore().scanAdded(current.instanceId, added)
        tasks.toast(t.modCompat.dependenciesInstalled(added.length), 'ok')
      }
      const mods = current.mode === 'add'
        ? await api.modCompatFiles(current.instanceId, current.paths)
        : (await refresh(current.instanceId)).filter((mod) => mod.enabled)
      // Another question may have replaced this one while the backend was working.
      if (prompt.value?.answer !== current.answer) return
      // Keep the caller's array in sync: "skip" / "disable" must act on the remaining problems only.
      current.mods.splice(0, current.mods.length, ...(current.mode === 'repair' ? mods.filter((mod) => mod.dependencies.length) : mods))
      if (report.unresolvedDependencies.length) {
        current.message = t.modCompat.dependenciesUnresolved(report.unresolvedDependencies.join(', '))
      } else if (!current.mods.some((mod) => mod.dependencies.length) && !report.blocked.length) {
        current.message = t.modCompat.dependenciesReady
      }
      // Adding continues automatically. A launch still needs an explicit click after repair.
      if (current.mode === 'add' && !current.mods.length && !report.blocked.length && !report.unresolvedDependencies.length) current.answer('launch')
    } catch (error) {
      current.error = errorMessage(error)
    } finally {
      current.downloading = false
      changed.value++
    }
  }

  /**
   * Checks jars before they are added. Resolves to the paths to add (mismatched ones dropped on "skip"),
   * or null when the user cancels. A failed check adds everything: it is advice.
   */
  async function confirmAdd(instanceId: string, paths: string[]): Promise<string[] | null> {
    let mods: ModCompat[]
    try {
      mods = await api.modCompatFiles(instanceId, paths)
    } catch (e) {
      console.warn('cannot check the new mods against the instance', e)
      return paths
    }
    if (!mods.length) return paths
    const choice = await ask(instanceId, 'add', mods, paths)
    if (choice === 'cancel') return null
    if (choice === 'launch') return paths
    const skipped = new Set(mods.map((mod) => mod.fileName))
    return paths.filter((path) => !skipped.has(baseName(path)))
  }

  /** Asks what to do when enabled mods declare another version or loader. Resolves to false when the launch is cancelled. */
  async function confirmLaunch(instanceId: string): Promise<boolean> {
    let mods: ModCompat[]
    try {
      mods = (await api.modCompat(instanceId)).filter((mod) => mod.enabled)
    } catch (e) {
      // The check is advice: failing to read the jars must not stop the game.
      console.warn('cannot check the mods against the instance', e)
      return true
    }
    if (!mods.length) return true
    const choice = await ask(instanceId, 'launch', mods)
    if (choice === 'cancel') return false
    if (choice === 'disable') {
      for (const mod of mods) await api.toggleResource(instanceId, 'mod', mod.fileName, false)
      await refresh(instanceId).catch((e: unknown) => console.warn('cannot refresh the mod check', e))
    }
    return true
  }

  return { byFile, prompt, changed, refresh, label, hint, confirmLaunch, confirmAdd, openDependencies, downloadDependencies }
})
