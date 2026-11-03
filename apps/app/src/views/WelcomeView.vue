<script lang="ts">
import { ref } from 'vue'

// Outside the component: the app remounts on a language change, and the welcome must stay on its step.
const step = ref(0)
let languagePreselected = false
</script>

<script setup lang="ts">
import { ArrowLeft, ArrowRight } from '@lucide/vue'
import { onMounted } from 'vue'
import mascot from '../assets/mascot.png'
import AccountSetup from '../components/AccountSetup.vue'
import GetStarted from '../components/GetStarted.vue'
import { LANGUAGES, language, t } from '../i18n'
import { useSettingsStore } from '../stores/settings'
import { useTasksStore } from '../stores/tasks'

/** First run: language, account, first instance. `done` fires when it is finished or skipped. */
const emit = defineEmits<{ done: [] }>()
const settings = useSettingsStore()
const tasks = useTasksStore()

const STEPS = 3
const titles = [() => t.welcome.languageTitle, () => t.welcome.accountTitle, () => t.welcome.instanceTitle]

onMounted(() => {
  if (languagePreselected) return
  languagePreselected = true
  const guess = navigator.language.toLowerCase().startsWith('ru') ? 'ru' : 'en'
  if (guess !== language.value) void tasks.guard(() => settings.setUi({ language: guess }))
})

const next = () => (step.value < STEPS - 1 ? step.value++ : finish())
async function finish() {
  if ((await tasks.guard(() => settings.markOnboarding('welcome').then(() => true))) !== undefined) {
    step.value = 0
    emit('done')
  }
}
</script>

<template>
  <div class="flex min-h-0 flex-1 items-center justify-center overflow-y-auto p-6">
    <div class="anim-fade flex w-full max-w-2xl flex-col gap-5">
      <div class="flex items-center gap-4">
        <img :src="mascot" alt="" class="pixelated anim-bob size-20 shrink-0" />
        <div class="flex flex-col gap-1">
          <h1 class="font-mono text-2xl font-bold tracking-tight">{{ step === 0 ? t.welcome.title : titles[step]() }}</h1>
          <p class="text-muted">{{ [t.welcome.languageHint, t.welcome.accountHint, t.welcome.instanceHint][step] }}</p>
        </div>
      </div>

      <div class="flex items-center gap-2" role="progressbar" :aria-valuenow="step + 1" aria-valuemin="1" :aria-valuemax="STEPS" :aria-label="t.welcome.stepOf(step + 1, STEPS)">
        <span v-for="i in STEPS" :key="i" class="h-1.5 flex-1 rounded-full transition-colors" :class="i <= step + 1 ? 'bg-accent' : 'bg-raised'" />
        <span class="badge ml-1">{{ t.welcome.stepOf(step + 1, STEPS) }}</span>
      </div>

      <section v-if="step === 0" :key="0" class="anim-pop grid gap-3 sm:grid-cols-2">
        <button
          v-for="lang in LANGUAGES"
          :key="lang.id"
          class="card p-6 text-center font-mono text-xl font-bold transition-colors"
          :class="language === lang.id ? 'border-accent bg-accent/15 text-accent' : 'hover:bg-hover'"
          :aria-pressed="language === lang.id"
          @click="tasks.guard(() => settings.setUi({ language: lang.id }))"
        >
          {{ lang.label }}
        </button>
      </section>

      <section v-else-if="step === 1" :key="1" class="anim-pop card flex flex-col gap-3 p-4">
        <p class="text-muted">{{ t.welcome.accountWhy }}</p>
        <AccountSetup @added="next" />
      </section>

      <section v-else :key="2" class="anim-pop flex justify-center">
        <GetStarted @chosen="finish" />
      </section>

      <div class="flex items-center justify-between gap-3">
        <button class="btn btn-ghost" @click="finish">{{ t.welcome.skip }}</button>
        <div class="flex gap-2">
          <button v-if="step > 0" class="btn" @click="step--"><ArrowLeft :size="15" />{{ t.common.back }}</button>
          <button v-if="step < STEPS - 1" class="btn btn-primary" @click="next">
            {{ step === 1 ? t.welcome.later : t.welcome.next }}<ArrowRight :size="15" />
          </button>
          <button v-else class="btn" @click="finish">{{ t.welcome.finish }}</button>
        </div>
      </div>
    </div>
  </div>
</template>
