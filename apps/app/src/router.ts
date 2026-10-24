import { createRouter, createWebHashHistory } from 'vue-router'

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: '/', name: 'library', component: () => import('./views/LibraryView.vue') },
    { path: '/instance/:id', name: 'instance', component: () => import('./views/InstanceView.vue'), props: true },
    { path: '/settings', name: 'settings', component: () => import('./views/SettingsView.vue') },
  ],
})
