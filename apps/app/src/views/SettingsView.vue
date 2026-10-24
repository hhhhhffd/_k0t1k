<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { AppWindow, Check, Coffee, FolderOpen, FolderSync, HardDrive, Info, Link, MemoryStick, Palette, RotateCcw, SlidersHorizontal, Terminal, Trash2, UserPlus, Users, Zap } from '@lucide/vue'
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { createShortcuts, deleteManagedJava, getSystemInfo, getUninstallPlan, isTauri, listManagedJava, listManagedJavaUsage, openDataDir, openLauncherFolder, openSharedFolder, setDataDir, uninstall } from '../api/commands'
import { type JavaInstall, type ManagedJavaUsage, SHARED_FOLDERS, type SharedFolder, type SystemInfo, type UninstallPlan } from '../api/types'
import mascot from '../assets/mascot.png'
import CommandFields from '../components/fields/CommandFields.vue'
import MemoryFields from '../components/fields/MemoryFields.vue'
import WindowFields from '../components/fields/WindowFields.vue'
import AccountLoginDialog from '../components/AccountLoginDialog.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import SelectMenu from '../components/SelectMenu.vue'
import ModalDialog from '../components/ModalDialog.vue'
import ThemeEditor from '../components/ThemeEditor.vue'
import ToggleSwitch from '../components/ToggleSwitch.vue'
import { t } from '../i18n/ru'
import { useCustomizationStore } from '../stores/customization'
import { useInstancesStore } from '../stores/instances'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'
import { THEMES, themeVars } from '../utils/theme'

const store = useSettingsStore()
const tasks = useTasksStore()
const instances = useInstancesStore()
const customization = useCustomizationStore()
const catOptions = computed(() => [
  { value: '', label: t.settings.noCat },
  { value: 'mascot', label: t.settings.mascotCat },
  ...customization.catPacks.map((p) => ({ value: p.id, label: p.name, hint: p.kind === 'video' ? t.settings.video : undefined })),
])
const iconSetOptions = computed(() => [
  { value: '', label: t.settings.builtinIcons },
  ...customization.iconSets.map((set) => ({ value: set.id, label: set.name, hint: `${Object.keys(set.files).length}` })),
])
const route = useRoute()
const router = useRouter()

const sectionIds = ['java', 'memory', 'jvm', 'window', 'commands', 'behavior', 'sharedFolders', 'appearance', 'accounts', 'storage', 'about'] as const
type Section = (typeof sectionIds)[number]
const sectionIcons = {
  java: { icon: Coffee, color: 'text-warn' },
  memory: { icon: MemoryStick, color: 'text-ok' },
  jvm: { icon: SlidersHorizontal, color: 'text-muted' },
  window: { icon: AppWindow, color: 'text-accent' },
  commands: { icon: Terminal, color: 'text-muted' },
  behavior: { icon: Zap, color: 'text-warn' },
  sharedFolders: { icon: FolderSync, color: 'text-warn' },
  appearance: { icon: Palette, color: 'text-accent' },
  accounts: { icon: Users, color: 'text-ok' },
  storage: { icon: HardDrive, color: 'text-muted' },
  about: { icon: Info, color: 'text-accent' },
} as const
const SECTION_KEY = 'k0t1k.settingsSection'
const asSection = (v: string | null | undefined) => (sectionIds.includes(v as Section) ? (v as Section) : null)
function storedSection() {
  try {
    return asSection(localStorage.getItem(SECTION_KEY))
  } catch {
    return null
  }
}
// The hash survives a reload of the window; localStorage covers opening Settings without one.
const section = ref<Section>(asSection(route.hash.slice(1)) ?? storedSection() ?? 'java')
watch(
  section,
  (id) => {
    try {
      localStorage.setItem(SECTION_KEY, id)
    } catch {
      // storage unavailable: the hash alone keeps the section
    }
    if (route.hash !== `#${id}`) void router.replace({ hash: `#${id}` })
  },
  { immediate: true },
)

const system = ref<SystemInfo | null>(null)
const managed = ref<JavaInstall[]>([])
const javaUsage = ref<ManagedJavaUsage[]>([])
const javaToDelete = ref<{ install: JavaInstall; usage: ManagedJavaUsage } | null>(null)
const newName = ref('')
const loginKind = ref<'microsoft' | 'elyby' | null>(null)
const restartNeeded = ref(false)
const uninstallPlan = ref<UninstallPlan | null>(null)

