import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { registerHooks } from 'node:module'
import { test } from 'node:test'
import { setImmediate } from 'node:timers/promises'
import { createRenderer } from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import { ElMessage, ElMessageBox } from 'element-plus'
import { clearMocks, mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { emit } from '@tauri-apps/api/event'
import { renderPlan } from '../src/planExport.js'
import { elementLocale, languages, locale, localizeMessage, setLocale, t as translate } from '../src/i18n.js'
import { subjectLabel } from '../src/classOptions.js'

// Exercise the component setup and IPC payloads without opening a desktop window.
registerHooks({
  load(url, context, next) {
    if (!url.endsWith('.vue')) return next(url, context)
    const file = new URL(url)
    const { descriptor } = parse(readFileSync(file, 'utf8'), { filename: file.pathname })
    return { format: 'module', source: compileScript(descriptor, { id: file.pathname }).content, shortCircuit: true }
  },
})
const { default: Scores } = await import('../src/Scores.vue')
const { default: Management } = await import('../src/Management.vue')
const { default: DataTools } = await import('../src/DataTools.vue')
const { default: Seating } = await import('../src/Seating.vue')
const { default: App } = await import('../src/App.vue')
const { default: LockScreen } = await import('../src/LockScreen.vue')
const { default: SecurityLock } = await import('../src/SecurityLock.vue')
const renderer = createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode() {}, nextSibling() {} })
globalThis.window = Object.assign(new EventTarget(), { atob, btoa, crypto })

async function mount(t, component, props, invoke, options) {
  mockIPC(invoke, options)
  let state
  const app = renderer.createApp({
    ...component,
    setup(props, context) {
      state = component.setup(props, context)
      return () => null
    },
  }, props)
  app.mount({})
  t.after(() => { app.unmount(); clearMocks() })
  await setImmediate()
  return state
}

const unlocked = { enabled: true, locked: false, corrupted: false, idle_timeout_minutes: 0, retry_after_seconds: 0 }
const locked = { ...unlocked, locked: true }

function desktopWindow(t) {
  mockWindows('main')
  globalThis.isTauri = true
  t.after(() => { delete globalThis.isTauri })
}

test('cold start calls only status until unlock and then initializes the workspace once', async t => {
  desktopWindow(t)
  const calls = []
  const state = await mount(t, App, {}, (command) => {
    calls.push(command)
    if (command === 'app_lock_status') return locked
    return []
  })
  assert.equal(calls[0], 'app_lock_status')
  assert.equal(state.workspaceReady.value, false)
  assert.ok(!calls.includes('desktop_status'))
  assert.ok(!calls.includes('dashboard_summary'))
  assert.ok(!calls.includes('list_records'))
  await state.enterWorkspace(unlocked)
  assert.equal(state.workspaceReady.value, true)
  assert.deepEqual(calls.filter(command => !command.startsWith('plugin:')), [
    'app_lock_status', 'desktop_status', 'dashboard_summary', 'list_records',
  ])
  await state.enterWorkspace(unlocked)
  assert.equal(calls.filter(command => command === 'dashboard_summary').length, 1)
})

test('a desktop status failure stays closed', async t => {
  desktopWindow(t)
  const state = await mount(t, App, {}, command => {
    if (command === 'app_lock_status') throw new Error('fixture offline')
    assert.match(command, /^plugin:/)
  })
  assert.equal(state.workspaceReady.value, false)
  assert.equal(state.startupError.value, 'fixture offline')
})

test('browser preview starts without desktop lock IPC', async t => {
  const calls = []
  const preview = await mount(t, App, {}, command => { calls.push(command); return [] })
  assert.equal(preview.workspaceReady.value, true)
  assert.equal(preview.appLock.value.enabled, false)
  assert.ok(!calls.includes('app_lock_status'))
})

