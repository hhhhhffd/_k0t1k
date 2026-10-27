<script setup lang="ts">
import { Copy, Save } from '@lucide/vue'
import { save } from '@tauri-apps/plugin-dialog'
import { computed, ref } from 'vue'
import * as api from '../api/commands'
import { isTauri } from '../api/commands'
import type { LocalResource } from '../api/types'
import { t } from '../i18n'
import { useTasksStore } from '../stores/tasks'
import { originProject } from '../utils/format'
import CheckBox from './CheckBox.vue'
import ModalDialog from './ModalDialog.vue'
import SelectMenu from './SelectMenu.vue'

type Format = 'markdown' | 'text' | 'csv' | 'json' | 'html'

const props = defineProps<{ resources: LocalResource[]; title: string; fileName: string }>()
const emit = defineEmits<{ close: [] }>()
const tasks = useTasksStore()

const format = ref<Format>('markdown')
const includeDisabled = ref(false)
const formatOptions = (Object.keys(t.modList.formats) as Format[]).map((value) => ({ value, label: t.modList.formats[value] }))
const extensions: Record<Format, string> = { markdown: 'md', text: 'txt', csv: 'csv', json: 'json', html: 'html' }

const rows = computed(() =>
  props.resources
    .filter((r) => includeDisabled.value || r.enabled)
    .map((r) => ({
      name: r.metadata?.name ?? r.fileName,
      version: r.metadata?.version ?? '',
      file: r.fileName,
      authors: r.metadata?.authors.join(', ') ?? '',
      url: r.origin ? (originProject(r.origin)?.url ?? '') : '',
    }))
    .sort((a, b) => a.name.localeCompare(b.name)),
)

const csvCell = (value: string) => (/[",\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value)
const html = (value: string) => value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
const withVersion = (row: { name: string; version: string }) => (row.version ? `${row.name} ${row.version}` : row.name)

const output = computed(() => {
  const list = rows.value
  const c = t.modList.columns
  switch (format.value) {
    case 'markdown':
      return list.map((r) => `- ${r.url ? `[${r.name}](${r.url})` : r.name}${r.version ? ` ${r.version}` : ''}`).join('\n')
    case 'text':
      return list.map(withVersion).join('\n')
    case 'csv':
      return [[c.name, c.version, c.file, c.authors, c.url], ...list.map((r) => [r.name, r.version, r.file, r.authors, r.url])]
        .map((row) => row.map(csvCell).join(','))
        .join('\n')
    case 'json':
      return JSON.stringify(list, null, 2)
    case 'html':
      return `<ul>\n${list.map((r) => `  <li>${r.url ? `<a href="${html(r.url)}">${html(r.name)}</a>` : html(r.name)}${r.version ? ` ${html(r.version)}` : ''}</li>`).join('\n')}\n</ul>`
  }
  return ''
})

async function copy() {
  try {
    await navigator.clipboard.writeText(output.value)
    tasks.toast(t.modList.copied, 'ok')
  } catch (e) {
    tasks.toast(String(e), 'error')
  }
}

async function saveToFile() {
  const extension = extensions[format.value]
  const path = isTauri
    ? await save({ defaultPath: `${props.fileName}.${extension}`, filters: [{ name: t.modList.formats[format.value], extensions: [extension] }] })
    : `C:\\Downloads\\${props.fileName}.${extension}`
  if (!path) return
  const saved = await tasks.guard(async () => {
    await api.saveTextFile(path, output.value)
    return true
  })
  if (saved) tasks.toast(t.modList.saved(path), 'ok')
}
</script>

<template>
  <ModalDialog :title="title" :width="640" @close="emit('close')">
    <div class="flex flex-col gap-3">
      <div class="flex items-center gap-3">
        <span class="label">{{ t.modList.format }}</span>
        <SelectMenu v-model="format" class="w-40" :options="formatOptions" :aria-label="t.modList.format" />
        <CheckBox v-model="includeDisabled" :label="t.modList.includeDisabled" />
        <span class="ml-auto text-faint">{{ rows.length }}</span>
      </div>
      <textarea class="input selectable h-72 resize-none py-2 font-mono text-[12px]" readonly :value="output" :aria-label="title" />
    </div>
    <template #footer>
      <button class="btn" :disabled="!rows.length" @click="copy"><Copy :size="15" />{{ t.common.copy }}</button>
      <button class="btn btn-primary" :disabled="!rows.length" @click="saveToFile"><Save :size="15" />{{ t.modList.save }}</button>
    </template>
  </ModalDialog>
</template>
