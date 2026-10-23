import { createApp } from 'vue'
import { createPinia } from 'pinia'
import App from './App.vue'
import { router } from './router'
import { applyCachedUi } from './stores/settings'
import './style.css'

applyCachedUi()
createApp(App).use(createPinia()).use(router).mount('#app')