test('shortcut lock keeps the editor and blocks closing with a draft while locked', async t => {
  desktopWindow(t)
  const calls = []
  const draft = { value: '尚未保存的成绩', hasUnsavedChanges: () => true, isBusy: () => true }
  let prompts = 0
  t.mock.method(ElMessageBox, 'confirm', async () => { prompts++ })
  const state = await mount(t, App, {}, command => {
    calls.push(command)
    if (command === 'app_lock_status') return unlocked
    if (command === 'lock_app') {
      assert.equal(state.appLock.value.locked, false)
      return locked
    }
    return []
  }, { shouldMockEvents: true })
  state.page.value = 'scores'
  state.activePage.value = draft
  const shortcut = Object.assign(new Event('keydown', { cancelable: true }), {
    code: 'KeyL', metaKey: true, shiftKey: true,
  })
  window.dispatchEvent(shortcut)
  await setImmediate()
  assert.ok(shortcut.defaultPrevented)
  assert.equal(state.appLock.value.locked, true)
  assert.equal(state.workspaceReady.value, true)
  assert.equal(state.page.value, 'scores')
  assert.equal(state.activePage.value.value, draft.value)
  await emit('tauri://close-requested')
  await setImmediate()
  assert.equal(prompts, 0)
  assert.ok(!calls.includes('plugin:window|destroy'))
  assert.equal(state.closeBlocked.value, true)
  await state.enterWorkspace(unlocked)
  assert.equal(state.closeBlocked.value, false)
  assert.equal(state.activePage.value.value, draft.value)
  assert.equal(calls.filter(command => command === 'dashboard_summary').length, 1)
  await state.lockWorkspace()
  await state.refreshGlobalData()
  assert.equal(calls.filter(command => command === 'dashboard_summary').length, 1)
  await state.enterWorkspace(unlocked)
  assert.equal(calls.filter(command => command === 'dashboard_summary').length, 2)
  const appSource = readFileSync(new URL('../src/App.vue', import.meta.url), 'utf8')
  assert.match(appSource, /v-if="workspaceReady"\s+v-show="!appLock\.locked"\s+:inert="appLock\.locked"/)
  assert.match(appSource, /:close-blocked="closeBlocked"/)
})

test('closing is blocked during a pending lock with a draft and allowed after locking without a draft', async t => {
  desktopWindow(t)
  const pending = Promise.withResolvers()
  const calls = []
  const confirm = t.mock.method(ElMessageBox, 'confirm', async () => {})
  const state = await mount(t, App, {}, command => {
    calls.push(command)
    if (command === 'app_lock_status') return unlocked
    if (command === 'lock_app') return pending.promise
    return []
  }, { shouldMockEvents: true })
  state.activePage.value = { hasUnsavedChanges: () => true }
  const locking = state.lockWorkspace()
  assert.equal(state.lockPending.value, true)
  await emit('tauri://close-requested')
  await setImmediate()
  assert.equal(confirm.mock.callCount(), 0)
  assert.ok(!calls.includes('plugin:window|destroy'))
  assert.equal(state.closeBlocked.value, true)

  pending.resolve(locked)
  await locking
  assert.equal(state.appLock.value.locked, true)
  state.activePage.value = { hasUnsavedChanges: () => false }
  await emit('tauri://close-requested')
  await setImmediate()
  assert.equal(confirm.mock.callCount(), 0)
  assert.ok(calls.includes('plugin:window|destroy'))
})

test('accepting a draft close prompt checks whether the app locked during the prompt', async t => {
  desktopWindow(t)
  const confirmation = Promise.withResolvers()
  const calls = []
  const confirm = t.mock.method(ElMessageBox, 'confirm', () => confirmation.promise)
  const state = await mount(t, App, {}, command => {
    calls.push(command)
    if (command === 'app_lock_status') return unlocked
    if (command === 'lock_app') return locked
    return []
  }, { shouldMockEvents: true })
  state.activePage.value = { hasUnsavedChanges: () => true }
  await emit('tauri://close-requested')
  await setImmediate()
  assert.equal(confirm.mock.callCount(), 1)
  await state.lockWorkspace()
  confirmation.resolve()
  await setImmediate()
  assert.ok(!calls.includes('plugin:window|destroy'))
  await state.enterWorkspace(unlocked)
  await emit('tauri://close-requested')
  await setImmediate()
  assert.equal(confirm.mock.callCount(), 2)
  assert.ok(calls.includes('plugin:window|destroy'))
})

test('activity resets the idle lock timer and an off setting clears it', async t => {
  desktopWindow(t)
  t.mock.timers.enable({ apis: ['setTimeout'] })
  let lockCalls = 0
  const timeout = { ...unlocked, idle_timeout_minutes: 5 }
  const state = await mount(t, App, {}, command => {
    if (command === 'app_lock_status') return timeout
    if (command === 'lock_app') { lockCalls++; return { ...timeout, locked: true } }
    return []
  })
  t.mock.timers.tick(299_000)
  window.dispatchEvent(new Event('wheel'))
  t.mock.timers.tick(299_000)
  assert.equal(lockCalls, 0)
  t.mock.timers.tick(1000)
  await setImmediate()
  assert.equal(lockCalls, 1)
  await state.enterWorkspace(timeout)
  await setImmediate()
  state.updateLockSettings(unlocked)
  await setImmediate()
  t.mock.timers.tick(600_000)
  assert.equal(lockCalls, 1)
})

