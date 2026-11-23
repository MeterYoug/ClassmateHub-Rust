<script setup>
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  ElAlert,
  ElButton,
  ElCheckbox,
  ElCheckboxGroup,
  ElCollapse,
  ElCollapseItem,
  ElDialog,
  ElDropdown,
  ElDropdownItem,
  ElDropdownMenu,
  ElEmpty,
  ElIcon,
  ElInput,
  ElInputNumber,
  ElOption,
  ElPopover,
  ElProgress,
  ElRadioButton,
  ElRadioGroup,
  ElSelect,
  ElTable,
  ElTableColumn,
  ElTabPane,
  ElTabs,
  ElTag,
  ElTooltip,
} from 'element-plus'
import {
  ArrowDown,
  Back,
  CopyDocument,
  Download,
  Lock,
  MagicStick,
  Plus,
  Refresh,
  Right,
  Search,
  Setting,
  Unlock,
} from '@element-plus/icons-vue'
import { renderPlan } from './planExport'
import { confirmAction, notifyError, notifyInfo, notifySuccess, notifyWarning } from './ui.js'

const props = defineProps({
  classId: { type: Number, default: null },
  classes: { type: Array, default: () => [] },
})

const rooms = ref([])
const roomId = ref(null)
const exams = ref([])
const examId = ref(null)
const history = ref([])
const historySearch = ref('')
const historyStatus = ref('')
const comparison = ref(null)
const undoStack = ref([])
const redoStack = ref([])
const batchMode = ref(false)
const editingActive = ref(false)
const batchSelection = ref([])
const pendingStudent = ref(null)
const inputWarnings = ref([])
const plan = ref(null)
const seed = ref(1)
const selected = ref(null)
const busy = ref(false)
const dirty = ref(false)
const teacher = ref(false)
const showBadges = ref(['gender', 'vision', 'score', 'discipline'])
const findQuery = ref('')
const inspectorOpen = ref(true)
const inspectorTab = ref('score')
const generateDialog = ref(false)
const advancedSeedOpen = ref([])
const dragOverId = ref(null)

const aiPreview = ref(null)
const aiFailure = ref('')
const aiRunning = ref(false)
const autoFallback = ref(false)
const aiProfiles = ref([])
const aiProfileId = ref(null)

let dragFrom = null
let dragStudent = null

const scoreLevelLabels = ['A', 'B', 'C', 'D', 'E']

const grid = computed(() => {
  if (!plan.value) return []
  const { snapshot, assignments } = plan.value
  const positions = new Map(snapshot.positions.map(item => [`${item.row}:${item.column}`, item]))
  const students = new Map(snapshot.students.map(item => [item.id, item]))
  const occupied = new Map(assignments.map(item => [item.position_id, item]))
  return Array.from({ length: snapshot.rows * snapshot.columns }, (_, index) => {
    const row = Math.floor(index / snapshot.columns) + 1
    const column = (index % snapshot.columns) + 1
    const position = positions.get(`${row}:${column}`)
    const assignment = occupied.get(position?.id)
    return { row, column, position, assignment, student: students.get(assignment?.student_id) }
  })
})

const total = computed(() => (plan.value?.ratings ?? []).reduce((sum, item) => sum + item.score, 0))
const maximum = computed(() => (plan.value?.ratings ?? []).reduce((sum, item) => sum + item.maximum, 0))
const scorePercent = computed(() => (maximum.value ? Math.round((total.value / maximum.value) * 100) : 0))
const chosen = computed(() => grid.value.find(cell => cell.position?.id === selected.value))
const editable = computed(() => !!plan.value && (!plan.value.confirmed || editingActive.value))
const planStatus = item => (item.archived ? '已归档' : item.confirmed ? '已启用' : '草稿')
const planStatusTagType = item => (item.archived ? 'info' : item.confirmed ? 'success' : 'warning')

const filteredHistory = computed(() =>
  history.value.filter(
    item =>
      (!historyStatus.value || planStatus(item) === historyStatus.value) &&
      item.name.toLowerCase().includes(historySearch.value.trim().toLowerCase()),
  ),
)

const unseated = computed(() => {
  if (!plan.value) return []
  const occupied = new Set(plan.value.assignments.map(item => item.student_id))
  return plan.value.snapshot.students.filter(item => !occupied.has(item.id))
})

const matchedStudentIds = computed(() => {
  const query = findQuery.value.trim().toLowerCase()
  if (!query || !plan.value) return new Set()
  return new Set(
    plan.value.snapshot.students
      .filter(s => s.name.toLowerCase().includes(query) || s.number.toLowerCase().includes(query))
      .map(s => s.id),
  )
})

const comparisonChanges = computed(() => {
  if (!comparison.value || !plan.value) return []
  function positionsByStudent(value) {
    const positions = new Map(value.snapshot.positions.map(item => [item.id, item]))
    return new Map(
      value.assignments.map(item => {
        const position = positions.get(item.position_id)
        const room = value.snapshot.classroom_name
        return [item.student_id, position ? `${room} · ${position.row}排${position.column}座` : '未入座']
      }),
    )
  }
  const before = positionsByStudent(comparison.value)
  const after = positionsByStudent(plan.value)
  const students = new Map(
    [...comparison.value.snapshot.students, ...plan.value.snapshot.students].map(item => [item.id, item.name]),
  )
  return Array.from(students, ([id, name]) => ({
    id,
    name,
    before: before.get(id) ?? '未入座',
    after: after.get(id) ?? '未入座',
  })).filter(item => item.before !== item.after)
})

const comparedMovedIds = computed(() => new Set(comparisonChanges.value.map(item => item.id)))

defineExpose({
  isBusy: () => busy.value,
  hasUnsavedChanges: () => dirty.value || busy.value,
})

async function canLeave() {
  if (!dirty.value) return true
  return confirmAction('当前方案有未保存的调整，确定放弃修改吗？', {
    title: '未保存的方案',
    confirmText: '放弃修改',
    type: 'warning',
    danger: true,
  })
}

async function run(action) {
  if (busy.value) return
  busy.value = true
  try {
    await action()
  } catch (error) {
    notifyError(error)
  } finally {
    busy.value = false
  }
}

function inputOptions() {
  return {
    classId: Number(props.classId),
    classroomId: Number(roomId.value),
    examId: examId.value ? Number(examId.value) : null,
  }
}

