import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import { router } from './router'
import { applyCachedLanguage } from './i18n'
import { applyCachedUi } from './stores/settings'
import './style.css'

applyCachedLanguage()
applyCachedUi()

// WebView2's own menu (Back, Reload, Print...) and browser shortcuts would reload or navigate the app.
// Text fields keep their native Cut/Copy/Paste menu; dev builds keep everything for "Inspect".
if (!import.meta.env.DEV) {
  const isTextField = (target: EventTarget | null) =>
    target instanceof HTMLElement && (target.isContentEditable || target.closest('input, textarea') !== null)
  window.addEventListener('contextmenu', (event) => {
    if (!isTextField(event.target) && !window.getSelection()?.toString()) event.preventDefault()
  })
  window.addEventListener('keydown', (event) => {
    const key = event.key.toLowerCase()
    const ctrl = event.ctrlKey || event.metaKey
    const blocked =
      key === 'f5' ||
      key === 'browserback' ||
      key === 'browserforward' ||
      (ctrl && (key === 'r' || key === 'p' || key === 'u' || key === 's' || key === 'f' || key === 'g')) ||
      (ctrl && event.shiftKey && (key === 'i' || key === 'j' || key === 'c')) ||
      (event.altKey && (key === 'arrowleft' || key === 'arrowright'))
    if (blocked) event.preventDefault()
  })
}
createApp(App).use(createPinia()).use(router).mount('#app')
