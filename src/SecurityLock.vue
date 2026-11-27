<script setup>
import { computed, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElButton, ElDialog, ElTag } from 'element-plus'
import { localizeMessage, t } from './i18n.js'

const props = defineProps({
  status: { type: Object, required: true },
  desktop: Boolean,
  locking: Boolean,
})
const emit = defineEmits(['status-change', 'lock'])
const timeoutOptions = [0, 5, 10, 15, 30]
const setupTimeout = ref(0)
const currentPassword = ref('')
const newPassword = ref('')
const confirmation = ref('')
const action = ref(null)
const busy = ref(false)
const error = ref('')
const feedback = ref('')
const recoveryCode = ref('')
const saved = ref(false)
const copyMessage = ref('')

const actions = {
  change: { title: '修改密码', command: 'change_app_lock_password' },
  regenerate: { title: '重新生成恢复码', command: 'regenerate_recovery_code' },
  disable: { title: '关闭应用锁', command: 'disable_app_lock' },
}
const actionDialog = computed({
  get: () => action.value !== null,
  set(open) { if (!open) closeAction() },
})

function hasUnsavedChanges() {
  return !!(newPassword.value || confirmation.value || currentPassword.value ||
    recoveryCode.value || (!props.status.enabled && setupTimeout.value))
}

defineExpose({ isBusy: () => busy.value, hasUnsavedChanges })

function closeAction() {
  if (busy.value) return
  action.value = null
  error.value = ''
  clearPasswords()
}

function clearPasswords() {
  currentPassword.value = ''
  newPassword.value = ''
  confirmation.value = ''
}

function openAction(name) {
  error.value = ''
  feedback.value = ''
  action.value = name
}

async function perform(command, args) {
  if (busy.value || props.locking) return
  busy.value = true
  error.value = ''
  feedback.value = ''
  try {
    const response = await invoke(command, args)
    if (response.recovery_code) {
      recoveryCode.value = response.recovery_code
      saved.value = false
      copyMessage.value = ''
    }
    emit('status-change', response.status ?? response)
    return true
  } catch (failure) {
    error.value = failure?.message ?? String(failure)
  } finally {
    busy.value = false
  }
}

function matchesConfirmation() {
  if (newPassword.value === confirmation.value) return true
  error.value = '两次输入的密码不一致'
  return false
}

async function setup() {
  if (!props.desktop || !matchesConfirmation()) return
  const completed = await perform('setup_app_lock', {
    password: newPassword.value,
    idleTimeoutMinutes: setupTimeout.value,
  })
  if (completed) {
    clearPasswords()
    setupTimeout.value = 0
  }
}

async function updateTimeout(event) {
  const select = event.target
  const completed = await perform('update_app_lock_settings', { idleTimeoutMinutes: Number(select.value) })
  if (!completed) select.value = String(props.status.idle_timeout_minutes)
}

async function submitAction() {
  const selected = action.value
  if (!selected || (selected === 'change' && !matchesConfirmation())) return
  const args = { currentPassword: currentPassword.value }
  if (selected === 'change') args.newPassword = newPassword.value
  const completed = await perform(actions[selected].command, args)
  if (completed) {
    closeAction()
    if (selected === 'change') feedback.value = '应用密码已修改'
  }
}

async function copyRecoveryCode() {
  try {
    await navigator.clipboard.writeText(recoveryCode.value)
    copyMessage.value = '已复制恢复码'
  } catch {
    copyMessage.value = '复制失败，请手动抄录恢复码。'
  }
}

function confirmSaved() {
  if (!saved.value) return
  recoveryCode.value = ''
  saved.value = false
}
</script>

