<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import type { JarScan, ScanResult } from '../api/types'
import { t } from '../i18n'
import { badgeOf, useRatscanStore } from '../stores/ratscan'
import { useTasksStore } from '../stores/tasks'
import ModalDialog from './ModalDialog.vue'

const props = defineProps<{ instanceId: string; jar: JarScan }>()
const emit = defineEmits<{ close: [] }>()
const ratscan = useRatscanStore()
const tasks = useTasksStore()
const result = ref<ScanResult | null>(null)
const loading = ref(true)

onMounted(async () => {
  result.value = (await tasks.guard(() => ratscan.report(props.instanceId, props.jar.sha1))) ?? null
  loading.value = false
})

const badge = computed(() => (props.jar.scan ? badgeOf(props.jar.scan.verdict) : 'unknown'))
const badgeClass = { clean: 'badge-ok', suspicious: 'badge-warn', dangerous: 'badge-danger', unknown: '' } as const
const severityClass = { low: '', medium: 'badge-warn', high: 'badge-danger', critical: 'badge-danger' } as const
const where = (e: { archive_path?: string; class_name?: string; method_name?: string; resource_path?: string }) =>
  [e.archive_path, e.class_name ?? e.resource_path, e.method_name].filter(Boolean).join(' › ')
</script>

<template>
  <ModalDialog :title="jar.fileName" :width="720" @close="emit('close')">
    <div class="flex flex-col gap-3">
      <div class="flex items-center gap-2">
        <span class="badge" :class="badgeClass[badge]">{{ t.scan[badge] }}</span>
        <span v-if="jar.scan?.riskScore !== null && jar.scan" class="text-muted">{{ t.scan.risk(jar.scan.riskScore ?? 0) }}</span>
      </div>
      <p v-if="jar.scan" class="selectable text-muted">{{ jar.scan.summary }}</p>
      <p v-if="badge === 'unknown'" class="text-warn">{{ t.scan.unknownHint }}</p>
      <p v-if="loading" class="text-faint">{{ t.common.loading }}</p>
      <template v-else-if="result">
        <p v-if="!result.report.findings.length" class="text-muted">{{ t.scan.noFindings }}</p>
        <ul v-else class="flex flex-col gap-2">
          <li v-for="(finding, i) in result.report.findings" :key="i" class="rounded-lg border border-line bg-raised p-3">
            <div class="flex items-center gap-2">
              <span class="badge" :class="severityClass[finding.severity]">{{ finding.severity }}</span>
              <span class="font-semibold">{{ finding.category }}</span>
              <span class="font-mono text-[11px] text-faint">{{ finding.rule_id }}</span>
            </div>
            <p v-if="finding.message" class="selectable mt-1 text-muted">{{ finding.message }}</p>
            <ul v-if="finding.evidence?.length" class="selectable mt-1 font-mono text-[11px] break-all text-faint">
              <li v-for="(evidence, j) in finding.evidence.slice(0, 4)" :key="j">{{ where(evidence) }}<template v-if="evidence.value_preview"> · {{ evidence.value_preview }}</template></li>
            </ul>
          </li>
        </ul>
        <div v-if="result.report.limitations?.length" class="text-[12px] text-warn">
          <div class="font-semibold">{{ t.scan.limitations }}</div>
          <ul class="list-disc pl-5"><li v-for="(limit, i) in result.report.limitations" :key="i">{{ limit.message }}</li></ul>
        </div>
      </template>
      <p class="text-[11px] text-faint">{{ t.scan.disclaimer }}</p>
    </div>
    <template #footer><button class="btn btn-primary" @click="emit('close')">{{ t.common.close }}</button></template>
  </ModalDialog>
</template>
