<script setup lang="ts">
import { t } from '../i18n'
import { useInstancesStore } from '../stores/instances'
import type { InstanceActions } from '../utils/useInstanceActions'
import ConfirmDialog from './ConfirmDialog.vue'
import ContextMenu from './ContextMenu.vue'
import ExportDialog from './ExportDialog.vue'
import PackUpdateDialog from './PackUpdateDialog.vue'
import PromptDialog from './PromptDialog.vue'

/** The menus and dialogs opened by the actions of `useInstanceActions`. */
const props = defineProps<{ actions: InstanceActions }>()
const emit = defineEmits<{ deleted: [] }>()
const store = useInstancesStore()
// top-level refs, so the template unwraps them
const { dialog, deleting, exportMenu, exporting, packUpdate, exportItems, submitPrompt } = props.actions

async function confirmDelete() {
  await props.actions.confirmDelete()
  emit('deleted')
}
</script>

<template>
  <ContextMenu v-if="exportMenu" :x="exportMenu.x" :y="exportMenu.y" :items="exportItems" @close="exportMenu = null" />
  <ExportDialog v-if="exporting" :instance="exporting.instance" :format="exporting.format" @close="exporting = null" />
  <PackUpdateDialog v-if="packUpdate?.managedPack" :instance="packUpdate" @close="packUpdate = null" />
  <PromptDialog
    v-if="dialog"
    :title="dialog.kind === 'rename' ? t.library.rename : dialog.kind === 'copy' ? t.library.copy : t.library.changeGroup"
    :label="dialog.kind === 'rename' ? t.library.newName : dialog.kind === 'copy' ? t.library.copyName : t.library.groupName"
    :initial="dialog.kind === 'rename' ? dialog.instance.name : dialog.kind === 'copy' ? `${dialog.instance.name} (${t.library.copySuffix})` : (dialog.instance.group ?? '')"
    :allow-empty="dialog.kind === 'group'"
    :suggestions="dialog.kind === 'group' ? store.groups : []"
    @submit="submitPrompt"
    @close="dialog = null"
  />
  <ConfirmDialog
    v-if="deleting.length"
    :title="deleting.length > 1 ? t.library.deleteMany(deleting.length) : t.library.deleteTitle"
    :text="deleting.length > 1 ? deleting.map((i) => i.name).join(', ') : t.library.deleteText(deleting[0]!.name)"
    @confirm="confirmDelete"
    @close="deleting = []"
  />
</template>
