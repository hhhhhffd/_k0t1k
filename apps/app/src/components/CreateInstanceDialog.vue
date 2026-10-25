<script setup lang="ts">
import { open } from '@tauri-apps/plugin-dialog'
import { FolderInput } from '@lucide/vue'
import curseforgeLogo from '../assets/brands/curseforge.svg'
import ftbLogo from '../assets/brands/ftb.svg'
import modrinthLogo from '../assets/brands/modrinth.svg'
import { computed, ref } from 'vue'
import { fileUrl, isTauri } from '../api/commands'
import type { Instance, LoaderId } from '../api/types'
import { t } from '../i18n'
import { useCustomizationStore } from '../stores/customization'
import { useInstancesStore } from '../stores/instances'
import { useLayoutStore } from '../stores/layout'
import { useTasksStore } from '../stores/tasks'
import { LOADER_LABELS } from '../utils/format'
import InstanceIcon, { INSTANCE_ICONS } from './InstanceIcon.vue'
import LoaderPicker from './LoaderPicker.vue'
import McVersionList from './McVersionList.vue'
import ModalDialog from './ModalDialog.vue'
import PackBrowser, { type PackChoice } from './PackBrowser.vue'

const emit = defineEmits<{ close: [] }>()
const store = useInstancesStore()
const tasks = useTasksStore()
const layout = useLayoutStore()
const customization = useCustomizationStore()

type Page = 'custom' | 'import' | 'modrinth' | 'curseForge' | 'ftb'
/** Each page shows its platform's own logo; the import page a folder like Prism's. */
const pages: { id: Page; logo?: string }[] = [
  { id: 'custom', logo: INSTANCE_ICONS.grass },
  { id: 'import' },
  { id: 'modrinth', logo: modrinthLogo },
  { id: 'curseForge', logo: curseforgeLogo },
  { id: 'ftb', logo: ftbLogo },
]
const page = ref<Page>('custom')

const name = ref('')
const group = ref('')
const icon = ref('grass')
/** A picture from the launcher's icons folder; it is copied into the instance once it exists. */
const libraryIcon = ref<string | null>(null)
const iconsOpen = ref(false)

const mcVersion = ref('')
const loader = ref<LoaderId | null>(null)
const loaderVersion = ref('')

const source = ref('')
const packChoice = ref<PackChoice | null>(null)

