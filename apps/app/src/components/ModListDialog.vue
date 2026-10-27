<script setup lang="ts">
import { Copy, Save } from '@lucide/vue'
import { save } from '@tauri-apps/plugin-dialog'
import { computed, reactive, ref, watch } from 'vue'
import * as api from '../api/commands'
import { isTauri } from '../api/commands'
import type { LocalResource } from '../api/types'
import { t } from '../i18n'
import { useTasksStore } from '../stores/tasks'
import { openExternal } from '../utils/external'
import { originProject, plainText } from '../utils/format'
import CheckBox from './CheckBox.vue'
import ModalDialog from './ModalDialog.vue'
import SelectMenu from './SelectMenu.vue'

type Format = 'html' | 'markdown' | 'text' | 'json' | 'csv' | 'custom'
type Field = 'version' | 'authors' | 'url' | 'filename'
interface Row {
  name: string
  version: string
  authors: string
  url: string
  filename: string
}
interface Saved {
  format: Format
  fields: Record<Field, boolean>
  includeDisabled: boolean
  template: string
}

const props = defineProps<{ resources: LocalResource[]; title: string; fileName: string }>()
const emit = defineEmits<{ close: [] }>()
const tasks = useTasksStore()

// The choices are remembered for the next export (per viewer; a failure to read them just means defaults).
const STORAGE_KEY = 'k0t1k.modListExport'
function restore(): Saved | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    return raw ? (JSON.parse(raw) as Saved) : null
  } catch {
    return null
  }
}
const saved = restore()
const format = ref<Format>(saved?.format ?? 'markdown')
const fields = reactive<Record<Field, boolean>>({ version: true, authors: false, url: true, filename: false, ...saved?.fields })
const includeDisabled = ref(saved?.includeDisabled ?? false)
const customTemplate = ref(saved?.template ?? '{name} [{version}]')
watch([format, fields, includeDisabled, customTemplate], () => {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ format: format.value, fields, includeDisabled: includeDisabled.value, template: customTemplate.value }))
  } catch {
    // storage unavailable: the choices are simply not remembered
  }
})

const formatOptions = (Object.keys(t.modList.formats) as Format[]).map((value) => ({ value, label: t.modList.formats[value] }))
const fieldList: Field[] = ['version', 'authors', 'url', 'filename']
const extensions: Record<Format, string> = { html: 'html', markdown: 'md', text: 'txt', json: 'json', csv: 'csv', custom: 'txt' }

const rows = computed<Row[]>(() =>
  props.resources
    .filter((r) => includeDisabled.value || r.enabled)
    .map((r) => ({
      name: plainText(r.metadata?.name ?? '') || r.fileName,
      version: r.metadata?.version ?? '',
      authors: r.metadata?.authors.join(', ') ?? '',
      url: r.origin ? (originProject(r.origin)?.url ?? '') : '',
      filename: r.fileName,
    }))
    .sort((a, b) => a.name.localeCompare(b.name)),
)

/** One line per file in the built-in text formats; a part whose field is off or empty is left out. */
function line(row: Row, f: Format): string {
  const link = fields.url && row.url
  const version = fields.version && row.version ? ` [${row.version}]` : ''
  const authors = fields.authors && row.authors ? ` ${t.modList.by} ${row.authors}` : ''
  const file = fields.filename ? ` (${row.filename})` : ''
  if (f === 'html') {
    const name = link ? `<a href="${html(row.url)}">${html(row.name)}</a>` : html(row.name)
    return `<li>${name}${html(version + authors + file)}</li>`
  }
  if (f === 'markdown') return `- ${link ? `[${row.name}](${row.url})` : row.name}${version}${authors}${file}`
  return `${row.name}${version}${authors}${file}${link ? ` ${row.url}` : ''}`
}

/** The pattern behind the current format, shown in the template box; editing it switches to the custom format. */
const template = computed({
  get: () => {
    if (format.value === 'custom') return customTemplate.value
    const sample: Row = { name: '{name}', version: '{version}', authors: '{authors}', url: '{url}', filename: '{filename}' }
    if (format.value === 'json' || format.value === 'csv') return ['name', ...fieldList.filter((f) => fields[f])].map((f) => `{${f}}`).join(format.value === 'csv' ? ',' : ', ')
    return line(sample, format.value).replace(/&lt;|&gt;|&quot;|&amp;/g, (m) => ({ '&lt;': '<', '&gt;': '>', '&quot;': '"', '&amp;': '&' })[m]!)
  },
  set: (value) => {
    customTemplate.value = value
    format.value = 'custom'
  },
})