const s = computed(() => store.settings)
const themeTiles = computed(() => [
  ...THEMES.map((th) => ({ id: th.id as string, label: th.label, base: th.id, vars: {} as Record<string, string> })),
  ...[...(s.value?.ui.customThemes ?? []), ...store.themeFiles].map((c) => ({ id: c.id, label: c.name, base: c.base, vars: themeVars(c.colors) })),
])

const onGameLaunchOptions = Object.entries(t.settings.onGameLaunchOptions).map(([value, label]) => ({ value, label }))
const onCloseOptions = Object.entries(t.settings.onCloseOptions).map(([value, label]) => ({ value, label }))

/** The two settings only make sense together, so the page shows them as one switch. */
const autoJava = computed({
  get: () => !!s.value && s.value.java.autoSelect && s.value.java.autoDownload,
  set: (on: boolean) => {
    if (!s.value) return
    s.value.java.autoSelect = on
    s.value.java.autoDownload = on
  },
})
const usageOf = (j: JavaInstall) => javaUsage.value.find((u) => u.path === j.path)
const instanceNames = (ids: string[]) => ids.map((id) => instances.byId(id)?.name ?? id).join(', ')

async function loadManagedJava() {
  managed.value = (await tasks.guard(listManagedJava)) ?? []
  javaUsage.value = (await tasks.guard(listManagedJavaUsage)) ?? []
}
/** A runtime nobody needs goes at once; one that some instance needs is deleted only after asking. */
async function removeJava(install: JavaInstall) {
  const usage = usageOf(install)
  if (!usage) return
  if (usage.usedBy.length) javaToDelete.value = { install, usage }
  else await deleteJava(usage)
}
async function deleteJava(usage: ManagedJavaUsage) {
  javaToDelete.value = null
  const done = await tasks.guard(() => deleteManagedJava(usage.dir).then(() => true))
  if (done) tasks.toast(t.settings.javaDeleted, 'ok')
  await loadManagedJava()
}

onMounted(async () => {
  system.value = (await tasks.guard(getSystemInfo)) ?? null
  await loadManagedJava()
})

// Autosave: every edit of the settings object is persisted after a short pause.
let timer: ReturnType<typeof setTimeout> | undefined
watch(
  () => JSON.stringify(store.settings),
  (now, before) => {
    if (before === 'null' || now === before) return
    clearTimeout(timer)
    timer = setTimeout(() => void tasks.guard(store.save), 400)
  },
)

async function changeDataDir(reset: boolean) {
  // Outside Tauri there is no native dialog; the mock accepts a fake path.
  const picked = reset ? null : isTauri ? await open({ directory: true, multiple: false, title: t.settings.pickDataDir }) : 'D:\\k0t1k-data'
  if (!reset && typeof picked !== 'string') return
  const needsRestart = await tasks.guard(() => setDataDir(picked))
  if (needsRestart === undefined) return
  restartNeeded.value = needsRestart
  system.value = (await tasks.guard(getSystemInfo)) ?? system.value
}

async function makeShortcuts() {
  if (await tasks.guard(() => createShortcuts(true, true))) tasks.toast(t.settings.shortcutsCreated, 'ok')
}

async function askUninstall() {
  uninstallPlan.value = (await tasks.guard(getUninstallPlan)) ?? null
}

async function confirmUninstall() {
  uninstallPlan.value = null
  await tasks.guard(uninstall)
}

function setSharedDefault(folder: SharedFolder, on: boolean) {
  if (s.value) s.value.sharedFoldersDefault = SHARED_FOLDERS.filter((f) => (f === folder ? on : s.value!.sharedFoldersDefault.includes(f)))
}

async function addAccount() {
  const name = newName.value.trim()
  if (!name) return
  newName.value = ''
  await tasks.guard(() => store.addAccount(name))
}
</script>