<template>
  <section class="content">
    <div class="lock-settings page-card">
      <div class="lock-actions">
        <h2>{{ t('应用锁') }}</h2>
        <ElTag :type="status.enabled ? 'success' : 'info'" effect="light">
          {{ t(status.enabled ? '已开启' : '未开启') }}
        </ElTag>
      </div>
      <p>{{ t('应用锁限制他人通过班伴查看或修改数据，数据库文件仍以原有格式存储，未加密。') }}</p>
      <p v-if="!desktop">{{ t('应用锁仅在桌面应用中可用。') }}</p>

      <form v-else-if="!status.enabled" class="lock-form" @submit.prevent="setup">
        <label>
          {{ t('新密码') }}
          <input v-model="newPassword" type="password" autocomplete="new-password" required>
        </label>
        <label>
          {{ t('确认新密码') }}
          <input v-model="confirmation" type="password" autocomplete="new-password" required>
        </label>
        <label>
          {{ t('空闲自动锁定') }}
          <select v-model.number="setupTimeout">
            <option v-for="minutes in timeoutOptions" :key="minutes" :value="minutes">
              {{ minutes ? t('{0} 分钟', minutes) : t('关闭') }}
            </option>
          </select>
        </label>
        <div>
          <ElButton type="primary" native-type="submit" :loading="busy">{{ t('开启应用锁') }}</ElButton>
        </div>
      </form>

      <div v-else class="lock-form">
        <label>
          {{ t('空闲自动锁定') }}
          <select :value="status.idle_timeout_minutes" :disabled="busy || locking" @change="updateTimeout">
            <option v-for="minutes in timeoutOptions" :key="minutes" :value="minutes">
              {{ minutes ? t('{0} 分钟', minutes) : t('关闭') }}
            </option>
          </select>
        </label>
        <div class="lock-actions">
          <ElButton type="primary" :loading="locking" :disabled="busy" @click="emit('lock')">{{ t('立即锁定') }}</ElButton>
          <ElButton v-for="(item, key) in actions" :key="key" :disabled="busy || locking" @click="openAction(key)">
            {{ t(item.title) }}
          </ElButton>
        </div>
      </div>
      <p v-if="error && !action" class="lock-error" role="alert">{{ localizeMessage(error) }}</p>
      <p v-if="feedback" role="status">{{ t(feedback) }}</p>
    </div>

    <ElDialog
      v-model="actionDialog"
      :title="t(actions[action]?.title ?? '')"
      width="min(90vw, 450px)"
      :show-close="!busy"
      :close-on-press-escape="!busy"
      :close-on-click-modal="false"
    >
      <form class="lock-form" @submit.prevent="submitAction">
        <label>
          {{ t('当前密码') }}
          <input v-model="currentPassword" type="password" autocomplete="current-password" required>
        </label>
        <template v-if="action === 'change'">
          <label>
            {{ t('新密码') }}
            <input v-model="newPassword" type="password" autocomplete="new-password" required>
          </label>
          <label>
            {{ t('确认新密码') }}
            <input v-model="confirmation" type="password" autocomplete="new-password" required>
          </label>
        </template>
        <p v-if="action === 'regenerate'">{{ t('生成新恢复码后，原恢复码立即失效。') }}</p>
        <p v-if="error" class="lock-error" role="alert">{{ localizeMessage(error) }}</p>
        <div class="lock-actions">
          <ElButton :disabled="busy" @click="closeAction">{{ t('取消') }}</ElButton>
          <ElButton :type="action === 'disable' ? 'danger' : 'primary'" native-type="submit" :loading="busy">
            {{ t(actions[action]?.title ?? '') }}
          </ElButton>
        </div>
      </form>
    </ElDialog>

    <ElDialog
      :model-value="!!recoveryCode"
      :title="t('保存恢复码')"
      width="min(90vw, 450px)"
      :show-close="false"
      :close-on-press-escape="false"
      :close-on-click-modal="false"
    >
      <p>{{ t('恢复码仅在此处显示，可用一次来重置密码。请妥善保存。') }}</p>
      <code class="lock-code">{{ recoveryCode }}</code>
      <div class="lock-actions">
        <ElButton @click="copyRecoveryCode">{{ t('复制恢复码') }}</ElButton>
        <span aria-live="polite">{{ t(copyMessage) }}</span>
      </div>
      <label class="lock-checkbox">
        <input v-model="saved" type="checkbox">
        {{ t('我已妥善保存恢复码') }}
      </label>
      <ElButton type="primary" :disabled="!saved" @click="confirmSaved">{{ t('完成') }}</ElButton>
    </ElDialog>
  </section>
</template>
