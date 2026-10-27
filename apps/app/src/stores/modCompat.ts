import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api/commands'
import type { ModCompat } from '../api/types'
import { t } from '../i18n'
import { LOADER_LABELS, minecraftVersionOf } from '../utils/format'
import { baseName } from '../utils/useFileDrop'
import { useInstancesStore } from './instances'

/** The question before launching an instance with enabled mods that were made for another version or loader. */
export interface ModCompatPrompt {
  instanceId: string
  /** `launch`: before starting the game; `add`: before copying new jars in ('disable' then means skip them). */
  mode: 'launch' | 'add'
  minecraftVersion: string
  mods: ModCompat[]
  answer: (choice: Choice) => void
}

type Choice = 'launch' | 'disable' | 'cancel'

const loaderNames = (mod: ModCompat) => mod.madeForLoaders.map((id) => LOADER_LABELS[id]).join(' / ')

/** What the mod needs, in words. */
export const needsOf = (mod: ModCompat) => (mod.loaderMismatch ? t.modCompat.needsLoader(loaderNames(mod)) : t.modCompat.needsVersion(mod.needs ?? '?'))

export const useModCompatStore = defineStore('modCompat', () => {
  /** Mismatched mods of every instance seen so far, by file name. */
  const byFile = ref<Record<string, Record<string, ModCompat>>>({})
  const prompt = ref<ModCompatPrompt | null>(null)

  async function refresh(instanceId: string) {
    const found = await api.modCompat(instanceId)
    byFile.value[instanceId] = Object.fromEntries(found.map((mod) => [mod.fileName, mod]))
    return found
  }

  const label = (mod: ModCompat) => (mod.loaderMismatch ? t.modCompat.madeForLoader(loaderNames(mod)) : t.modCompat.madeFor(mod.needs ?? '?'))
  const hint = (mod: ModCompat) => (mod.loaderMismatch ? t.modCompat.loaderHint(loaderNames(mod)) : t.modCompat.versionHint(mod.needs ?? '?'))

  async function ask(instanceId: string, mode: ModCompatPrompt['mode'], mods: ModCompat[]): Promise<Choice> {
    const instance = useInstancesStore().instances.find((i) => i.id === instanceId)
    // A second question while one is open cancels the first one instead of leaving it pending forever.
    prompt.value?.answer('cancel')
    let current: ModCompatPrompt | undefined
    const choice = await new Promise<Choice>((resolve) => {
      current = { instanceId, mode, minecraftVersion: instance ? minecraftVersionOf(instance) : '?', mods, answer: resolve }
      prompt.value = current
    })
    // Close the dialog only if it still shows this question; compared by `answer` because the ref holds a proxy.
    if (prompt.value && prompt.value.answer === current?.answer) prompt.value = null
    return choice
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
    const choice = await ask(instanceId, 'add', mods)
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

  return { byFile, prompt, refresh, label, hint, confirmLaunch, confirmAdd }
})