test('lock screen handles cooldown and keeps the workspace hidden until the new code is acknowledged', async t => {
  let status = locked
  const calls = []
  const recovered = []
  const screen = await mount(t, LockScreen, { status: locked, onUnlocked: value => recovered.push(value) }, (command, args) => {
    calls.push([command, args])
    if (command === 'unlock_app') throw '密码错误次数过多，请在 5 秒后重试'
    if (command === 'app_lock_status') return status
    return { status: unlocked, recovery_code: 'ABCD-ABCD-ABCD-ABCD' }
  })
  status = { ...locked, retry_after_seconds: 5 }
  screen.password.value = 'wrong-password'
  await screen.submit()
  assert.equal(screen.retrySeconds.value, 5)
  assert.equal(screen.password.value, '')
  await screen.switchForm()
  await screen.submit()
  assert.equal(calls.length, 2)
  screen.retrySeconds.value = 0
  screen.recoveryCode.value = 'abcd abcd abcd abcd'
  screen.newPassword.value = 'new-password'
  screen.confirmation.value = 'different'
  await screen.submit()
  assert.equal(calls.length, 2)
  screen.confirmation.value = 'new-password'
  await screen.submit()
  assert.deepEqual(calls.at(-1), ['recover_app_lock', {
    recoveryCode: 'abcd abcd abcd abcd', newPassword: 'new-password',
  }])
  assert.equal(recovered.length, 0)
  assert.equal(screen.result.value.recovery_code, 'ABCD-ABCD-ABCD-ABCD')
  await screen.confirmSaved()
  assert.equal(recovered.length, 0)
  screen.saved.value = true
  status = { ...unlocked, idle_timeout_minutes: 10 }
  await screen.confirmSaved()
  assert.deepEqual(calls.at(-1), ['app_lock_status', {}])
  assert.deepEqual(recovered, [status])
  assert.equal(screen.result.value, null)
})

async function recoverPassword(screen) {
  await screen.switchForm()
  screen.recoveryCode.value = 'abcd abcd abcd abcd'
  screen.newPassword.value = screen.confirmation.value = 'new-password'
  await screen.submit()
  assert.equal(screen.result.value.recovery_code, 'ABCD-ABCD-ABCD-ABCD')
  screen.saved.value = true
  screen.copyMessage.value = '已复制恢复码'
}

function assertRecoveryCleared(screen) {
  assert.equal(screen.result.value, null)
  assert.equal(screen.recovering.value, false)
  assert.equal(screen.saved.value, false)
  assert.equal(screen.copyMessage.value, '')
  assert.equal(screen.password.value, '')
}

test('recovery code display resets its idle timer on activity and clears immediately when locking', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] })
  const status = { ...unlocked, idle_timeout_minutes: 5 }
  const pending = Promise.withResolvers()
  const calls = []
  const screen = await mount(t, LockScreen, { status: locked }, command => {
    calls.push(command)
    if (command === 'lock_app') return pending.promise
    if (command === 'unlock_app') return status
    return { status, recovery_code: 'ABCD-ABCD-ABCD-ABCD' }
  })
  await recoverPassword(screen)
  t.mock.timers.tick(299_000)
  window.dispatchEvent(new Event('wheel'))
  t.mock.timers.tick(299_000)
  assert.ok(screen.result.value)
  assert.deepEqual(calls, ['recover_app_lock'])
  t.mock.timers.tick(1000)
  assert.deepEqual(calls, ['recover_app_lock', 'lock_app'])
  assertRecoveryCleared(screen)
  assert.equal(screen.busy.value, true)
  pending.resolve(locked)
  await setImmediate()
  screen.password.value = 'new-password'
  await screen.submit()
  assert.equal(calls.at(-1), 'unlock_app')
})

test('acknowledging a recovery code keeps the workspace closed when the latest status is locked', async t => {
  const calls = []
  const recovered = []
  const screen = await mount(t, LockScreen, { status: locked, onUnlocked: status => recovered.push(status) }, command => {
    calls.push(command)
    if (command === 'app_lock_status') return locked
    return { status: unlocked, recovery_code: 'ABCD-ABCD-ABCD-ABCD' }
  })
  await recoverPassword(screen)
  await screen.confirmSaved()
  assert.deepEqual(calls, ['recover_app_lock', 'app_lock_status'])
  assert.deepEqual(recovered, [])
  assertRecoveryCleared(screen)
})