async function inspectInputs() {
  inputWarnings.value = []
  if (!props.classId || !roomId.value) return
  try {
    inputWarnings.value = await invoke('inspect_plan_inputs', inputOptions())
  } catch (error) {
    inputWarnings.value = [String(error)]
  }
}

async function loadHistory() {
  history.value = props.classId ? await invoke('list_plans', { classId: Number(props.classId) }) : []
}

function setPlan(value, changed = false) {
  plan.value = value
  dirty.value = changed
  selected.value = null
  undoStack.value = []
  redoStack.value = []
  batchMode.value = false
  batchSelection.value = []
  pendingStudent.value = null
  comparison.value = null
  editingActive.value = false
  dragOverId.value = null
  planNameError.value = ''
  if (value) seed.value = value.seed
}

async function loadClass(autoOpen = true) {
  rooms.value = []
  exams.value = []
  history.value = []
  inputWarnings.value = []
  roomId.value = null
  examId.value = null
  setPlan(null)
  if (!props.classId) return
  const id = Number(props.classId)
  const [allRooms, allExams] = await Promise.all([
    invoke('list_records', { resource: 'classrooms', classId: id }),
    invoke('list_exams', { classId: id }),
  ])
  rooms.value = allRooms.filter(item => item.active)
  exams.value = allExams
  roomId.value = (rooms.value.find(room => room.is_default) ?? rooms.value[0])?.id ?? null
  examId.value = exams.value.find(exam => exam.is_current && exam.rank_grading)?.id ?? null
  await Promise.all([loadHistory(), inspectInputs()])

  if (autoOpen && history.value.length) {
    const preferred =
      history.value.find(item => item.confirmed && !item.archived) ??
      history.value.find(item => !item.confirmed) ??
      history.value[0]
    if (preferred) {
      setPlan(await invoke('open_plan', { id: preferred.id }))
    }
  }
}

async function openHistoryItem(item) {
  if (!item || item.id === plan.value?.id) return
  if (!(await canLeave())) return
  await run(async () => {
    setPlan(await invoke('open_plan', { id: item.id }))
  })
}

function loadComparison(id) {
  if (!id) {
    comparison.value = null
    return
  }
  return run(async () => {
    comparison.value = await invoke('open_plan', { id: Number(id) })
  })
}

function readSeed() {
  const value = Number(seed.value)
  if (seed.value === '' || !Number.isInteger(value) || value < 0 || value > 4294967295) {
    throw new Error('排座种子须为 0 至 4294967295 之间的整数')
  }
  return value
}

function nextRandomSeed() {
  seed.value = Math.floor(Math.random() * 900000) + 1
}

async function generate(manual = false) {
  if (!roomId.value) {
    notifyWarning('请先选择可用教室')
    return
  }
  if (!(await canLeave())) return
  await run(async () => {
    setPlan(await invoke(manual ? 'new_manual_plan' : 'generate_plan', { ...inputOptions(), seed: readSeed() }), true)
    generateDialog.value = false
    notifySuccess(manual ? '已创建空白手动草稿，可从右侧待入座池安排学生' : '已生成离线排座草稿')
  })
}

async function shuffleBatch() {
  if (!plan.value || !editable.value) return
  nextRandomSeed()
  await run(async () => {
    setPlan(await invoke('regenerate_plan', { plan: plan.value, seed: readSeed() }), true)
    notifySuccess('已换一批重排，锁定座位保持原位')
  })
}

async function regenerate() {
  await run(async () => {
    setPlan(await invoke('regenerate_plan', { plan: plan.value, seed: readSeed() }), true)
    generateDialog.value = false
    notifySuccess('已重新生成，锁定座位保持原位')
  })
}

async function prepareAi(keepLocks) {
  if (!keepLocks && !roomId.value) {
    notifyWarning('请先选择可用教室')
    return
  }
  if (!keepLocks && !(await canLeave())) return
  await run(async () => {
    const prior = keepLocks ? plan.value : null
    const inputs = prior
      ? { classId: prior.snapshot.class_id, classroomId: prior.snapshot.classroom_id, examId: prior.snapshot.exam_id }
      : inputOptions()
    const preview = await invoke('prepare_ai', {
      ...inputs,
      seed: readSeed(),
      prior,
      profileId: aiProfileId.value,
    })
    generateDialog.value = false
    aiFailure.value = ''
    autoFallback.value = false
    aiPreview.value = preview
  })
}

function clearAi() {
  aiPreview.value = null
  aiFailure.value = ''
}

async function cancelAi() {
  const id = aiPreview.value?.id
  clearAi()
  if (!id) return
  try {
    await invoke('cancel_ai', { id })
    notifyInfo('已取消 AI 排座')
  } catch (error) {
    notifyError(error)
  }
}

async function applyFallback(id) {
  const reason = aiFailure.value
  clearAi()
  setPlan(await invoke('fallback_ai', { id }), true)
  notifyWarning(reason ? `${reason}。已改用离线排座生成草稿` : '已改用离线排座生成草稿')
}

async function confirmAi() {
  const id = aiPreview.value?.id
  if (!id || aiRunning.value) return
  await run(async () => {
    aiRunning.value = true
    try {
      const result = await invoke('run_ai', { id })
      if (aiPreview.value?.id !== id) return
      if (result.status === 'success') {
        clearAi()
        setPlan(result.plan, true)
        notifySuccess('AI 方案已通过本地硬约束校验')
      } else if (result.status === 'failed') {
        aiFailure.value = result.reason
        if (autoFallback.value) await applyFallback(id)
      } else {
        clearAi()
        notifyInfo('已取消 AI 排座')
      }
    } catch (error) {
      if (aiPreview.value?.id === id) clearAi()
      throw error
    } finally {
      aiRunning.value = false
    }
  })
}

function manualFallback() {
  const id = aiPreview.value?.id
  if (id && aiFailure.value) return run(() => applyFallback(id))
}

onBeforeUnmount(() => {
  const id = aiPreview.value?.id
  if (id) void invoke('cancel_ai', { id }).catch(() => {})
})

async function adjust(adjustment) {
  await run(async () => {
    const previous = { ...plan.value }
    plan.value = await invoke('adjust_plan', { plan: previous, adjustment, editingActive: editingActive.value })
    undoStack.value.push(previous)
    redoStack.value = []
    selected.value = adjustment.to ?? adjustment.position ?? null
    dirty.value = true
  })
}

