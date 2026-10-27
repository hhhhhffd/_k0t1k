import { defineStore } from 'pinia'
import { ref } from 'vue'
import * as api from '../api/commands'
import type { ModCompat } from '../api/types'
import { t } from '../i18n'
import { LOADER_LABELS, minecraftVersionOf } from '../utils/format'
import { useInstancesStore } from './instances'

/** The question before launching an instance with enabled mods that were made for another version or loader. */
export interface ModCompatPrompt {
  instanceId: string
  minecraftVersion: string
  mods: ModCompat[]
  answer: (choice: 'launch' | 'disable' | 'cancel') => void
}

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
    const instance = useInstancesStore().instances.find((i) => i.id === instanceId)
    const choice = await new Promise<'launch' | 'disable' | 'cancel'>((resolve) => {
      prompt.value = { instanceId, minecraftVersion: instance ? minecraftVersionOf(instance) : '?', mods, answer: resolve }
    })
    prompt.value = null
    if (choice === 'cancel') return false
    if (choice === 'disable') {
      for (const mod of mods) await api.toggleResource(instanceId, 'mod', mod.fileName, false)
      await refresh(instanceId).catch((e: unknown) => console.warn('cannot refresh the mod check', e))
    }
    return true
  }

  return { byFile, prompt, refresh, label, hint, confirmLaunch }
})