test('recovery shortcut clears the code and prevents a concurrent status response from unlocking', async t => {
  const pending = Promise.withResolvers()
  const calls = []
  const recovered = []
  const screen = await mount(t, LockScreen, { status: locked, onUnlocked: status => recovered.push(status) }, command => {
    calls.push(command)
    if (command === 'app_lock_status') return pending.promise
    if (command === 'lock_app') return locked
    return { status: unlocked, recovery_code: 'ABCD-ABCD-ABCD-ABCD' }
  })
  await recoverPassword(screen)
  const confirming = screen.confirmSaved()
  const shortcut = Object.assign(new Event('keydown', { cancelable: true }), {
    code: 'KeyL', ctrlKey: true, shiftKey: true,
  })
  window.dispatchEvent(shortcut)
  assert.ok(shortcut.defaultPrevented)
  assertRecoveryCleared(screen)
  pending.resolve(unlocked)
  await confirming
  assert.deepEqual(calls, ['recover_app_lock', 'app_lock_status', 'lock_app'])
  assert.deepEqual(recovered, [])
})

test('security settings send password confirmations and show the returned recovery code once', async t => {
  const calls = []
  const statusChanges = []
  const state = await mount(t, SecurityLock, {
    desktop: true, status: { ...unlocked, enabled: false }, onStatusChange: status => statusChanges.push(status),
  }, (command, args) => {
    calls.push([command, args])
    if (command === 'setup_app_lock') return { status: unlocked, recovery_code: 'ABCD-ABCD-ABCD-ABCD' }
    return unlocked
  })
  state.newPassword.value = '設定したパスワード'
  await state.setup()
  assert.equal(calls.length, 0)
  assert.equal(state.hasUnsavedChanges(), true)
  state.confirmation.value = state.newPassword.value
  state.setupTimeout.value = 10
  await state.setup()
  assert.deepEqual(calls[0], ['setup_app_lock', { password: '設定したパスワード', idleTimeoutMinutes: 10 }])
  assert.equal(state.newPassword.value, '')
  assert.equal(state.setupTimeout.value, 0)
  state.saved.value = true
  state.confirmSaved()
  assert.equal(state.recoveryCode.value, '')
  state.openAction('change')
  state.currentPassword.value = '設定したパスワード'
  state.newPassword.value = state.confirmation.value = 'new-password'
  await state.submitAction()
  assert.deepEqual(calls[1], ['change_app_lock_password', {
    currentPassword: '設定したパスワード', newPassword: 'new-password',
  }])
  assert.equal(state.currentPassword.value, '')
  assert.deepEqual(statusChanges, [unlocked, unlocked])
})

test('app lock errors localize cooldown arguments and corruption guidance across five locales', t => {
  t.after(() => setLocale('zh-Hans'))
  const template = '密码错误次数过多，请在 {0} 秒后重试'
  const message = '密码错误次数过多，请在 15 秒后重试'
  const corrupted = '应用锁配置损坏。请检查或移除应用数据目录中的 app-lock.json，再重新启动。'
  const closeBlocked = '当前存在未保存的修改，请先解锁后再关闭窗口。'
  for (const language of languages) {
    setLocale(language.id)
    assert.equal(localizeMessage(message), translate(template, 15))
    assert.ok(localizeMessage(message).includes('15'))
    assert.ok(localizeMessage(corrupted).includes('app-lock.json'))
    if (language.id !== 'zh-Hans') {
      assert.notEqual(localizeMessage(message), message)
      assert.notEqual(localizeMessage(corrupted), corrupted)
      assert.notEqual(translate(closeBlocked), closeBlocked)
      assert.notEqual(translate('应用锁'), '应用锁')
    }
  }
})