function restoreAdjustment(from, to) {
  const previous = from.pop()
  if (!previous) return
  to.push({ ...plan.value })
  plan.value = { ...previous, name: plan.value.name }
  selected.value = null
  batchSelection.value = []
  dirty.value = true
}

function chooseCell(cell) {
  const id = cell.position?.id
  if (busy.value || !editable.value || cell.position?.kind !== 'seat') return
  if (batchMode.value) {
    if (cell.assignment) {
      batchSelection.value = batchSelection.value.includes(id)
        ? batchSelection.value.filter(item => item !== id)
        : [...batchSelection.value, id]
    }
    return
  }
  if (pendingStudent.value && !cell.assignment) {
    adjust({ kind: 'place', student_id: pendingStudent.value, position: id })
    pendingStudent.value = null
    return
  }
  if (selected.value === id) {
    selected.value = null
    return
  }
  if (chosen.value?.assignment) {
    adjust({ kind: 'move', from: selected.value, to: id })
    return
  }
  if (cell.assignment) selected.value = id
}

function startDrag(event, cell) {
  if (!cell.assignment || cell.assignment.locked) return
  dragFrom = cell.position.id
  event.dataTransfer.effectAllowed = 'move'
  event.dataTransfer.setData('text/plain', String(dragFrom))
}

function startStudentDrag(event, student) {
  dragStudent = student.id
  event.dataTransfer.effectAllowed = 'move'
  event.dataTransfer.setData('text/plain', String(student.id))
}

function onDragOver(event, cell) {
  if (!editable.value || cell.position?.kind !== 'seat') return
  if (dragStudent != null && cell.assignment) return
  if (dragFrom != null && dragFrom === cell.position.id) return
  event.preventDefault()
  dragOverId.value = cell.position.id
}

function onDragLeave(cell) {
  if (dragOverId.value === cell.position?.id) dragOverId.value = null
}

function drop(event, cell) {
  event.preventDefault()
  dragOverId.value = null
  if (dragStudent != null && !cell.assignment && cell.position.kind === 'seat') {
    adjust({ kind: 'place', student_id: dragStudent, position: cell.position.id })
  }
  if (dragFrom != null && dragFrom !== cell.position.id && cell.position.kind === 'seat') {
    adjust({ kind: 'move', from: dragFrom, to: cell.position.id })
  }
  dragFrom = null
  dragStudent = null
}

function clearDragState() {
  dragFrom = null
  dragStudent = null
  dragOverId.value = null
}

const planNameError = ref('')

function validatePlanName() {
  const trimmed = (plan.value?.name ?? '').trim()
  if (!trimmed || trimmed.length > 100) {
    planNameError.value = '请填写 1 至 100 个字符的方案名称'
    notifyError(planNameError.value)
    return false
  }
  planNameError.value = ''
  return true
}

async function save(confirm) {
  if (!validatePlanName()) return
  if (confirm) {
    const ok = await confirmAction('确认后这份方案将设为本班「已启用」方案，原已启用方案会自动归档。确定继续吗？', {
      title: '确认启用方案',
      confirmText: '确认启用',
      type: 'info',
    })
    if (!ok) return
  }
  await run(async () => {
    setPlan(await invoke('save_plan', { plan: plan.value, confirm }))
    await loadHistory()
    notifySuccess(confirm ? '方案已确认启用' : '草稿已保存')
  })
}

async function activate() {
  const ok = await confirmAction('重新启用这份方案并归档同班当前方案？', {
    title: '重新启用方案',
    confirmText: '确认启用',
    type: 'info',
  })
  if (!ok) return
  await run(async () => {
    setPlan(await invoke('activate_plan', { id: plan.value.id }))
    await loadHistory()
    notifySuccess('方案已重新启用')
  })
}

async function saveActive() {
  if (!validatePlanName()) return
  const ok = await confirmAction('将当前调整直接保存到已启用的方案中？', {
    title: '保存启用方案调整',
    confirmText: '保存调整',
    type: 'warning',
  })
  if (!ok) return
  await run(async () => {
    setPlan(await invoke('save_active_plan', { plan: plan.value }))
    await loadHistory()
    notifySuccess('已保存当前启用方案的调整')
  })
}

async function cancelActiveEdit() {
  if (!(await canLeave())) return
  await run(async () => {
    setPlan(await invoke('open_plan', { id: plan.value.id }))
  })
}

async function copy() {
  await run(async () => {
    setPlan(await invoke('copy_plan', { id: plan.value.id }))
    await loadHistory()
    notifySuccess('已复制为可编辑草稿')
  })
}

async function exportFile(format) {
  if (!plan.value?.id || dirty.value) {
    notifyWarning('请先保存当前方案后再导出')
    return
  }
  await run(async () => {
    const options = { id: plan.value.id, teacher: teacher.value }
    let saved
    if (format === 'xlsx') {
      saved = await invoke('export_plan_workbook', options)
    } else {
      const sheet = await invoke('plan_export_sheet', options)
      saved = await invoke('export_plan_image', await renderPlan(sheet, format))
    }
    if (saved) notifySuccess(`座位表已导出（${teacher.value ? '教师版' : '学生版'}）`)
  })
}

function studentScoreBadge(student) {
  if (!student) return ''
  if (student.score_level != null && scoreLevelLabels[student.score_level]) return scoreLevelLabels[student.score_level]
  if (student.score_rank != null) return `#${student.score_rank}`
  return ''
}

function formatTime(value) {
  return value ? String(value).replace('T', ' ').slice(0, 16) : ''
}

function cellLabel(cell) {
  const label = `第 ${cell.row} 排第 ${cell.column} 座`
  const content = cell.student?.name ?? ({ seat: '空座', aisle: '过道', unavailable: '不可用' }[cell.position?.kind] ?? '未设置')
  return label + '，' + content + (cell.assignment?.locked ? '（锁定）' : '')
}

watch(
  () => props.classId,
  () => {
    void run(() => loadClass(true))
  },
)

onMounted(() =>
  run(async () => {
    await loadClass(true)
    try {
      const view = await invoke('list_ai_profiles')
      aiProfiles.value = view.profiles.filter(item => item.profile.active).map(item => item.profile)
      aiProfileId.value = view.default_id
    } catch {
      aiProfiles.value = []
    }
  }),
)
</script>