<template>
  <div v-if="s" class="mx-auto flex max-w-5xl gap-6 p-6">
    <nav class="sticky top-6 flex h-fit w-44 shrink-0 flex-col gap-1" :aria-label="t.settings.title">
      <h1 class="mb-2 px-3 text-xl font-bold">{{ t.settings.title }}</h1>
      <button
        v-for="id in sectionIds"
        :key="id"
        class="flex h-8 items-center gap-2.5 rounded-lg px-3 text-left font-medium transition-colors"
        :class="section === id ? 'bg-accent/15 text-accent' : 'text-fg hover:bg-hover'"
        @click="section = id"
      >
        <component :is="sectionIcons[id].icon" :size="16" :class="section === id ? '' : sectionIcons[id].color" />
        {{ t.settings.sections[id] }}
      </button>
    </nav>

    <div class="flex min-w-0 max-w-2xl flex-1 flex-col gap-4">
      <h2 class="text-[17px] font-bold">{{ t.settings.sections[section] }}</h2>

      <template v-if="section === 'java'">
        <section class="card flex flex-col gap-3 p-4">
          <p class="text-muted">{{ t.settings.javaIntro }}</p>
          <label class="flex items-center gap-3">
            <ToggleSwitch v-model="autoJava" :label="t.settings.autoJava" />
            <span><span class="block font-medium">{{ t.settings.autoJava }}</span><span class="text-[12px] text-faint">{{ t.settings.autoJavaHint }}</span></span>
          </label>
          <div v-if="s.java.path" class="flex items-center gap-3 rounded-lg border border-warn/40 bg-warn/10 px-3 py-2 text-[12.5px] text-warn">
            <span class="min-w-0 flex-1">{{ t.settings.globalJavaForced }} <span class="font-mono break-all">{{ s.java.path }}</span></span>
            <button class="btn h-7 shrink-0" @click="s.java.path = null"><RotateCcw :size="14" />{{ t.settings.resetJava }}</button>
          </div>
          <p class="text-[12px] text-faint">{{ t.settings.javaPerInstance }}</p>
        </section>
        <section class="card p-4">
          <h3 class="section-title mb-2">{{ t.settings.managed }}</h3>
          <ul class="flex flex-col gap-1.5">
            <li v-for="j in managed" :key="j.path" class="flex items-center gap-3 rounded-lg bg-raised px-3 py-2">
              <span class="badge badge-accent">Java {{ j.major }}</span>
              <div class="min-w-0 flex-1">
                <div class="font-medium">{{ j.vendor }} {{ j.version }}</div>
                <div class="truncate text-[12px]" :class="usageOf(j)?.usedBy.length ? 'text-muted' : 'text-faint'">
                  {{ usageOf(j)?.usedBy.length ? t.settings.javaUsedBy(instanceNames(usageOf(j)!.usedBy)) : t.settings.javaUnused }}
                </div>
              </div>
              <button
                class="btn btn-ghost btn-icon shrink-0 text-danger"
                :disabled="!usageOf(j) || usageOf(j)!.running"
                :title="usageOf(j)?.running ? t.settings.javaRunning : t.common.delete"
                :aria-label="t.common.delete"
                @click="removeJava(j)"
              >
                <Trash2 :size="15" />
              </button>
            </li>
            <li v-if="!managed.length" class="text-faint">{{ t.settings.noManagedJava }}</li>
          </ul>
        </section>
        <ConfirmDialog
          v-if="javaToDelete"
          :title="t.settings.deleteJavaTitle(javaToDelete.install.major)"
          :text="t.settings.deleteJavaText(instanceNames(javaToDelete.usage.usedBy))"
          @confirm="deleteJava(javaToDelete.usage)"
          @close="javaToDelete = null"
        />
      </template>

      <section v-else-if="section === 'memory'" class="card p-4"><MemoryFields v-model="s.memory" :total-ram-mib="system?.totalRamMib" /></section>

      <section v-else-if="section === 'jvm'" class="card p-4">
        <textarea v-model="s.jvmArgs" class="input font-mono" rows="4" :aria-label="t.settings.sections.jvm" />
      </section>

      <section v-else-if="section === 'window'" class="card p-4"><WindowFields v-model="s.window" /></section>

      <section v-else-if="section === 'commands'" class="card p-4"><CommandFields v-model="s.commands" /></section>

      <template v-else-if="section === 'appearance'">
        <section class="card p-4">
          <h3 class="section-title mb-3">{{ t.settings.theme }}</h3>
          <div class="grid grid-cols-2 gap-3">
            <button
              v-for="th in themeTiles"
              :key="th.id"
              :data-theme="th.base"
              class="overflow-hidden rounded-xl border-2 text-left transition-colors"
              :class="s.ui.theme === th.id ? 'border-accent' : 'border-line hover:border-faint'"
              :aria-pressed="s.ui.theme === th.id"
              :style="{ ...th.vars, background: 'var(--c-base)', color: 'var(--c-fg)' }"
              @click="store.setUi({ theme: th.id })"
            >
              <div class="flex h-20">
                <div class="w-1/4" style="background: var(--c-surface); border-right: 1px solid var(--c-line)">
                  <div class="m-2 h-1.5 rounded" style="background: var(--c-accent)" />
                  <div class="m-2 h-1.5 rounded" style="background: var(--c-line)" />
                  <div class="m-2 h-1.5 rounded" style="background: var(--c-line)" />
                </div>
                <div class="flex-1 p-2">
                  <div class="mb-1.5 h-2 w-1/2 rounded" style="background: var(--c-fg)" />
                  <div class="flex gap-1.5">
                    <div class="h-9 flex-1 rounded" style="background: var(--c-raised); border: 1px solid var(--c-line)" />
                    <div class="h-9 flex-1 rounded" style="background: var(--c-raised); border: 1px solid var(--c-line)" />
                  </div>
                  <div class="mt-1.5 h-3 w-1/3 rounded" style="background: var(--c-accent)" />
                </div>
              </div>
              <div class="flex items-center justify-between px-3 py-2" style="background: var(--c-surface)">
                <span class="font-medium">{{ th.label }}</span>
                <Check v-if="s.ui.theme === th.id" :size="15" style="color: var(--c-accent)" />
              </div>
            </button>
          </div>
        </section>
        <ThemeEditor />
        <section class="card flex flex-col gap-4 p-4">
          <div class="flex items-center justify-between">
            <h3 class="section-title">{{ t.settings.customization }}</h3>
            <button class="btn h-7" @click="customization.refresh()"><RotateCcw :size="14" />{{ t.settings.rescanFolders }}</button>
          </div>
          <div class="grid grid-cols-[160px_1fr_auto] items-center gap-x-3 gap-y-3">
            <span class="label">{{ t.settings.catPack }}</span>
            <SelectMenu :model-value="s.ui.catPack ?? ''" :options="catOptions" :aria-label="t.settings.catPack" @update:model-value="store.setUi({ catPack: $event || null })" />
            <button class="btn h-8" @click="tasks.guard(() => openLauncherFolder('catpacks'))"><FolderOpen :size="14" />{{ t.settings.openFolder }}</button>
            <span class="label">{{ t.settings.iconSet }}</span>
            <SelectMenu :model-value="s.ui.iconSet ?? ''" :options="iconSetOptions" :aria-label="t.settings.iconSet" @update:model-value="store.setUi({ iconSet: $event || null })" />
            <button class="btn h-8" @click="tasks.guard(() => openLauncherFolder('iconsets'))"><FolderOpen :size="14" />{{ t.settings.openFolder }}</button>
            <span class="label">{{ t.settings.themeFiles }}</span>
            <span class="text-[12px] text-faint">{{ t.settings.themeFilesHint(store.themeFiles.length) }}</span>
            <button class="btn h-8" @click="tasks.guard(() => openLauncherFolder('themes'))"><FolderOpen :size="14" />{{ t.settings.openFolder }}</button>
          </div>
          <p class="text-[12px] text-faint">{{ t.settings.customizationHint }}</p>
        </section>
        <section class="card flex items-center gap-3 p-4">
          <ToggleSwitch :model-value="s.ui.liteMode" :label="t.settings.lite" @update:model-value="store.setUi({ liteMode: $event })" />
          <span><span class="block font-medium">{{ t.settings.lite }}</span><span class="text-[12px] text-faint">{{ t.settings.liteHint }}</span></span>
        </section>
      </template>

      <section v-else-if="section === 'behavior'" class="card flex flex-col gap-4 p-4">
        <div class="flex flex-col gap-2">
          <span><span class="block font-medium">{{ t.settings.onGameLaunch }}</span><span class="text-[12px] text-faint">{{ t.settings.onGameLaunchHint }}</span></span>
          <div class="flex gap-1 rounded-lg bg-raised p-1" role="radiogroup" :aria-label="t.settings.onGameLaunch">
            <button
              v-for="o in onGameLaunchOptions"
              :key="o.value"
              role="radio"
              :aria-checked="s.behavior.onGameLaunch === o.value"
              class="h-8 flex-1 rounded-md px-2 font-medium transition-colors"
              :class="s.behavior.onGameLaunch === o.value ? 'bg-accent/15 text-accent' : 'text-muted hover:bg-hover hover:text-fg'"
              @click="s.behavior.onGameLaunch = o.value as typeof s.behavior.onGameLaunch"
            >{{ o.label }}</button>
          </div>
          <p class="text-[12px] text-muted">{{ t.settings.onGameLaunchExplain[s.behavior.onGameLaunch] }}</p>
        </div>
        <div class="flex flex-col gap-2">
          <span><span class="block font-medium">{{ t.settings.onClose }}</span><span class="text-[12px] text-faint">{{ t.settings.onCloseHint }}</span></span>
          <div class="flex gap-1 rounded-lg bg-raised p-1" role="radiogroup" :aria-label="t.settings.onClose">
            <button
              v-for="o in onCloseOptions"
              :key="o.value"
              role="radio"
              :aria-checked="s.behavior.onClose === o.value"
              class="h-8 flex-1 rounded-md px-2 font-medium transition-colors"
              :class="s.behavior.onClose === o.value ? 'bg-accent/15 text-accent' : 'text-muted hover:bg-hover hover:text-fg'"
              @click="s.behavior.onClose = o.value as typeof s.behavior.onClose"
            >{{ o.label }}</button>
          </div>
        </div>
        <div class="flex items-center gap-4">
          <span class="min-w-0 flex-1"><span class="block font-medium">{{ t.settings.discord }}</span><span class="text-[12px] text-faint">{{ t.settings.discordHint }}</span></span>
          <ToggleSwitch v-model="s.discord.enabled" :label="t.settings.discord" />
        </div>
      </section>

      <section v-else-if="section === 'sharedFolders'" class="card flex flex-col gap-3 p-4">
        <p class="text-[12px] text-faint">{{ t.settings.sharedDefaultsHint }}</p>
        <h3 class="text-[14px] font-semibold">{{ t.settings.sharedDefaults }}</h3>
        <div v-for="folder in SHARED_FOLDERS" :key="folder" class="flex items-center gap-3">
          <ToggleSwitch
            :model-value="s.sharedFoldersDefault.includes(folder)"
            :label="t.sharedFolder.names[folder]"
            @update:model-value="setSharedDefault(folder, $event)"
          />
          <span class="min-w-0 flex-1 font-medium">{{ t.sharedFolder.names[folder] }}</span>
          <button class="btn h-7" @click="tasks.guard(() => openSharedFolder(folder))"><FolderOpen :size="15" />{{ t.settings.open }}</button>
        </div>
      </section>

      <template v-else-if="section === 'accounts'">
        <section class="card p-4">
          <ul class="flex flex-col gap-1.5">
            <li v-for="acc in s.accounts" :key="acc.id" class="flex items-center gap-3 rounded-lg bg-raised px-3 py-2">
              <span class="min-w-0 flex-1 truncate font-medium">{{ acc.name }}</span>
              <span class="badge">{{ t.account.kinds[acc.kind] }}</span>
              <span v-if="s.selectedAccountId === acc.id" class="badge badge-accent"><Check :size="12" />{{ t.settings.selected }}</span>
              <button v-else class="btn h-7" @click="tasks.guard(() => store.selectAccount(acc.id))">{{ t.settings.select }}</button>
              <button class="btn btn-ghost btn-icon h-7 text-muted hover:!text-danger" :aria-label="t.common.delete" @click="tasks.guard(() => store.removeAccount(acc.id))"><Trash2 :size="15" /></button>
            </li>
          </ul>
          <div class="mt-3 flex flex-wrap gap-2">
            <button class="btn" @click="loginKind = 'microsoft'"><UserPlus :size="15" />{{ t.account.signInMicrosoft }}</button>
            <button class="btn" @click="loginKind = 'elyby'"><UserPlus :size="15" />{{ t.account.signInElyby }}</button>
          </div>
          <form class="mt-3 flex gap-2" @submit.prevent="addAccount">
            <input v-model="newName" class="input" :placeholder="t.settings.nickname" maxlength="16" :aria-label="t.settings.nickname" />
            <button class="btn btn-primary" type="submit" :disabled="!newName.trim()"><UserPlus :size="15" />{{ t.settings.addAccount }}</button>
          </form>
        </section>
      </template>

      <template v-else-if="section === 'storage'">
        <section v-if="system" class="card flex flex-col gap-3 p-4">
          <dl class="grid grid-cols-[130px_1fr] items-baseline gap-y-2">
            <dt class="text-faint">{{ t.settings.dataDir }}</dt>
            <dd class="selectable font-mono text-[12px] break-all">{{ system.dataDir }}</dd>
            <dt class="text-faint">{{ t.settings.mode }}</dt>
            <dd>
              <span class="badge" :class="system.portable ? 'badge-accent' : ''">{{ system.portable ? t.settings.portable : t.settings.installed }}</span>
              <span class="ml-2 text-[12px] text-faint">{{ system.portable ? t.settings.portableHint : t.settings.installedHint }}</span>
            </dd>
            <dt class="text-faint">{{ t.settings.exeDir }}</dt>
            <dd class="selectable font-mono text-[12px] break-all">{{ system.exeDir }}</dd>
          </dl>
          <div class="flex flex-wrap gap-2">
            <button class="btn" @click="tasks.guard(openDataDir)"><FolderOpen :size="15" />{{ t.settings.openFolder }}</button>
            <button class="btn" @click="changeDataDir(false)">{{ t.settings.changeDataDir }}</button>
            <button class="btn btn-ghost" @click="changeDataDir(true)"><RotateCcw :size="15" />{{ t.settings.resetDataDir }}</button>
          </div>
          <div class="flex flex-wrap gap-2 border-t border-line pt-3">
            <button class="btn" @click="makeShortcuts"><Link :size="15" />{{ t.settings.createShortcuts }}</button>
            <button class="btn btn-danger" @click="askUninstall"><Trash2 :size="15" />{{ t.settings.uninstall }}</button>
          </div>
        </section>
        <p v-if="restartNeeded" class="rounded-lg border border-warn/50 bg-warn/10 px-4 py-3 text-warn" role="status">{{ t.settings.restartNotice }}</p>
      </template>

      <section v-else class="card flex items-start gap-4 p-4">
        <img :src="mascot" alt="" class="pixelated size-20" />
        <div class="flex flex-col gap-1">
          <div class="font-mono text-xl font-bold">_k0t1k</div>
          <p class="text-muted">{{ t.settings.aboutText }}</p>
          <dl v-if="system" class="mt-2 grid grid-cols-[110px_1fr] gap-y-1 text-muted">
            <dt class="text-faint">{{ t.settings.version }}</dt><dd>{{ system.appVersion }}</dd>
            <dt class="text-faint">{{ t.settings.system }}</dt><dd>{{ system.os }}</dd>
            <dt class="text-faint">{{ t.settings.dataDir }}</dt><dd class="selectable font-mono text-[12px]">{{ system.dataDir }}</dd>
          </dl>
        </div>
      </section>
    </div>
    <ModalDialog v-if="uninstallPlan" :title="t.settings.uninstallTitle" :width="520" @close="uninstallPlan = null">
      <div class="flex flex-col gap-3">
        <p class="text-muted">{{ t.settings.uninstallIntro }}</p>
        <div class="flex flex-col gap-1">
          <span class="text-faint">{{ t.settings.uninstallFolders }}</span>
          <span v-for="folder in uninstallPlan.folders" :key="folder" class="selectable font-mono text-[12px] break-all">{{ folder }}</span>
        </div>
        <div v-if="uninstallPlan.shortcuts.length" class="flex flex-col gap-1">
          <span class="text-faint">{{ t.settings.uninstallShortcuts }}</span>
          <span v-for="lnk in uninstallPlan.shortcuts" :key="lnk" class="selectable font-mono text-[12px] break-all">{{ lnk }}</span>
        </div>
        <p class="text-[12px] text-faint">{{ t.settings.uninstallNote }}</p>
      </div>
      <template #footer>
        <button class="btn" @click="uninstallPlan = null">{{ t.common.cancel }}</button>
        <button class="btn btn-danger" @click="confirmUninstall">{{ t.settings.uninstall }}</button>
      </template>
    </ModalDialog>
    <AccountLoginDialog v-if="loginKind" :kind="loginKind" @close="loginKind = null" />
  </div>
</template>