test('model discovery, connection test and save use the current endpoint', async t => {
  const calls = []
  const profiles = { profiles: [], default_id: null }
  t.mock.method(ElMessage, 'info', () => {})
  t.mock.method(ElMessage, 'success', () => {})
  const form = await mount(t, DataTools, { mode: 'ai' }, (cmd, args) => {
    calls.push([cmd, args])
    if (cmd === 'fetch_ai_models') return []
    return profiles
  })
  await form.openProfile()
  const profile = form.editor.value
  profile.name = '测试配置'
  profile.provider = 'OLLAMA'
  form.applyPreset()
  form.key.value = 'fixture-A'
  profile.provider = 'CUSTOM'
  form.applyPreset()
  assert.equal(form.key.value, '')
  form.key.value = 'fixture-A'
  profile.settings.endpoint = 'http://127.0.0.1:9002/v1/chat/completions'
  profile.settings.model = 'demo'
  await form.fetchModels()
  assert.equal(calls.at(-1)[0], 'fetch_ai_models')
  assert.equal(calls.at(-1)[1].key, null)
  assert.equal(calls.at(-1)[1].useSavedKey, false)
  await form.testConnection()
  assert.equal(calls.at(-1)[1].key, null)
  form.key.value = 'fixture-B'
  await form.saveSettings()
  assert.equal(calls.at(-1)[0], 'save_ai_profile')
  assert.equal(calls.at(-1)[1].key.endpoint, profile.settings.endpoint)
  assert.equal(calls.at(-1)[1].key.value, 'fixture-B')
})

test('restore results use distinct notifications and refresh the actual database state', async t => {
  let status
  let refreshes = 0
  const notices = []
  for (const kind of ['success', 'warning', 'error']) t.mock.method(ElMessage, kind, () => notices.push(kind))
  t.mock.method(ElMessageBox, 'confirm', async () => {})
  const form = await mount(t, DataTools, { mode: 'backup', onRecoveryChanged: () => refreshes++ }, cmd => {
    if (cmd === 'apply_backup') return { status, message: status }
    if (cmd === 'recovery_status') return { pending: status === 'blocked' }
    return []
  })
  for (status of ['restored', 'rolled_back', 'blocked']) {
    form.summary.value = { token: 'fixture' }
    await form.restore()
    assert.equal(form.recovery.value.pending, status === 'blocked')
  }
  assert.deepEqual(notices, ['success', 'warning', 'error'])
  assert.equal(refreshes, 3)
})

test('score, exam and template drawers preserve edits on cancel and guard pending saves', async t => {
  let discard = false
  let prompts = 0
  t.mock.method(ElMessageBox, 'confirm', async () => { prompts++; if (!discard) throw new Error('cancel') })
  const exam = { id: 1, class_id: 1, name: '考试', is_current: true, subjects: [{ id: 1, name: '数学', full_score: 10000 }] }
  const rows = [1, 2].map(id => ({ student_id: id, student_no: String(id), name: String(id), details: [], remark: '', active: true }))
  let saves = 0
  const form = await mount(t, Scores, { classId: 1, classes: [{ id: 1, subjects: '数学' }] }, cmd => {
    if (cmd === 'list_exams') return [exam]
    if (cmd === 'list_scores') return rows
    if (cmd === 'save_scores') saves++
    return []
  })
  for (const [open, draft, field] of [
    [() => form.editScore(rows[0]), form.scoreDraft, 'remark'],
    [() => form.newExam(), form.editor, 'name'],
    [() => form.editExam(), form.editor, 'name'],
    [() => form.editTemplate(), form.templateEditor, 'name'],
  ]) {
    await open()
    assert.equal(form.dirty.value, false)
    const before = prompts
    await form.closeDrawer()
    assert.equal(prompts, before)
    await open()
    draft.value[field] = '已修改'
    discard = false
    await form.closeDrawer()
    assert.equal(prompts, before + 1)
    assert.equal(draft.value[field], '已修改')
    discard = true
    form.saving.value = true
    await form.closeDrawer()
    assert.ok(draft.value)
    form.saving.value = false
    await form.closeDrawer()
    assert.equal(draft.value, null)
  }
  t.mock.method(ElMessage, 'success', () => {})
  await form.editScore(rows[0])
  form.scoreDraft.value.values[1] = '0'
  await form.saveScore(true)
  assert.equal(saves, 1)
  assert.equal(form.scoreDraft.value.student_id, 2)
  assert.equal(form.dirty.value, false)
})