<template>
  <div class="seating-page">
    <div v-if="!classId" class="page-container">
      <div class="page-card">
        <ElEmpty description="请先在基础资料中创建班级，或在顶部选择一个班级" />
      </div>
    </div>

    <template v-else>
      <div class="seating-toolbar">
        <div class="toolbar-left">
          <template v-if="plan">
            <div v-if="editable" class="plan-name-wrap">
              <span class="form-label is-required">方案名称</span>
              <ElInput
                :model-value="plan.name"
                maxlength="100"
                :disabled="busy"
                placeholder="请输入方案名称（必填）"
                :class="{ 'is-field-error': !!planNameError }"
                style="width: 220px"
                @update:model-value="plan.name = $event; dirty = true; if ($event.trim()) planNameError = ''"
              />
            </div>
            <strong v-else class="plan-title">{{ plan.name }}</strong>
            <ElTag :type="planStatusTagType(plan)" effect="dark" size="small">{{ planStatus(plan) }}</ElTag>
            <ElTag :type="plan.source === 'ai' ? 'primary' : 'info'" effect="plain" size="small">
              {{ plan.source === 'ai' ? 'AI 排座' : '离线排座' }}
            </ElTag>
            <ElTag v-if="dirty" type="warning" effect="light" size="small">未保存</ElTag>
            <span class="plan-meta">{{ plan.snapshot.classroom_name }}</span>
          </template>
          <span v-else class="plan-meta">当前班级暂无已打开的方案</span>
        </div>

        <div class="toolbar-right">
          <ElButton :icon="Plus" :disabled="busy || !rooms.length" @click="generateDialog = true">新建方案</ElButton>
          <template v-if="plan">
            <template v-if="editable && !editingActive">
              <ElTooltip content="随机更换排座种子重新生成，保持已锁定的座位不动" placement="bottom">
                <ElButton :icon="Refresh" :loading="busy" @click="shuffleBatch">换一批</ElButton>
              </ElTooltip>
              <ElButton :icon="MagicStick" :disabled="busy" @click="prepareAi(true)">AI 重排</ElButton>
              <ElButton :loading="busy" @click="save(false)">保存草稿</ElButton>
              <ElButton type="primary" :loading="busy" :disabled="!plan.assignments.length" @click="save(true)">
                确认启用
              </ElButton>
            </template>
            <template v-else-if="editingActive">
              <ElButton :disabled="busy" @click="cancelActiveEdit">取消编辑</ElButton>
              <ElButton type="primary" :loading="busy" :disabled="!dirty || !plan.assignments.length" @click="saveActive">
                保存调整
              </ElButton>
            </template>
            <template v-else>
              <ElButton v-if="plan.confirmed && !plan.archived" type="primary" :disabled="busy" @click="editingActive = true">
                编辑已启用方案
              </ElButton>
              <ElButton v-if="plan.archived" type="primary" :loading="busy" @click="activate">重新启用</ElButton>
              <ElButton v-if="plan.id" :icon="CopyDocument" :disabled="busy || dirty" @click="copy">复制为草稿</ElButton>
            </template>

            <ElDropdown trigger="click" :disabled="busy || dirty || !plan.id" @command="exportFile">
              <ElButton :icon="Download" :disabled="busy || dirty || !plan.id">
                导出<ElIcon class="el-icon--right"><ArrowDown /></ElIcon>
              </ElButton>
              <template #dropdown>
                <ElDropdownMenu>
                  <ElDropdownItem command="png">导出 PNG 图片（{{ teacher ? '教师版' : '学生版' }}）</ElDropdownItem>
                  <ElDropdownItem command="xlsx">导出 Excel 表格（{{ teacher ? '教师版' : '学生版' }}）</ElDropdownItem>
                  <ElDropdownItem command="pdf">导出横向 A4 PDF（{{ teacher ? '教师版' : '学生版' }}）</ElDropdownItem>
                </ElDropdownMenu>
              </template>
            </ElDropdown>
          </template>
          <ElButton text @click="inspectorOpen = !inspectorOpen">
            {{ inspectorOpen ? '收起侧栏' : '展开侧栏' }}
          </ElButton>
        </div>
      </div>

      <div class="seating-workspace" :class="{ 'no-inspector': !inspectorOpen }">
        <div class="canvas-pane">
          <div v-if="!plan" class="empty-canvas page-card">
            <ElEmpty :description="rooms.length ? '当前班级还没有排座方案，点击下方按钮立即生成' : '当前班级暂无可用教室，请先在「基础资料 → 教室管理」配置布局'">
              <div v-if="rooms.length" class="empty-actions">
                <ElButton type="primary" size="large" :loading="busy" @click="generate(false)">一键离线排座</ElButton>
                <ElButton size="large" :disabled="busy" @click="generate(true)">空白手动排座</ElButton>
                <ElButton size="large" :disabled="busy" @click="generateDialog = true">更多排座选项</ElButton>
              </div>
            </ElEmpty>
            <ElAlert
              v-if="inputWarnings.length"
              type="warning"
              show-icon
              :closable="false"
              title="生成前数据检查提醒"
              style="margin-top: 16px; max-width: 560px"
            >
              <ul class="warn-list">
                <li v-for="warning in inputWarnings" :key="warning">{{ warning }}</li>
              </ul>
            </ElAlert>
          </div>

          <template v-else>
            <div class="canvas-subbar">
              <div class="subbar-left">
                <template v-if="editable">
                  <ElTooltip content="撤销上一步调座" placement="top">
                    <ElButton size="small" :icon="Back" :disabled="busy || !undoStack.length" @click="restoreAdjustment(undoStack, redoStack)">
                      撤销
                    </ElButton>
                  </ElTooltip>
                  <ElTooltip content="重做" placement="top">
                    <ElButton size="small" :icon="Right" :disabled="busy || !redoStack.length" @click="restoreAdjustment(redoStack, undoStack)">
                      重做
                    </ElButton>
                  </ElTooltip>
                  <ElButton
                    size="small"
                    :type="batchMode ? 'primary' : 'default'"
                    :plain="!batchMode"
                    :disabled="busy"
                    @click="batchMode = !batchMode; batchSelection = []; selected = null"
                  >
                    {{ batchMode ? `退出多选 (${batchSelection.length})` : '多选锁座' }}
                  </ElButton>
                  <template v-if="batchMode">
                    <ElButton size="small" :icon="Lock" :disabled="busy || !batchSelection.length" @click="adjust({ kind: 'batch_lock', positions: batchSelection, locked: true })">
                      批量锁定
                    </ElButton>
                    <ElButton size="small" :icon="Unlock" :disabled="busy || !batchSelection.length" @click="adjust({ kind: 'batch_lock', positions: batchSelection, locked: false })">
                      批量解锁
                    </ElButton>
                  </template>
                </template>
                <ElInput
                  v-model="findQuery"
                  size="small"
                  clearable
                  placeholder="快速找人（姓名／学号）"
                  :prefix-icon="Search"
                  style="width: 185px"
                />
                <ElTag v-if="findQuery.trim()" size="small" :type="matchedStudentIds.size ? 'success' : 'info'">
                  命中 {{ matchedStudentIds.size }} 人
                </ElTag>
              </div>

              <div class="subbar-right">
                <ElPopover placement="bottom-end" :width="210" trigger="click">
                  <template #reference>
                    <ElButton size="small" :icon="Setting">座位标签</ElButton>
                  </template>
                  <div class="badge-popover">
                    <p class="form-hint" style="margin-bottom: 8px">在座位卡上显示调座参考特征：</p>
                    <ElCheckboxGroup v-model="showBadges" class="badge-checks">
                      <ElCheckbox value="gender">性别（男／女）</ElCheckbox>
                      <ElCheckbox value="vision">视力关注（近视）</ElCheckbox>
                      <ElCheckbox value="score">成绩等级／排名</ElCheckbox>
                      <ElCheckbox value="discipline">纪律关注</ElCheckbox>
                    </ElCheckboxGroup>
                  </div>
                </ElPopover>
                <ElTooltip content="导出座位表时同步采用当前选择的视角" placement="top">
                  <ElRadioGroup v-model="teacher" size="small">
                    <ElRadioButton :value="false">学生视角</ElRadioButton>
                    <ElRadioButton :value="true">教师视角</ElRadioButton>
                  </ElRadioGroup>
                </ElTooltip>
              </div>
            </div>

            <div v-if="editable && chosen?.assignment" class="seat-context-bar">
              <span>
                已选中：<strong>{{ chosen.student?.name }}</strong>（第 {{ chosen.row }} 排第 {{ chosen.column }} 座）
                <span class="form-hint">· 点击另一座位可移座或互换</span>
              </span>
              <div class="action-group">
                <ElButton
                  size="small"
                  :icon="chosen.assignment.locked ? Unlock : Lock"
                  :disabled="busy"
                  @click="adjust({ kind: 'lock', position: selected })"
                >
                  {{ chosen.assignment.locked ? '解除锁定' : '锁定座位' }}
                </ElButton>
                <ElButton
                  size="small"
                  type="danger"
                  plain
                  :disabled="busy || chosen.assignment.locked"
                  @click="adjust({ kind: 'clear', position: selected })"
                >
                  移回待入座
                </ElButton>
                <ElButton size="small" text @click="selected = null">取消选中</ElButton>
              </div>
            </div>

            <ElAlert
              v-if="plan.snapshot.warnings.length"
              type="warning"
              show-icon
              :closable="false"
              style="margin-bottom: 10px"
            >
              <template #title>
                快照规则提示（{{ plan.snapshot.warnings.length }} 条）：{{ plan.snapshot.warnings.join('；') }}
              </template>
            </ElAlert>

            <div class="canvas-board page-card">
              <div v-if="plan.snapshot.podium_front" class="podium">讲 台</div>
              <div class="layout-scroll seating-scroll" aria-label="排座网格">
                <div class="layout-body">
                  <div v-if="plan.snapshot.podium_side === 'left'" class="podium podium-side">讲台</div>
                  <div
                    class="seat-grid"
                    :style="{ gridTemplateColumns: `repeat(${plan.snapshot.columns}, minmax(104px, 1fr))` }"
                  >
                    <template v-for="cell in grid" :key="`${cell.row}:${cell.column}`">
                      <button
                        v-if="cell.position?.kind === 'seat'"
                        type="button"
                        class="seat-card"
                        :class="{
                          occupied: !!cell.student,
                          vacant: !cell.student,
                          locked: !!cell.assignment?.locked,
                          chosen: selected === cell.position.id || batchSelection.includes(cell.position.id),
                          'drag-over': dragOverId === cell.position.id,
                          highlighted: !!cell.student && matchedStudentIds.has(cell.student.id),
                          moved: !!cell.student && comparedMovedIds.has(cell.student.id),
                        }"
                        :aria-label="cellLabel(cell)"
                        :disabled="busy || !editable"
                        :draggable="!busy && editable && !!cell.assignment && !cell.assignment.locked"
                        @click="chooseCell(cell)"
                        @dragstart="startDrag($event, cell)"
                        @dragenter.prevent="onDragOver($event, cell)"
                        @dragover="onDragOver($event, cell)"
                        @dragleave="onDragLeave(cell)"
                        @drop="drop($event, cell)"
                        @dragend="clearDragState"
                      >
                        <div class="seat-card-head">
                          <span class="seat-pos">{{ cell.row }}-{{ cell.column }}</span>
                          <span v-if="cell.assignment?.locked" class="lock-pill" title="已锁定">🔒</span>
                        </div>
                        <div class="seat-student-name">
                          {{ cell.student?.name ?? '空座' }}
                        </div>
                        <div v-if="teacher && cell.student" class="seat-student-no">{{ cell.student.number }}</div>
                        <div v-if="cell.student && showBadges.length" class="seat-badges">
                          <span
                            v-if="showBadges.includes('gender') && cell.student.gender != null"
                            class="micro-badge"
                            :class="cell.student.gender ? 'badge-boy' : 'badge-girl'"
                          >
                            {{ cell.student.gender ? '男' : '女' }}
                          </span>
                          <span v-if="showBadges.includes('vision') && cell.student.poor_vision" class="micro-badge badge-vision">
                            近视
                          </span>
                          <span
                            v-if="showBadges.includes('score') && studentScoreBadge(cell.student)"
                            class="micro-badge badge-score"
                          >
                            {{ studentScoreBadge(cell.student) }}
                          </span>
                          <span
                            v-if="showBadges.includes('discipline') && cell.student.discipline_attention"
                            class="micro-badge badge-discipline"
                          >
                            关注
                          </span>
                        </div>
                      </button>
                      <div v-else-if="cell.position" class="seat-nonseat" :class="cell.position.kind">
                        {{ cell.position.kind === 'aisle' ? '过道' : '不可用' }}
                      </div>
                      <div v-else class="seat-nonseat empty-slot" aria-hidden="true"></div>
                    </template>
                  </div>
                  <div v-if="plan.snapshot.podium_side === 'right'" class="podium podium-side">讲台</div>
                </div>
              </div>
              <div v-if="!plan.snapshot.podium_front && !plan.snapshot.podium_side" class="podium">讲 台</div>
            </div>
          </template>
        </div>

        <aside v-if="inspectorOpen" class="inspector-pane page-card">
          <ElTabs v-model="inspectorTab" class="inspector-tabs">
            <ElTabPane label="评分与待入座" name="score">
              <div v-if="plan && editable && unseated.length" class="unseated-box">
                <div class="box-title">
                  <strong>待入座学生</strong>
                  <ElTag type="warning" size="small" round>{{ unseated.length }} 人</ElTag>
                </div>
                <p class="form-hint">拖动或先点击学生再点击空座即可入座。</p>
                <div class="unseated-chips">
                  <button
                    v-for="student in unseated"
                    :key="student.id"
                    type="button"
                    class="student-chip"
                    :class="{ active: pendingStudent === student.id }"
                    :disabled="busy || batchMode"
                    draggable="true"
                    @click="pendingStudent = pendingStudent === student.id ? null : student.id; selected = null"
                    @dragstart="startStudentDrag($event, student)"
                    @dragend="clearDragState"
                  >
                    {{ student.name }}
                  </button>
                </div>
              </div>

              <template v-if="plan">
                <div class="score-summary">
                  <div class="score-head">
                    <span>综合规则得分</span>
                    <strong v-if="maximum">{{ total }} / {{ maximum }}</strong>
                    <span v-else class="form-hint">暂无软规则评分</span>
                  </div>
                  <ElProgress v-if="maximum" :percentage="scorePercent" :stroke-width="8" />
                </div>

                <div v-if="plan.ratings.length" class="rating-list">
                  <div v-for="(rating, index) in plan.ratings" :key="index" class="rating-row">
                    <div class="rating-label">
                      <span>{{ rating.label }}</span>
                      <strong>{{ rating.maximum ? `${rating.score}/${rating.maximum}` : '无数据' }}</strong>
                    </div>
                    <ElProgress
                      v-if="rating.maximum"
                      :percentage="Math.round((rating.score / rating.maximum) * 100)"
                      :show-text="false"
                      :stroke-width="5"
                    />
                  </div>
                </div>
                <ElEmpty v-else :image-size="56" description="当前方案无软规则评分项" />
              </template>
              <ElEmpty v-else :image-size="60" description="打开或生成方案后查看实时评分" />
            </ElTabPane>

            <ElTabPane :label="`方案历史 (${history.length})`" name="history">
              <div class="history-filters">
                <ElInput v-model="historySearch" size="small" clearable placeholder="搜索方案名称" :prefix-icon="Search" />
                <ElRadioGroup v-model="historyStatus" size="small">
                  <ElRadioButton value="">全部</ElRadioButton>
                  <ElRadioButton value="已启用">已启用</ElRadioButton>
                  <ElRadioButton value="草稿">草稿</ElRadioButton>
                  <ElRadioButton value="已归档">归档</ElRadioButton>
                </ElRadioGroup>
              </div>
              <div v-if="filteredHistory.length" class="history-list">
                <button
                  v-for="item in filteredHistory"
                  :key="item.id"
                  type="button"
                  class="history-card"
                  :class="{ current: plan?.id === item.id }"
                  :disabled="busy"
                  @click="openHistoryItem(item)"
                >
                  <div class="history-card-top">
                    <strong class="history-name">{{ item.name }}</strong>
                    <ElTag :type="planStatusTagType(item)" size="small" effect="light">{{ planStatus(item) }}</ElTag>
                  </div>
                  <div class="history-card-meta">
                    <ElTag size="small" effect="plain" :type="item.source === 'ai' ? 'primary' : 'info'">
                      {{ item.source === 'ai' ? 'AI' : '离线' }}
                    </ElTag>
                    <span>{{ formatTime(item.updated_at) }}</span>
                  </div>
                </button>
              </div>
              <ElEmpty v-else :image-size="56" description="暂无符合条件的历史方案" />
            </ElTabPane>

            <ElTabPane label="历史对比" name="compare">
              <template v-if="plan && history.some(item => item.id !== plan.id)">
                <ElSelect
                  :model-value="comparison?.id ?? null"
                  clearable
                  placeholder="选择要对比的历史方案"
                  style="width: 100%; margin-bottom: 12px"
                  :disabled="busy"
                  @change="loadComparison"
                >
                  <ElOption
                    v-for="item in history.filter(entry => entry.id !== plan.id)"
                    :key="item.id"
                    :label="`${item.name}（${planStatus(item)}）`"
                    :value="item.id"
                  />
                </ElSelect>
                <template v-if="comparison">
                  <p class="form-hint" style="margin-bottom: 10px">
                    与「{{ comparison.name }}」相比，共 <strong>{{ comparisonChanges.length }}</strong> 名学生座位变动（左侧已用蓝边标出）。
                  </p>
                  <ElTable v-if="comparisonChanges.length" :data="comparisonChanges" size="small" max-height="420">
                    <ElTableColumn prop="name" label="学生" width="76" />
                    <ElTableColumn prop="before" label="原座位" />
                    <ElTableColumn prop="after" label="当前座位" />
                  </ElTable>
                  <ElEmpty v-else :image-size="48" description="两份方案座位完全一致" />
                </template>
              </template>
              <ElEmpty v-else :image-size="56" description="需要至少两份已保存方案才能对比" />
            </ElTabPane>
          </ElTabs>
        </aside>
      </div>
    </template>

    <!-- 新建／生成方案弹窗 -->
    <ElDialog v-model="generateDialog" title="生成或创建排座方案" width="min(92vw, 560px)" destroy-on-close>
      <div class="drawer-form">
        <p class="required-legend"><span>*</span> 为必填项</p>
        <div class="form-field" :class="{ 'is-error': !roomId }">
          <span class="form-label is-required">目标教室</span>
          <ElSelect v-model="roomId" placeholder="请选择目标教室" :disabled="busy" @change="run(inspectInputs)">
            <ElOption v-for="room in rooms" :key="room.id" :label="`${room.name}${room.is_default ? '（默认）' : ''}`" :value="room.id" />
          </ElSelect>
          <p v-if="!roomId" class="form-error">请选择目标教室</p>
        </div>
        <div class="form-field">
          <span class="form-label">成绩来源</span>
          <ElSelect v-model="examId" clearable placeholder="学生档案中的成绩等级（默认）" :disabled="busy" @change="run(inspectInputs)">
            <ElOption label="学生档案中的成绩等级（默认）" :value="null" />
            <ElOption
              v-for="exam in exams"
              :key="exam.id"
              :label="`${exam.name}${exam.rank_grading ? '（使用档案等级）' : '（沿用旧版总排名）'}${exam.is_current ? '（当前）' : ''}`"
              :value="exam.id"
            />
          </ElSelect>
        </div>
        <div v-if="aiProfiles.length" class="form-field">
          <span class="form-label">AI 模型（使用 AI 排座时）</span>
          <ElSelect v-model="aiProfileId" :disabled="busy">
            <ElOption
              v-for="item in aiProfiles"
              :key="item.id"
              :label="`${item.name} · ${item.settings.model}`"
              :value="item.id"
            />
          </ElSelect>
        </div>

        <ElCollapse v-model="advancedSeedOpen">
          <ElCollapseItem title="高级选项：排座随机种子" name="seed">
            <div class="seed-row">
              <ElInputNumber v-model="seed" :min="0" :max="4294967295" :disabled="busy" controls-position="right" style="flex: 1" />
              <ElButton :disabled="busy" @click="nextRandomSeed">随机种子</ElButton>
            </div>
            <p class="form-hint" style="margin-top: 6px">相同资料和种子下，离线引擎会复现完全一致的座位表。</p>
          </ElCollapseItem>
        </ElCollapse>

        <ElAlert v-if="inputWarnings.length" type="warning" show-icon :closable="false" title="生成前数据提示">
          <ul class="warn-list">
            <li v-for="warning in inputWarnings" :key="warning">{{ warning }}</li>
          </ul>
        </ElAlert>
      </div>
      <template #footer>
        <div class="generate-footer">
          <ElButton :disabled="busy || !roomId" @click="generate(true)">空白手动创建</ElButton>
          <ElButton :icon="MagicStick" :disabled="busy || !roomId" @click="prepareAi(false)">AI 排座</ElButton>
          <ElButton v-if="plan && editable && !editingActive" :disabled="busy" @click="regenerate">保留锁座重排</ElButton>
          <ElButton type="primary" :loading="busy" :disabled="!roomId" @click="generate(false)">离线生成新方案</ElButton>
        </div>
      </template>
    </ElDialog>

    <!-- AI 脱敏发送范围确认弹窗 -->
    <ElDialog
      :model-value="!!aiPreview"
      title="确认 AI 发送范围"
      width="min(92vw, 620px)"
      :close-on-click-modal="false"
      :before-close="() => cancelAi()"
    >
      <div v-if="aiPreview" class="drawer-form">
        <div class="ai-meta-box">
          <div><strong>服务地址：</strong><code>{{ aiPreview.endpoint }}</code></div>
          <div><strong>模型名称：</strong><code>{{ aiPreview.model }}</code></div>
        </div>
        <p class="form-hint" style="font-size: 13px; color: #314b43">
          本次仅发送 <strong>{{ aiPreview.payload.students.length }}</strong> 个一次性学生编号、<strong>{{ aiPreview.payload.seats.length }}</strong> 个座位，以及讲台方向、相邻、同桌、锁座和前排等必要硬约束。姓名、学号与成绩均保留在本机。
        </p>
        <details class="payload-details">
          <summary>查看实际发送 JSON 字段</summary>
          <pre>{{ JSON.stringify(aiPreview.payload, null, 2) }}</pre>
        </details>
        <ElAlert v-if="aiRunning" type="info" show-icon :closable="false" title="正在等待 AI 返回排座建议，当前本地方案仍完整保留…" />
        <ElAlert v-else-if="aiFailure" type="error" show-icon :closable="false" :title="`${aiFailure}。可点击下方改用离线排座。`" />
        <ElCheckbox v-else v-model="autoFallback">AI 失败或校验未通过时自动改用离线排座</ElCheckbox>
      </div>
      <template #footer>
        <div class="drawer-footer">
          <ElButton @click="cancelAi">{{ aiRunning ? '取消 AI 调用' : '取消' }}</ElButton>
          <ElButton v-if="aiFailure" type="primary" :loading="busy" @click="manualFallback">改用离线排座</ElButton>
          <ElButton v-if="!aiRunning && !aiFailure" type="primary" :loading="busy" @click="confirmAi">确认发送</ElButton>
        </div>
      </template>
    </ElDialog>
  </div>
