import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { registerHooks } from 'node:module'
import { test } from 'node:test'
import { setImmediate } from 'node:timers/promises'
import { createRenderer } from 'vue'
import { compileScript, parse } from 'vue/compiler-sfc'
import { ElMessage, ElMessageBox } from 'element-plus'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { renderPlan } from '../src/planExport.js'

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
const { default: DataTools } = await import('../src/DataTools.vue')
const { default: Seating } = await import('../src/Seating.vue')
const renderer = createRenderer({ createComment: () => ({}), insert() {}, remove() {}, parentNode() {}, nextSibling() {} })
globalThis.window = { atob, btoa }

async function mount(t, component, props, invoke) {
  mockIPC(invoke)
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