test('language preferences persist and localize nested messages without rewriting file paths or names', async t => {
  const settings = new Map()
  window.localStorage = {
    getItem: key => settings.get(key) ?? null,
    setItem: (key, value) => settings.set(key, value),
  }
  globalThis.document = { documentElement: { lang: 'zh-Hans' } }
  t.after(() => {
    setLocale('zh-Hans')
    delete window.localStorage
    delete globalThis.document
  })
  assert.deepEqual(languages.map(({ id }) => id), ['zh-Hans', 'en', 'zh-Hant', 'ja', 'ko'])
  for (const [id, label, elementName] of [
    ['zh-Hans', '智能排座', 'zh-cn'],
    ['en', 'Seating', 'en'],
    ['zh-Hant', '智能排座', 'zh-tw'],
    ['ja', '座席配置', 'ja'],
    ['ko', '좌석 배정', 'ko'],
  ]) {
    setLocale(id)
    assert.equal(translate('智能排座'), label)
    assert.equal(elementLocale().name, elementName)
    assert.equal(document.documentElement.lang, id)
  }
  setLocale('es')
  assert.equal(locale.value, 'ko')
  assert.equal(settings.get('classmatehub.locale'), 'ko')
  const reopened = await import('../src/i18n.js?test-reopen-language')
  assert.equal(reopened.locale.value, 'ko')

  setLocale('en')
  assert.equal(subjectLabel('数学'), 'Mathematics')
  assert.equal(subjectLabel('学号'), '学号')
  assert.equal(
    localizeMessage('第 3 行第 5 列：分数超过科目满分'),
    "Row 3, column 5: The score exceeds the subject's maximum.",
  )
  assert.equal(
    localizeMessage('锁座冲突：「工作台」与「学生」不能同桌'),
    'Seat lock conflict: “工作台” and “学生” cannot share a desk.',
  )
  const path = 'E:/Synthetic/成绩资料.exe'
  assert.ok(localizeMessage(`新版安装包已保存到：${path}。请先关闭班伴，再运行安装包覆盖升级。`).includes(path))
})

test('localized absent input keeps the existing score command value', async t => {
  const exam = { id: 1, class_id: 1, name: 'Test exam', is_current: true, subjects: [{ id: 1, name: '数学', full_score: 10000 }] }
  const row = { student_id: 1, student_no: '01', name: 'Test student', details: [], remark: '', active: true }
  let saved
  t.mock.method(ElMessage, 'success', () => {})
  const form = await mount(t, Scores, { classId: 1 }, (command, args) => {
    if (command === 'list_exams') return [exam]
    if (command === 'list_scores') return [row]
    if (command === 'save_scores') saved = args.input
    return []
  })
  setLocale('ja')
  t.after(() => setLocale('zh-Hans'))
  await form.editScore(row)
  form.updateScoreInput(1, '欠席')
  assert.equal(form.scoreDraft.value.values[1], '缺考')
  setLocale('ko')
  assert.equal(form.scoreDraft.value.values[1], '缺考')
  await form.saveScore()
  assert.deepEqual(saved, [{ student_id: 1, subject_id: 1, value: '缺考' }])
})

test('score export and score template send native save arguments and only report success after saving', async t => {
  setLocale('ja')
  t.after(() => setLocale('zh-Hans'))
  const success = t.mock.method(ElMessage, 'success', () => {})
  const exports = []
  const templates = []
  const form = await mount(t, Scores, { classId: 1 }, (command, args) => {
    if (command === 'list_exams') return [{ id: 7, is_current: true, subjects: [] }]
    if (command === 'export_scores') return exports.push(args) > 1
    if (command === 'score_template') return templates.push(args) > 1
    return []
  })
  form.exportByStudentNo.value = true
  await form.download(true)
  assert.deepEqual(exports, [{ examId: 7, byStudentNo: true, locale: 'ja' }])
  assert.equal(success.mock.callCount(), 0)
  await form.download(true)
  assert.equal(success.mock.callCount(), 1)
  await form.download(false)
  assert.deepEqual(templates, [{ examId: 7, locale: 'ja' }])
  assert.equal(success.mock.callCount(), 1)
  await form.download(false)
  assert.equal(success.mock.callCount(), 2)
})

test('records export sends the localized default name and only reports success after saving', async t => {
  setLocale('ja')
  t.after(() => setLocale('zh-Hans'))
  const success = t.mock.method(ElMessage, 'success', () => {})
  const exports = []
  const form = await mount(t, Management, { section: 'classes', classId: 1 }, (command, args) => {
    if (command === 'list_records' && args.resource === 'classes') return [{ id: 1, name: 'Fixture class' }]
    if (command === 'export_records') return exports.push(args) > 1
    return []
  })
  await form.exportRecords()
  assert.deepEqual(exports, [{ resource: 'classes', classId: 1, ids: [1], locale: 'ja', filename: 'クラスの情報.xlsx' }])
  assert.equal(success.mock.callCount(), 0)
  await form.exportRecords()
  assert.equal(success.mock.callCount(), 1)
})