// The name field shows what the instance gets called when it is left empty.
const isUrl = (s: string) => /^(https?|curseforge):\/\//i.test(s.trim())
const suggestedName = computed(() => {
  if (page.value === 'custom') return mcVersion.value ? (loader.value ? `${LOADER_LABELS[loader.value]} ${mcVersion.value}` : mcVersion.value) : ''
  if (page.value === 'import') {
    const last = source.value.trim().split(/[\\/?#]/).filter(Boolean).pop() ?? ''
    return decodeURIComponent(last).replace(/\.(mrpack|zip)$/i, '')
  }
  return packChoice.value?.title ?? ''
})
const canCreate = computed(() => {
  if (page.value === 'custom') return mcVersion.value !== '' && (!loader.value || loaderVersion.value !== '')
  if (page.value === 'import') return source.value.trim() !== ''
  return packChoice.value !== null
})

function switchPage(next: Page) {
  page.value = next
  packChoice.value = null
}

async function browseFile() {
  const path = isTauri
    ? await open({ multiple: false, filters: [{ name: t.create.importFilter, extensions: ['mrpack', 'zip'] }] })
    : 'C:\\Downloads\\Example Pack.mrpack'
  if (typeof path === 'string') source.value = path
}

/** Runs in the background after the dialog closes (modpacks can take minutes); a placeholder tile shows meanwhile. */
async function finish(work: Promise<Instance | undefined>) {
  const created = await work
  if (created) layout.selectedId = created.id
}

function create() {
  if (!canCreate.value) return
  const finalName = name.value.trim() || suggestedName.value
  const groupName = group.value.trim()
  let work: Promise<Instance | undefined>
  if (page.value === 'custom') {
    const request = {
      name: finalName,
      group: groupName || undefined,
      icon: icon.value,
      minecraftVersion: mcVersion.value,
      loader: loader.value ? { id: loader.value, version: loaderVersion.value } : undefined,
    }
    const picture = libraryIcon.value
    work = tasks.guard(async () => {
      const created = await store.create(request)
      return picture ? store.setIcon(created.id, picture) : created
    })
  } else if (page.value === 'import') {
    const src = source.value.trim()
    work = tasks.guard(() =>
      isUrl(src) ? store.importModpackUrl(src, finalName || undefined, groupName) : store.importModpack(src, finalName || undefined, groupName),
    )
  } else {
    work = packChoice.value!.install(finalName, groupName)
  }
  emit('close')
  void finish(work)
}
</script>

<template>
  <ModalDialog :title="t.create.title" large @close="emit('close')">
    <div class="flex shrink-0 items-start gap-4 border-b border-line p-4">
      <div class="relative">
        <button class="rounded-lg p-1 hover:bg-hover" :title="t.create.icon" :aria-expanded="iconsOpen" @click="iconsOpen = !iconsOpen">
          <img v-if="libraryIcon" :src="fileUrl(libraryIcon)" alt="" class="size-16 rounded-md object-cover" />
          <InstanceIcon v-else :icon="icon" :size="64" />
        </button>
        <template v-if="iconsOpen">
          <div class="fixed inset-0 z-10" @mousedown="iconsOpen = false" />
          <div class="card anim-pop absolute top-full left-0 z-20 mt-1 grid w-max grid-cols-7 gap-1 p-2 shadow-[var(--shadow-pop)]">
            <button
              v-for="key in Object.keys(INSTANCE_ICONS)"
              :key="key"
              class="rounded-md p-0.5"
              :class="!libraryIcon && icon === key ? 'bg-accent/30 ring-2 ring-accent' : 'hover:bg-hover'"
              :aria-label="key"
              :aria-pressed="!libraryIcon && icon === key"
              @click="((icon = key), (libraryIcon = null), (iconsOpen = false))"
            >
              <InstanceIcon :icon="key" :size="36" />
            </button>
            <button
              v-for="path in customization.iconLibrary"
              :key="path"
              class="rounded-md p-0.5"
              :class="libraryIcon === path ? 'bg-accent/30 ring-2 ring-accent' : 'hover:bg-hover'"
              :aria-label="path"
              :aria-pressed="libraryIcon === path"
              @click="((libraryIcon = path), (iconsOpen = false))"
            >
              <img :src="fileUrl(path)" alt="" class="size-9 rounded object-cover" />
            </button>
          </div>
        </template>
      </div>
      <div class="grid min-w-0 flex-1 grid-cols-[80px_1fr] items-center gap-x-3 gap-y-2">
        <label for="new-name" class="label">{{ t.create.name }}</label>
        <input id="new-name" v-model="name" class="input" :placeholder="suggestedName" autofocus @keydown.enter="create" />
        <label for="new-group" class="label">{{ t.create.group }}</label>
        <input id="new-group" v-model="group" class="input" list="group-options" :placeholder="t.create.groupPlaceholder" />
        <datalist id="group-options"><option v-for="g in store.groups" :key="g" :value="g" /></datalist>
      </div>
    </div>

    <div class="flex min-h-0 flex-1">
      <nav class="flex w-52 shrink-0 flex-col gap-0.5 border-r border-line bg-surface p-1.5" role="tablist" aria-orientation="vertical">
        <button
          v-for="p in pages"
          :key="p.id"
          role="tab"
          :aria-selected="page === p.id"
          class="tool h-9 w-full justify-start"
          :class="page === p.id ? 'tool-active text-accent' : ''"
          @click="switchPage(p.id)"
        >
          <img v-if="p.logo" :src="p.logo" alt="" class="pixelated size-[18px] object-contain" />
          <FolderInput v-else :size="18" class="text-warn" />
          {{ t.create.pages[p.id] }}
        </button>
      </nav>

      <section class="flex min-h-0 min-w-0 flex-1 flex-col gap-3 overflow-y-auto p-4">
        <h3 class="shrink-0 text-[15px] font-semibold">{{ t.create.pages[page] }}</h3>

        <template v-if="page === 'custom'">
          <div class="flex min-h-[250px] flex-[3] flex-col"><McVersionList v-model="mcVersion" /></div>
          <div class="flex min-h-[240px] flex-[2] flex-col border-t border-line pt-3">
            <LoaderPicker v-model:loader="loader" v-model:loader-version="loaderVersion" :mc-version="mcVersion" />
          </div>
        </template>

        <div v-else-if="page === 'import'" class="flex flex-col gap-3">
          <label for="import-source" class="label">{{ t.create.importSource }}</label>
          <div class="flex gap-2">
            <input id="import-source" v-model="source" class="input flex-1 font-mono" placeholder="https://…" @keydown.enter="create" />
            <button class="btn" @click="browseFile">{{ t.iset.browse }}</button>
          </div>
          <div class="text-muted">
            <p class="mb-1">{{ t.create.importSupported }}</p>
            <ul class="list-disc pl-5">
              <li v-for="line in t.create.importFormats" :key="line">{{ line }}</li>
            </ul>
          </div>
        </div>

        <PackBrowser v-else :key="page" v-model="packChoice" :platform="page" />
      </section>
    </div>

    <template #footer>
      <button class="btn" @click="emit('close')">{{ t.common.cancel }}</button>
      <button class="btn btn-primary min-w-24" :disabled="!canCreate" @click="create">{{ t.common.create }}</button>
    </template>
  </ModalDialog>
</template>
