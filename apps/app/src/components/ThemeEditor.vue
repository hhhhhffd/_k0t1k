<script setup lang="ts">
import { Copy, Download, Plus, Trash2, Upload } from '@lucide/vue'
import { computed, ref } from 'vue'
import type { CustomTheme, ThemeId } from '../api/types'
import { t } from '../i18n'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'
import { errorMessage } from '../utils/format'
import { builtinColors, isBuiltinTheme, isColorValue, parseThemeJson, THEME_COLORS, THEMES, toHex } from '../utils/theme'
import ConfirmDialog from './ConfirmDialog.vue'
import SelectMenu from './SelectMenu.vue'

const store = useSettingsStore()
const tasks = useTasksStore()

const baseOptions = THEMES.map((th) => ({ value: th.id, label: th.label }))
const newBase = ref<ThemeId>('k0t1k')
const panel = ref<'import' | 'export' | null>(null)
const text = ref('')
const confirmDelete = ref(false)

const themes = computed(() => store.settings?.ui.customThemes ?? [])
const current = computed(() => themes.value.find((c) => c.id === store.settings?.ui.theme))
const currentColors = computed(() => (current.value ? { ...builtinColors(current.value.base), ...current.value.colors } : {}))

function nextName(base: string): string {
  const names = new Set(themes.value.map((c) => c.name))
  for (let n = 1; ; n++) {
    const name = n === 1 ? base : `${base} ${n}`
    if (!names.has(name)) return name
  }
}

async function add(theme: Omit<CustomTheme, 'id'>) {
  const id = `custom-${Date.now().toString(36)}`
  const list = [...themes.value, { ...theme, id }]
  await tasks.guard(() => store.setUi({ customThemes: list, theme: id }))
  panel.value = null
}

const create = () => add({ name: nextName(t.themes.newName), base: newBase.value, colors: builtinColors(newBase.value) })

function duplicate() {
  const src = current.value
  if (src) void add({ name: nextName(src.name), base: src.base, colors: { ...currentColors.value } })
}

async function remove() {
  confirmDelete.value = false
  const id = current.value?.id
  await tasks.guard(() => store.setUi({ customThemes: themes.value.filter((c) => c.id !== id), theme: 'k0t1k' }))
}

function setColor(key: string, value: string) {
  const theme = current.value
  if (!theme || !isColorValue(value)) return
  theme.colors = { ...currentColors.value, [key]: value }
  store.reapply()
}

function setBase(base: string) {
  const theme = current.value
  if (!theme || !isBuiltinTheme(base)) return
  theme.base = base
  store.reapply()
}

async function exportTheme() {
  const theme = current.value
  if (!theme) return
  text.value = JSON.stringify({ name: theme.name, base: theme.base, colors: currentColors.value }, null, 2)
  panel.value = 'export'
  try {
    await navigator.clipboard.writeText(text.value)
    tasks.toast(t.themes.copied, 'ok')
  } catch {
    // no clipboard access: the JSON stays visible for manual copying
  }
}

function openImport() {
  text.value = ''
  panel.value = 'import'
}

function importTheme() {
  try {
    const parsed = parseThemeJson(text.value)
    void add({ ...parsed, name: nextName(parsed.name) })
  } catch (e) {
    tasks.toast(errorMessage(e), 'error')
  }
}
</script>

<template>
  <section class="card flex flex-col gap-4 p-4">
    <h3 class="section-title">{{ t.themes.title }}</h3>

    <div class="flex flex-wrap items-end gap-2">
      <div class="flex w-44 flex-col gap-1">
        <span class="label">{{ t.themes.base }}</span>
        <SelectMenu v-model="newBase" :options="baseOptions" :aria-label="t.themes.base" />
      </div>
      <button class="btn btn-primary" @click="create"><Plus :size="15" />{{ t.themes.create }}</button>
      <button class="btn" @click="openImport"><Upload :size="15" />{{ t.themes.import }}</button>
    </div>

    <div v-if="panel" class="flex flex-col gap-2">
      <span class="label">{{ panel === 'import' ? t.themes.importHint : t.themes.exportHint }}</span>
      <textarea v-model="text" class="input selectable font-mono text-[12px]" rows="9" :readonly="panel === 'export'" :aria-label="t.themes.json" spellcheck="false" />
      <div class="flex justify-end gap-2">
        <button class="btn" @click="panel = null">{{ t.common.close }}</button>
        <button v-if="panel === 'import'" class="btn btn-primary" :disabled="!text.trim()" @click="importTheme">{{ t.themes.import }}</button>
        <button v-else class="btn" @click="exportTheme"><Copy :size="15" />{{ t.common.copy }}</button>
      </div>
    </div>

    <div v-if="current" class="flex flex-col gap-3 border-t border-line pt-4">
      <div class="flex flex-wrap items-end gap-2">
        <label class="flex min-w-48 flex-1 flex-col gap-1">
          <span class="label">{{ t.themes.name }}</span>
          <input v-model="current.name" class="input" maxlength="32" />
        </label>
        <div class="flex w-44 flex-col gap-1">
          <span class="label">{{ t.themes.base }}</span>
          <SelectMenu :model-value="current.base" :options="baseOptions" :aria-label="t.themes.base" @update:model-value="setBase" />
        </div>
      </div>

      <ul class="grid grid-cols-2 gap-x-4 gap-y-2">
        <li v-for="c in THEME_COLORS" :key="c.key" class="flex items-center gap-2">
          <input
            type="color"
            class="h-8 w-10 shrink-0 cursor-pointer rounded-md border border-line bg-transparent p-0.5"
            :value="toHex(currentColors[c.key] ?? '')"
            :aria-label="c.label"
            @input="setColor(c.key, ($event.target as HTMLInputElement).value)"
          />
          <div class="min-w-0 flex-1">
            <span class="label block truncate">{{ c.label }}</span>
            <input
              :value="currentColors[c.key]"
              class="input h-7 font-mono text-[12px]"
              :class="isColorValue(currentColors[c.key] ?? '') ? '' : '!border-danger'"
              :aria-label="c.label"
              @change="setColor(c.key, ($event.target as HTMLInputElement).value.trim())"
            />
          </div>
        </li>
      </ul>

      <div class="flex flex-wrap justify-end gap-2">
        <button class="btn" @click="duplicate"><Copy :size="15" />{{ t.themes.duplicate }}</button>
        <button class="btn" @click="exportTheme"><Download :size="15" />{{ t.themes.export }}</button>
        <button class="btn btn-danger" @click="confirmDelete = true"><Trash2 :size="15" />{{ t.common.delete }}</button>
      </div>
    </div>
    <p v-else class="text-[12px] text-faint">{{ t.themes.hint }}</p>

    <ConfirmDialog
      v-if="confirmDelete && current"
      :title="t.themes.deleteTitle"
      :text="t.themes.deleteText(current.name)"
      @confirm="remove"
      @close="confirmDelete = false"
    />
  </section>
</template>