const html = (value: string) => value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
const csvCell = (value: string) => (/[",\n]/.test(value) ? `"${value.replace(/"/g, '""')}"` : value)

const output = computed(() => {
  const list = rows.value
  const chosen = fieldList.filter((f) => fields[f])
  switch (format.value) {
    case 'html':
      return `<html><body><ul>\n${list.map((r) => `  ${line(r, 'html')}`).join('\n')}\n</ul></body></html>`
    case 'markdown':
    case 'text':
      return list.map((r) => line(r, format.value)).join('\n')
    case 'json':
      return JSON.stringify(list.map((r) => Object.fromEntries([['name', r.name], ...chosen.map((f) => [f, r[f]])])), null, 2)
    case 'csv':
      return [['name', ...chosen], ...list.map((r) => [r.name, ...chosen.map((f) => r[f])])].map((row) => row.map(csvCell).join(',')).join('\n')
    case 'custom':
      return list.map((r) => customTemplate.value.replace(/\{(name|version|authors|url|filename)\}/g, (_, key: keyof Row) => r[key])).join('\n')
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
  const done = await tasks.guard(async () => {
    await api.saveTextFile(path, output.value)
    return true
  })
  if (done) tasks.toast(t.modList.saved(path), 'ok')
}
</script>

<template>
  <ModalDialog :title="title" :width="1040" @close="emit('close')">
    <div class="flex flex-col gap-4">
      <section class="flex gap-4">
        <label class="flex min-w-0 flex-1 flex-col gap-1">
          <span class="label">{{ t.modList.template }}</span>
          <textarea v-model="template" class="input h-28 resize-none font-mono text-[12px]" spellcheck="false" />
          <span class="text-[12px] text-faint">{{ t.modList.templateHint }}</span>
        </label>
        <div class="flex w-56 shrink-0 flex-col gap-1.5">
          <span class="label">{{ t.modList.format }}</span>
          <SelectMenu v-model="format" :options="formatOptions" :aria-label="t.modList.format" />
          <span class="label mt-2">{{ t.modList.fieldsLabel }}</span>
          <CheckBox v-for="f in fieldList" :key="f" v-model="fields[f]" :label="t.modList.fields[f]" />
          <div class="my-1 border-t border-line" />
          <CheckBox v-model="includeDisabled" :label="t.modList.includeDisabled" />
        </div>
      </section>

      <section class="flex flex-col gap-1">
        <span class="label">{{ t.modList.result }} · {{ rows.length }}</span>
        <div class="grid h-80 grid-cols-2 gap-3">
          <textarea class="input selectable h-full resize-none font-mono text-[12px]" readonly :value="output" :aria-label="t.modList.result" />
          <div class="selectable h-full overflow-y-auto rounded-lg border border-line bg-base px-3 py-2" :aria-label="t.modList.preview">
            <ul class="list-disc pl-4 text-[13px]">
              <li v-for="r in rows" :key="r.filename">
                <a v-if="fields.url && r.url" :href="r.url" class="text-accent hover:underline" @click.prevent="tasks.guard(() => openExternal(r.url))">{{ r.name }}</a>
                <template v-else>{{ r.name }}</template>
                <span v-if="fields.version && r.version" class="text-muted"> [{{ r.version }}]</span>
                <span v-if="fields.authors && r.authors" class="text-muted"> {{ t.modList.by }} {{ r.authors }}</span>
                <span v-if="fields.filename" class="text-faint"> ({{ r.filename }})</span>
              </li>
            </ul>
          </div>
        </div>
      </section>
    </div>
    <template #footer>
      <span class="mr-auto self-center text-[12px] text-faint">{{ t.modList.metadataNote }}</span>
      <button class="btn" :disabled="!rows.length" @click="copy"><Copy :size="15" />{{ t.common.copy }}</button>
      <button class="btn btn-primary" :disabled="!rows.length" @click="saveToFile"><Save :size="15" />{{ t.modList.save }}</button>
    </template>
  </ModalDialog>
</template>
