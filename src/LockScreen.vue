<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElButton } from 'element-plus'
import { languages, locale, localizeMessage, setLocale, t } from './i18n.js'

const props = defineProps({
  status: { type: Object, default: null },
  startupError: { type: String, default: '' },
  checking: Boolean,
  closeBlocked: Boolean,
})
const emit = defineEmits(['unlocked', 'retry'])
const dialog = ref(null)
const password = ref('')
const showPassword = ref(false)
const recovering = ref(false)
const recoveryCode = ref('')
const newPassword = ref('')
const confirmation = ref('')
const result = ref(null)
const saved = ref(false)
const busy = ref(false)
const error = ref('')
const copyMessage = ref('')
const retrySeconds = ref(props.status?.retry_after_seconds ?? 0)
const coolingDown = computed(() => retrySeconds.value > 0)
let retryDeadline = Date.now() + retrySeconds.value * 1000
let ticker
let idleTimer
const activityEvents = ['pointerdown', 'keydown', 'wheel', 'touchstart']

function tick() {
  retrySeconds.value = Math.max(0, Math.ceil((retryDeadline - Date.now()) / 1000))
}

function focusField() {
  dialog.value?.querySelector('[data-lock-focus]')?.focus()
}

function scheduleRecoveryLock() {
  clearTimeout(idleTimer)
  const minutes = result.value?.status.idle_timeout_minutes
  if (minutes > 0) idleTimer = setTimeout(lockRecovery, minutes * 60_000)
}

function clearRecovery() {
  clearTimeout(idleTimer)
  result.value = null
  saved.value = false
  copyMessage.value = ''
  recovering.value = false
}

async function lockRecovery() {
  if (!result.value) return
  clearRecovery()
  busy.value = true
  error.value = ''
  try {
    await invoke('lock_app')
  } catch (failure) {
    error.value = failure?.message ?? String(failure)
  } finally {
    busy.value = false
    await nextTick()
    focusField()
  }
}

function recoveryLockShortcut(event) {
  if (!result.value || event.repeat || event.isComposing) return
  if ((event.ctrlKey || event.metaKey) && event.shiftKey && !event.altKey && event.code === 'KeyL') {
    event.preventDefault()
    void lockRecovery()
  }
}

onMounted(() => {
  dialog.value?.showModal()
  focusField()
  ticker = setInterval(tick, 1000)
  for (const event of activityEvents) window.addEventListener(event, scheduleRecoveryLock, { capture: true, passive: true })
  window.addEventListener('keydown', recoveryLockShortcut, { capture: true })
})

onBeforeUnmount(() => {
  clearInterval(ticker)
  clearTimeout(idleTimer)
  for (const event of activityEvents) window.removeEventListener(event, scheduleRecoveryLock, { capture: true })
  window.removeEventListener('keydown', recoveryLockShortcut, { capture: true })
  dialog.value?.close()
})

async function switchForm() {
  recovering.value = !recovering.value
  password.value = ''
  newPassword.value = ''
  confirmation.value = ''
  recoveryCode.value = ''
  showPassword.value = false
  error.value = ''
  await nextTick()
  focusField()
}

async function submit() {
  if (busy.value || coolingDown.value || props.status?.corrupted || result.value) return
  if (!props.status?.locked) return
  if (recovering.value && newPassword.value !== confirmation.value) {
    error.value = '两次输入的密码不一致'
    return
  }
  busy.value = true
  error.value = ''
  try {
    if (recovering.value) {
      result.value = await invoke('recover_app_lock', {
        recoveryCode: recoveryCode.value,
        newPassword: newPassword.value,
      })
      scheduleRecoveryLock()
      await nextTick()
      focusField()
    } else {
      emit('unlocked', await invoke('unlock_app', { password: password.value }))
    }
  } catch (failure) {
    error.value = failure?.message ?? String(failure)
    try {
      const status = await invoke('app_lock_status')
      retryDeadline = Date.now() + status.retry_after_seconds * 1000
      tick()
      if (coolingDown.value) error.value = ''
    } catch {
      // Keep the authentication error visible when the status query fails.
    }
  } finally {
    password.value = ''
    recoveryCode.value = ''
    newPassword.value = ''
    confirmation.value = ''
    busy.value = false
  }
}

async function copyRecoveryCode() {
  const recovery = result.value
  if (!recovery) return
  try {
    await navigator.clipboard.writeText(recovery.recovery_code)
    if (result.value === recovery) copyMessage.value = '已复制恢复码'
  } catch {
    if (result.value === recovery) copyMessage.value = '复制失败，请手动抄录恢复码。'
  }
}