test('seating views reverse the display while keeping the original seats and assignments', async t => {
  const requests = []
  const positions = [
    { id: 11, row: 1, column: 1, kind: 'seat' },
    { id: 12, row: 1, column: 2, kind: 'aisle' },
    { id: 13, row: 1, column: 3, kind: 'unavailable' },
    { id: 23, row: 2, column: 3, kind: 'seat' },
  ]
  const assignments = [{ position_id: 11, student_id: 1 }, { position_id: 23, student_id: 2 }]
  const snapshot = {
    rows: 2, columns: 3, positions,
    students: [{ id: 1, name: '测试甲' }, { id: 2, name: '测试乙' }],
  }
  const form = await mount(t, Seating, {}, (command, args) => {
    requests.push([command, args])
    if (command === 'adjust_plan') return args.plan
    if (command === 'list_ai_profiles') return { profiles: [] }
  })
  for (const [side, front, studentPodium, teacherPodium] of [
    [null, true, 'front', 'back'],
    [null, false, 'back', 'front'],
    ['left', false, 'left', 'right'],
    ['right', false, 'right', 'left'],
  ]) {
    form.setPlan({ id: 1, assignments, snapshot: { ...snapshot, podium_side: side, podium_front: front } })
    form.teacher.value = false
    const studentGrid = form.grid.value
    assert.deepEqual(studentGrid.map(cell => cell.position?.id), [11, 12, 13, undefined, undefined, 23])
    assert.deepEqual(form.displayColumns.value, [1, 2, 3])
    assert.deepEqual(form.gridRows.value.map(entry => entry.row), [1, 2])
    await form.exportFile('xlsx')
    assert.equal(requests.at(-1)[1].teacher, false)
    form.chooseCell(studentGrid[0])
    const beforeSwitch = requests.length
    form.teacher.value = true
    const teacherGrid = form.grid.value
    assert.deepEqual(teacherGrid, [...studentGrid].reverse())
    assert.deepEqual(form.displayColumns.value, [3, 2, 1])
    assert.deepEqual(form.gridRows.value.map(entry => entry.row), [2, 1])
    assert.equal(teacherGrid[0].student.name, '测试乙')
    assert.equal(form.chosen.value.position.id, 11)
    assert.equal(form.podium.value, teacherPodium)
    assert.equal(form.dirty.value, false)
    assert.equal(requests.length, beforeSwitch)
    await form.exportFile('xlsx')
    assert.equal(requests.at(-1)[1].teacher, true)
    form.teacher.value = false
    assert.equal(form.podium.value, studentPodium)
    assert.deepEqual(form.grid.value, studentGrid)
    form.teacher.value = true
    form.chooseCell(form.grid.value[0])
    await setImmediate()
    assert.deepEqual(requests.at(-1)[1].adjustment, { kind: 'move', from: 11, to: 23 })
  }
})

