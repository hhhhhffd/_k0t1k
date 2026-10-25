import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import { router } from './router'
import { applyCachedLanguage } from './i18n'
import { applyCachedUi } from './stores/settings'
import './style.css'

applyCachedLanguage()
applyCachedUi()
createApp(App).use(createPinia()).use(router).mount('#app')
