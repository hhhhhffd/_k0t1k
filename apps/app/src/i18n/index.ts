// No i18n library on purpose. `t` is a reactive object holding the active dictionary; switching the language
// replaces its sections in place, so `t.section.key` call sites need no change.
import { ref, shallowReactive } from 'vue'
import { en, type Dictionary } from './en'
import { ru } from './ru'

export type Language = 'en' | 'ru'

/** Shown in the picker; each language is named in itself. */
export const LANGUAGES: { id: Language; label: string }[] = [
  { id: 'en', label: 'English' },
  { id: 'ru', label: 'Русский' },
]

const dictionaries: Record<Language, Dictionary> = { en, ru }
const CACHE_KEY = 'k0t1k.language'

export const language = ref<Language>('en')
export const t: Dictionary = shallowReactive({ ...en })

export const isLanguage = (value: unknown): value is Language => value === 'en' || value === 'ru'

/** Switches the dictionary. The choice is cached so the first paint after a restart is already right. */
export function setLanguage(next: Language) {
  language.value = next
  Object.assign(t, dictionaries[next])
  document.documentElement.lang = next
  try {
    localStorage.setItem(CACHE_KEY, next)
  } catch {
    // only avoids a flash of the wrong language on startup; settings.json stays the source of truth
  }
}

/** Apply the last known language before the first paint, before the settings finish loading. */
export function applyCachedLanguage() {
  try {
    const cached = localStorage.getItem(CACHE_KEY)
    if (isLanguage(cached)) setLanguage(cached)
  } catch {
    // storage unavailable: stay on the default
  }
}