test('PNG and single-page PDF use white-background table headers with centered names and no ratings', async t => {
  // Record Canvas layout and let jsPDF embed a valid one-pixel fixture on the single page.
  const pixel = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAAXNSR0IArs4c6QAAAARnQU1BAACxjwv8YQUAAAAJcEhZcwAADsMAAA7DAcdvqGQAAAANSURBVBhXY/j///9/AAn7A/0FQ0XKAAAAAElFTkSuQmCC'
  const pages = []
  globalThis.document = {
    fonts: { ready: Promise.resolve() },
    createElement() {
      const labels = new Map()
      const fills = []
      const strokes = []
      pages.push({ labels, fills, strokes })
      const ctx = {
        fillStyle: '#ffffff',
        strokeStyle: '#000000',
        lineWidth: 1,
        font: '',
        textAlign: 'left',
        textBaseline: 'top',
        scale() {},
        fillRect(x, y, w, h) { fills.push({ color: ctx.fillStyle, x, y, w, h }) },
        strokeRect(x, y, w, h) { strokes.push({ color: ctx.strokeStyle, x, y, w, h }) },
        measureText: value => ({ width: value.length * 10 }),
        fillText: (value, x, y) => {
          const list = labels.get(value) ?? []
          list.push({ x, y, align: ctx.textAlign, baseline: ctx.textBaseline })
          labels.set(value, list)
        },
      }
      return {
        getContext: () => ctx,
        toBlob: callback => callback(new Blob([Buffer.from(pixel, 'base64')])),
        toDataURL: () => 'data:image/png;base64,' + pixel,
      }
    },
  }
  t.after(() => { delete globalThis.document })

  const sheet = {
    class_name: '测试班', rows: 9, columns: 8,
    cells: [
      { row: 1, column: 1, kind: 'seat', name: '测试甲', number: '202601' },
      { row: 9, column: 8, kind: 'seat', name: '测试乙', number: '202602' },
    ],
    ratings: [{ label: '男女搭配均衡', score: 2086, maximum: 4000 }],
  }
  const studentPng = await renderPlan({ ...sheet, podium: 'front', podium_front: true, teacher: false }, 'png')
  assert.equal(Buffer.from(studentPng).toString('ascii', 1, 4), 'PNG')
  const student = pages.shift()
  assert.ok(student.fills.every(item => item.color === '#ffffff'))
  assert.ok(student.strokes.length > 0)
  assert.ok(!student.labels.has('规则评分'))
  assert.ok(!student.labels.has('1排 1座'))
  assert.ok([...student.labels.keys()].every(text => !text.includes('视角')))
  assert.ok(student.labels.has('排 \\ 列'))
  assert.equal(student.labels.get('测试甲')[0].align, 'center')
  assert.ok(student.labels.get('讲台')[0].y < student.labels.get('测试甲')[0].y)
  assert.ok(student.labels.get('测试甲')[0].x < student.labels.get('测试乙')[0].x)
  assert.ok(student.labels.get('测试甲')[0].y < student.labels.get('测试乙')[0].y)

  const teacherSheet = { ...sheet, podium: 'back', podium_front: false, teacher: true }
  await renderPlan(teacherSheet, 'png')
  const teacher = pages.shift()
  assert.ok(teacher.fills.every(item => item.color === '#ffffff'))
  assert.ok(!teacher.labels.has('规则评分'))
  assert.ok(!teacher.labels.has('1排 1座'))
  assert.ok([...teacher.labels.keys()].every(text => !text.includes('视角')))
  assert.ok(teacher.labels.get('讲台')[0].y > teacher.labels.get('测试甲')[0].y)
  assert.ok(teacher.labels.get('测试乙')[0].x < teacher.labels.get('测试甲')[0].x)
  assert.ok(teacher.labels.get('测试乙')[0].y < teacher.labels.get('测试甲')[0].y)

  const pdf = await renderPlan(teacherSheet, 'pdf')
  assert.equal(Buffer.from(pdf).toString('ascii', 0, 5), '%PDF-')
  assert.equal(pages.length, 1)
  assert.ok(pages[0].labels.has('测试甲') && pages[0].labels.has('测试乙'))
  assert.ok(!pages[0].labels.has('规则评分'))
})

test('seating opens plan list by default and guards returning to list when unsaved', async t => {
  let discard = false
  let prompts = 0
  t.mock.method(ElMessageBox, 'confirm', async () => {
    prompts++
    if (!discard) throw new Error('cancel')
  })
  const entries = [
    { id: 10, name: '第一次智能排座', confirmed: true, archived: false, source: 'offline', updated_at: '2026-11-24T10:00:00' },
    { id: 11, name: 'AI 备选方案', confirmed: false, archived: false, source: 'ai', updated_at: '2026-11-24T10:30:00' },
  ]
  const detail = id => ({
    id,
    name: entries.find(item => item.id === id)?.name ?? '方案',
    confirmed: id === 10,
    archived: false,
    source: id === 11 ? 'ai' : 'offline',
    seed: 1,
    assignments: [],
    ratings: [],
    snapshot: { rows: 1, columns: 1, podium_side: 'front', podium_front: true, classroom_name: '默认教室', positions: [], students: [], warnings: [] },
  })
  const form = await mount(t, Seating, { classId: 1, classes: [{ id: 1, name: '一年级' }] }, (command, args) => {
    if (command === 'list_records') return [{ id: 1, name: '默认教室', active: true, is_default: true }]
    if (command === 'list_exams') return []
    if (command === 'list_plans') return entries
    if (command === 'inspect_plan_inputs') return []
    if (command === 'open_plan') return detail(args.id)
    if (command === 'list_ai_profiles') return { profiles: [], default_id: null }
    return []
  })
  assert.equal(form.plan.value, null)
  assert.equal(form.history.value.length, 2)
  await form.openHistoryItem(entries[1])
  assert.equal(form.plan.value.id, 11)
  await form.backToList()
  assert.equal(form.plan.value, null)
  assert.equal(prompts, 0)

  await form.openHistoryItem(entries[1])
  form.dirty.value = true
  discard = false
  await form.backToList()
  assert.equal(prompts, 1)
  assert.equal(form.plan.value.id, 11)
  discard = true
  await form.backToList()
  assert.equal(prompts, 2)
  assert.equal(form.plan.value, null)
  assert.equal(form.dirty.value, false)
})