async function confirmSaved() {
  const recovery = result.value
  if (!recovery || !saved.value || busy.value) return
  busy.value = true
  error.value = ''
  let latestStatus
  try {
    latestStatus = await invoke('app_lock_status')
  } catch (failure) {
    if (result.value === recovery) error.value = failure?.message ?? String(failure)
    return
  } finally {
    if (result.value === recovery) busy.value = false
  }
  if (result.value !== recovery) return
  clearRecovery()
  if (latestStatus.locked === false) {
    emit('unlocked', latestStatus)
  } else {
    retryDeadline = Date.now() + latestStatus.retry_after_seconds * 1000
    tick()
    await nextTick()
    focusField()
  }
}
</script>

<template>
  <dialog ref="dialog" class="app-lock-screen" aria-labelledby="lock-screen-title" @cancel.prevent @keydown.esc.stop>
    <select
      class="lock-language"
      :value="locale"
      :aria-label="t('界面语言')"
      @change="setLocale($event.target.value)"
    >
      <option v-for="language in languages" :key="language.id" :value="language.id">{{ language.name }}</option>
    </select>
    <div class="lock-panel">
      <div class="brand">
        <span class="brand-icon">班</span>
        <span>班伴<small>CLASSMATE HUB</small></span>
      </div>
      <p v-if="closeBlocked" class="lock-error" role="status">{{ t('当前存在未保存的修改，请先解锁后再关闭窗口。') }}</p>

      <template v-if="!status">
        <h1 id="lock-screen-title">{{ t('无法读取应用锁状态') }}</h1>
        <p v-if="startupError" class="lock-error" role="alert">{{ localizeMessage(startupError) }}</p>
        <ElButton type="primary" :loading="checking" @click="emit('retry')">{{ t('重试') }}</ElButton>
      </template>
      <template v-else-if="status.corrupted">
        <h1 id="lock-screen-title">{{ t('应用已锁定') }}</h1>
        <p class="lock-error" role="alert">{{ t('应用锁配置损坏。请检查或移除应用数据目录中的 app-lock.json，再重新启动。') }}</p>
      </template>
      <template v-else-if="result">
        <h1 id="lock-screen-title">{{ t('保存恢复码') }}</h1>
        <p>{{ t('恢复码仅在此处显示，可用一次来重置密码。请妥善保存。') }}</p>
        <code class="lock-code">{{ result.recovery_code }}</code>
        <div class="lock-actions">
          <ElButton @click="copyRecoveryCode">{{ t('复制恢复码') }}</ElButton>
          <span aria-live="polite">{{ t(copyMessage) }}</span>
        </div>
        <label class="lock-checkbox">
          <input v-model="saved" type="checkbox" data-lock-focus>
          {{ t('我已妥善保存恢复码') }}
        </label>
        <p v-if="error" class="lock-error" role="alert">{{ localizeMessage(error) }}</p>
        <ElButton type="primary" :loading="busy" :disabled="!saved" @click="confirmSaved">{{ t('进入工作区') }}</ElButton>
      </template>
      <template v-else>
        <h1 id="lock-screen-title">{{ t(recovering ? '使用恢复码重置密码' : '应用已锁定') }}</h1>
        <form class="lock-form" @submit.prevent="submit">
          <template v-if="recovering">
            <label>
              {{ t('恢复码') }}
              <input v-model="recoveryCode" required autocomplete="off" spellcheck="false" data-lock-focus>
            </label>
            <label>
              {{ t('新密码') }}
              <input v-model="newPassword" required type="password" autocomplete="new-password">
            </label>
            <label>
              {{ t('确认新密码') }}
              <input v-model="confirmation" required type="password" autocomplete="new-password">
            </label>
          </template>
          <div v-else class="lock-password-row">
            <label>
              {{ t('应用密码') }}
              <input
                v-model="password"
                :type="showPassword ? 'text' : 'password'"
                required
                autocomplete="current-password"
                spellcheck="false"
                data-lock-focus
              >
            </label>
            <ElButton :aria-pressed="showPassword" @click="showPassword = !showPassword">
              {{ t(showPassword ? '隐藏密码' : '显示密码') }}
            </ElButton>
          </div>
          <p v-if="coolingDown || error" class="lock-error" role="status">
            {{ coolingDown ? t('密码错误次数过多，请在 {0} 秒后重试', retrySeconds) : localizeMessage(error) }}
          </p>
          <ElButton type="primary" native-type="submit" :loading="busy" :disabled="coolingDown">
            {{ t(recovering ? '重置密码' : '解锁') }}
          </ElButton>
          <ElButton link type="primary" :disabled="busy" @click="switchForm">
            {{ t(recovering ? '返回密码解锁' : '使用恢复码重置密码') }}
          </ElButton>
        </form>
      </template>
    </div>
  </dialog>
</template>