</template>

<style scoped>
.seating-page {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

.seating-toolbar {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 10px 14px;
  padding: 12px 24px;
  background: #ffffff;
  border-bottom: 1px solid var(--brand-border);
}

.toolbar-left,
.toolbar-right {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.plan-title {
  font-size: 15px;
  color: #1f3c35;
}

.plan-name-wrap {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.plan-meta {
  font-size: 12px;
  color: #6a8078;
}

.seating-workspace {
  flex: 1;
  min-height: 0;
  display: grid;
  grid-template-columns: minmax(0, 1fr) 316px;
  gap: 16px;
  padding: 16px 24px 24px;
}

.seating-workspace.no-inspector {
  grid-template-columns: minmax(0, 1fr);
}

.canvas-pane {
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.empty-canvas {
  display: grid;
  place-items: center;
  min-height: 360px;
}

.empty-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 10px;
}

.canvas-subbar {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 8px 12px;
  margin-bottom: 10px;
}

.subbar-left,
.subbar-right {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.seat-context-bar {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  padding: 8px 14px;
  margin-bottom: 10px;
  background: #ebf6f0;
  border: 1px solid #c5e2d5;
  border-radius: 10px;
  font-size: 13px;
  color: #1e5444;
}

.canvas-board {
  padding: 14px 16px;
}

.seating-scroll {
  max-height: calc(100vh - 280px);
}

.seat-card {
  display: flex;
  flex-direction: column;
  justify-content: space-between;
  gap: 4px;
  min-height: 78px;
  padding: 7px 8px;
  border: 1px solid #cfe1d8;
  border-radius: 9px;
  background: #ffffff;
  color: #1f3933;
  font: inherit;
  text-align: left;
  cursor: pointer;
  transition: transform 0.12s ease, border-color 0.14s ease, box-shadow 0.14s ease, background-color 0.14s ease;
}

.seat-card.occupied {
  background: #ffffff;
  border-color: #c4ddd2;
  border-left: 3px solid #398771;
}

.seat-card.occupied:hover:not(:disabled) {
  border-color: #398771;
  box-shadow: 0 4px 12px rgba(57, 135, 113, 0.14);
}

.seat-card.vacant {
  background: #f8fbfa;
  border: 1px dashed #cfdcd6;
  color: #8a9f97;
}

.seat-card.vacant:hover:not(:disabled) {
  border-color: #398771;
  background: #eff8f4;
}

.seat-card.locked {
  background: #fffbeb;
  border-color: #e8c775;
  border-left: 3px solid #d9822b;
}

.seat-card.chosen {
  outline: 2px solid #398771;
  outline-offset: 1px;
  box-shadow: 0 0 0 4px rgba(57, 135, 113, 0.16);
}

.seat-card.drag-over {
  border: 2px dashed #398771 !important;
  background: #e4f5ec !important;
  transform: scale(1.03);
}

.seat-card.highlighted {
  box-shadow: 0 0 0 3px #e6a23c;
  background: #fef8e8;
}

.seat-card.moved {
  box-shadow: inset 0 0 0 1.5px #409eff;
}

.seat-card:disabled {
  opacity: 1;
  cursor: default;
}

.seat-card-head {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 10px;
  color: #7c9189;
}

.lock-pill {
  font-size: 11px;
  line-height: 1;
}

.seat-student-name {
  font-size: 14px;
  font-weight: 700;
  text-align: center;
  line-height: 1.25;
  word-break: break-all;
}

.seat-card.vacant .seat-student-name {
  font-size: 12px;
  font-weight: 500;
}

.seat-student-no {
  font-size: 10px;
  color: #6b827a;
  text-align: center;
}

.seat-badges {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 3px;
}

.micro-badge {
  display: inline-block;
  padding: 1px 5px;
  border-radius: 4px;
  font-size: 10px;
  font-weight: 600;
  line-height: 1.35;
}

.badge-boy {
  background: #e8f3ff;
  color: #2c6db5;
}

.badge-girl {
  background: #fceef5;
  color: #b84a7d;
}

.badge-vision {
  background: #fff3e0;
  color: #b8670a;
}

.badge-score {
  background: #e7f6ee;
  color: #206b54;
}

.badge-discipline {
  background: #fdecec;
  color: #c24141;
}

.seat-nonseat {
  display: grid;
  place-items: center;
  min-height: 78px;
  border-radius: 9px;
  font-size: 12px;
}

.seat-nonseat.aisle {
  background: #f1f4f6;
  border: 1px dashed #d6dfe4;
  color: #798c94;
}

.seat-nonseat.unavailable {
  background: #fdf2f2;
  border: 1px solid #f0cccc;
  color: #b85c5c;
}

.seat-nonseat.empty-slot {
  background: transparent;
}

.inspector-pane {
  padding: 12px 14px;
  overflow: auto;
  max-height: calc(100vh - 150px);
}

.unseated-box {
  padding: 10px 12px;
  margin-bottom: 14px;
  background: #fff9ef;
  border: 1px solid #f2dfb8;
  border-radius: 10px;
}

.box-title {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 4px;
  font-size: 13px;
}

.unseated-chips {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 8px;
}

.student-chip {
  padding: 5px 10px;
  border: 1px solid #d5e2dc;
  border-radius: 999px;
  background: #ffffff;
  color: #23433a;
  font: inherit;
  font-size: 12px;
  cursor: grab;
}

.student-chip.active {
  background: #398771;
  border-color: #398771;
  color: #ffffff;
}

.score-summary {
  padding: 12px;
  background: #f6faf8;
  border: 1px solid #e4eeea;
  border-radius: 10px;
  margin-bottom: 12px;
}

.score-head {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  margin-bottom: 8px;
  font-size: 13px;
}

.score-head strong {
  font-size: 18px;
  color: #1f5c4a;
}

.rating-list {
  display: grid;
  gap: 10px;
}

.rating-row {
  display: grid;
  gap: 4px;
}

.rating-label {
  display: flex;
  justify-content: space-between;
  font-size: 12px;
  color: #465f57;
}

.history-filters {
  display: grid;
  gap: 8px;
  margin-bottom: 12px;
}

.history-list {
  display: grid;
  gap: 8px;
}

.history-card {
  width: 100%;
  display: grid;
  gap: 6px;
  padding: 10px 12px;
  border: 1px solid #e4ece9;
  border-radius: 9px;
  background: #ffffff;
  text-align: left;
  font: inherit;
  cursor: pointer;
  transition: border-color 0.14s ease, background-color 0.14s ease;
}

.history-card:hover {
  border-color: #a9d1c2;
  background: #f8fcfa;
}

.history-card.current {
  border-color: #398771;
  background: #edf7f2;
}

.history-card-top {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 6px;
}

.history-name {
  font-size: 13px;
  color: #1f3933;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.history-card-meta {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: 11px;
  color: #778c85;
}

.badge-checks {
  display: grid;
  gap: 6px;
}

.seed-row {
  display: flex;
  gap: 8px;
}

.warn-list {
  margin: 4px 0 0;
  padding-left: 18px;
  font-size: 12px;
}

.generate-footer {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 8px;
}

.ai-meta-box {
  padding: 10px 12px;
  background: #f6faf8;
  border: 1px solid #e3ece8;
  border-radius: 8px;
  display: grid;
  gap: 4px;
  font-size: 12px;
  word-break: break-all;
}

.payload-details pre {
  max-height: 200px;
  overflow: auto;
  padding: 10px;
  margin-top: 8px;
  background: #f3f7f5;
  border-radius: 8px;
  font-size: 12px;
}

@media (max-width: 1060px) {
  .seating-workspace {
    grid-template-columns: minmax(0, 1fr);
    padding: 14px 16px 24px;
  }
  .inspector-pane {
    max-height: none;
  }
  .seating-toolbar {
    padding: 10px 16px;
  }
}
</style>
