<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, toRef, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  ElAlert,
  ElButton,
  ElCheckbox,
  ElDialog,
  ElDrawer,
  ElEmpty,
  ElInput,
  ElOption,
  ElSelect,
  ElTable,
  ElTableColumn,
  ElTag,
} from 'element-plus'
import { CopyDocument, Download, Plus, Refresh, Search, Upload } from '@element-plus/icons-vue'
import LayoutEditor from './LayoutEditor.vue'
import { classGrades, classSubjects, nextSchoolTerm, splitSubjects, termLabel, terms } from './classOptions.js'
import { downloadWorkbook, workbookBytes } from './workbook.js'
import { confirmAction, notifyError, notifyInfo, notifySuccess, notifyWarning } from './ui.js'

const props = defineProps({
  section: { type: String, required: true },
  classId: { type: Number, default: null },
  classes: { type: Array, default: () => [] },
  openImport: Boolean,
})
const emit = defineEmits(['classes-changed', 'open-import-handled'])

const section = toRef(props, 'section')
const records = ref([])
const statusFilter = ref('all')
const search = ref('')
const filters = ref({})
const selection = ref([])
const tableRef = ref(null)
const studentFileInput = ref(null)
const studentPreview = ref(null)
const studentImport = ref(false)
const updateStudents = ref(false)
const students = ref([])
const classrooms = ref([])
const form = ref(null)
const originalRecord = ref(null)
const originalClassroom = ref(null)
const copying = ref(false)
const copyOptions = ref(null)
const loading = ref(false)
const loadError = ref('')
const saving = ref(false)
const inlineLayout = ref(null)
const editorRevision = ref(0)
const ruleType = ref('')
const ruleStudents = ref([])
const ruleMaxRow = ref(1)

// Folded "座位明细" (positions) drawer inside 教室管理
const positionsDrawerClassroom = ref(null)
const classroomPositions = ref([])
const positionsStatusFilter = ref('all')
const positionSelection = ref([])
const positionForm = ref(null)
const originalPosition = ref(null)
const formErrors = ref({})
const copyErrors = ref({})
const positionErrors = ref({})

function clearError(bag, key) {
  if (bag.value[key]) {
    const next = { ...bag.value }
    delete next[key]
    bag.value = next
  }
}

let initialDraft = ''
let loadVersion = 0

defineExpose({
  isBusy,
  hasUnsavedChanges: () =>
    isBusy() ||
    hasUnsavedDraft() ||
    !!studentPreview.value ||
    !!positionForm.value ||
    inlineLayout.value?.hasUnsavedChanges(),
})

const text = (key, label, initial = '', meta = {}) => ({ key, label, initial, type: 'text', ...meta })
const number = (key, label, initial = 1, meta = {}) => ({ key, label, initial, type: 'number', ...meta })
const choice = (key, label, options, initial, meta = {}) => ({ key, label, initial: initial ?? options[0]?.[0] ?? null, options, ...meta })
const active = choice('active', '状态', [[1, '正常'], [0, '停用']])
const labels = values => values.map(value => [value, value || '未填写'])
const yesNo = [[0, '否'], [1, '是']]

const subjectChoices = computed(() => [...new Set([...classSubjects, ...splitSubjects(originalRecord.value?.subjects)])])
const selectedSubjects = computed({
  get: () => splitSubjects(form.value?.subjects),
  set: subjects => {
    form.value.subjects = subjects.join(',')
    clearError(formErrors, 'subjects')
  },
})
const legacyGrade = computed(() => {
  const grade = originalRecord.value?.grade
  return grade && !classGrades.some(option => option.name === grade) ? grade : ''
})

const schemas = {
  classes: {
    label: '班级',
    fields: [
      text('name', '班级名称', '', { required: true, placeholder: '请输入班级名称，例如：一年级（1）班' }),
      text('grade', '年级', '', { required: true, placeholder: '请选择年级' }),
      text('school_stage', '学段', '', { note: '随年级自动带出', placeholder: '选择年级后自动带出' }),
      text('school_year', '学年', '', { placeholder: '选填，例如：2026-2027' }),
      text('term', '学期', terms[0], { required: true, placeholder: '请选择学期' }),
      text('subjects', '班级科目', '', { required: true, placeholder: '请至少选择一门班级科目' }),
      active,
      text('remark', '备注', '', { placeholder: '选填' }),
    ],
  },
  students: {
    label: '学生',
    fields: [
      text('student_no', '学号', '', { required: true, placeholder: '请输入学号，同班不可重复' }),
      text('name', '学生姓名', '', { required: true, placeholder: '请输入学生姓名' }),
      choice('gender', '性别', labels(['', '男', '女', '未知'])),
      number('height', '身高（厘米）', null, { placeholder: '选填，例如：165.5' }),
      choice('eyesight', '视力等级', labels(['正常', '轻度', '中度', '重度'])),
      choice('score_level', '成绩等级', labels(['', 'A', 'B', 'C', 'D'])),
      choice('discipline', '纪律等级', labels(['正常', '关注', '重点关注'])),
      text('notes', '特殊需求', '', { placeholder: '选填，例如：靠窗／行动不便' }),
      number('sort_no', '排序号', 0, { placeholder: '默认 0，按数字从小到大排序' }),
      active,
      text('remark', '备注', '', { placeholder: '选填' }),
    ],
  },
  classrooms: {
    label: '教室',
    fields: [
      text('name', '教室布局名称', '', { required: true, placeholder: '请输入教室名称，例如：主教室' }),
      number('rows', '座位行数', 6, { required: true, placeholder: '请输入 1 至 50 的整数' }),
      number('columns', '座位列数', 8, { required: true, placeholder: '请输入 1 至 50 的整数' }),
      choice('podium', '讲台位置', [['front', '前方'], ['back', '后方'], ['left', '左侧'], ['right', '右侧']], 'front', { required: true }),
      choice('is_default', '默认布局', yesNo),
      active,
      text('remark', '备注', '', { placeholder: '选填' }),
    ],
  },
  rules: {
    label: '规则',
    fields: [
      text('name', '规则名称', '', { required: true, placeholder: '请输入规则名称' }),
      choice('kind', '类型', [['HARD', '硬规则'], ['SOFT', '软规则']], 'HARD', { note: '随规则类型自动确定' }),
      number('weight', '规则权重', 80, { required: true, placeholder: '请输入 0 至 1000 的整数' }),
      text('config', '规则摘要', '{}'),
      choice('enabled', '是否启用', yesNo, 1),
      active,
      text('remark', '备注', '', { placeholder: '选填' }),
    ],
  },
  relations: {
    label: '学生关系',
    fields: [
      choice('student_id', '学生', [], null, { required: true, placeholder: '请选择学生' }),
      choice('other_student_id', '另一位学生', [], null, { required: true, placeholder: '请选择另一位学生' }),
      choice('kind', '关系', [['NOT_DESKMATE', '不同桌'], ['NOT_ADJACENT', '不相邻'], ['PREFER_DESKMATE', '偏好同桌']], 'NOT_DESKMATE', { required: true }),
      active,
    ],
  },
}

const current = computed(() => schemas[section.value] ?? schemas.classes)
const filterKeys = {
  classes: ['grade', 'school_year', 'term'],
  students: ['gender', 'eyesight', 'score_level', 'discipline'],
  classrooms: ['podium', 'is_default'],
  rules: ['kind', 'enabled'],
  relations: ['kind'],
}
const filterFields = computed(() => current.value.fields.filter(field => (filterKeys[section.value] ?? []).includes(field.key)))

const ruleTypes = [
  { code: 'FRONT_ROW', label: '指定学生坐前排', kind: 'HARD', weight: 100 },
  { code: 'NOT_DESKMATE', label: '指定学生不能同桌', kind: 'HARD', weight: 100 },
  { code: 'NOT_ADJACENT', label: '指定学生不能相邻', kind: 'HARD', weight: 100 },
  { code: 'PREFER_DESKMATE', label: '指定学生优先同桌', kind: 'SOFT', weight: 80 },
  { code: 'VISION_FRONT', label: '近视学生靠前', kind: 'SOFT', weight: 80 },
  { code: 'HEIGHT_BACK', label: '高个学生靠后', kind: 'SOFT', weight: 60 },
  { code: 'GENDER_BALANCE', label: '男女搭配均衡', kind: 'SOFT', weight: 40 },
  { code: 'SCORE_BALANCE', label: '成绩强弱均衡', kind: 'SOFT', weight: 40 },
  { code: 'DISCIPLINE_SCATTER', label: '纪律关注学生分散', kind: 'SOFT', weight: 70 },
]
const studentRules = ['FRONT_ROW', 'NOT_DESKMATE', 'NOT_ADJACENT', 'PREFER_DESKMATE']
const studentMap = computed(() => new Map(students.value.map(item => [item.id, item])))

const configuredStudents = config =>
  Array.isArray(config.student_ids) ? config.student_ids : Number.isSafeInteger(config.student_id) ? [config.student_id] : []

function parsedRule(item) {
  try {
    const config = JSON.parse(item.config || '{}')
    return config && typeof config === 'object' && !Array.isArray(config) ? config : {}
  } catch {
    return {}
  }
}

function knownRule(item) {
  const config = parsedRule(item)
  const type = ruleTypes.find(entry => entry.code === config?.type && entry.kind === item.kind)
  if (!type) return null
  const ids = configuredStudents(config)
  if (
    studentRules.includes(type.code) &&
    (ids.length < (type.code === 'FRONT_ROW' ? 1 : 2) ||
      ids.some(id => !Number.isSafeInteger(id) || id <= 0) ||
      new Set(ids).size !== ids.length)
  ) {
    return null
  }
  if (type.code === 'FRONT_ROW' && (!Number.isInteger(config.max_row) || config.max_row < 1 || config.max_row > 50)) {
    return null
  }
  return type.code
}

function ruleSummary(record) {
  const code = knownRule(record)
  if (!code) return '既有自定义／异常配置'
  const meta = ruleTypes.find(item => item.code === code)
  const config = parsedRule(record)
  if (!studentRules.includes(code)) return meta.label
  const names = configuredStudents(config).map(id => {
    const student = studentMap.value.get(id)
    return student ? `${student.name}${student.active ? '' : '（停用）'}` : `#${id}`
  })
  const namePreview = names.length > 4 ? `${names.slice(0, 4).join('、')} 等 ${names.length} 人` : names.join('、')
  if (code === 'FRONT_ROW') return `前 ${config.max_row} 排：${namePreview}`
  return `${meta.label}：${namePreview}`
}

const choices = field =>
  field.key === 'classroom_id'
    ? classrooms.value.map(item => [item.id, `${item.name}${item.active ? '' : '（停用）'}`, !item.active])
    : ['student_id', 'other_student_id'].includes(field.key)
      ? students.value.map(item => [item.id, `${item.student_no} · ${item.name}${item.active ? '' : '（停用）'}`, !item.active])
      : field.options

const display = (field, value, record) => {
  if (field.key === 'term') return termLabel(value)
  if (section.value === 'rules' && field.key === 'config') return ruleSummary(record)
  return (choices(field) || []).find(option => option[0] === value)?.[1] ?? (value === '' || value == null ? '—' : value)
}

const filterOptions = field => {
  const initial =
    field.key === 'grade'
      ? classGrades.map(item => item.name)
      : field.key === 'term'
        ? terms
        : (choices(field) ?? []).map(option => option[1])
  return [...new Set([...initial, ...records.value.map(item => display(field, item[field.key], item))].filter(v => v && v !== '—'))]
}

const visibleRecords = computed(() => {
  const query = search.value.trim().toLowerCase()
  return records.value.filter(
    item =>
      (statusFilter.value === 'all' || String(item.active) === statusFilter.value) &&
      filterFields.value.every(field => !filters.value[field.key] || display(field, item[field.key], item) === filters.value[field.key]) &&
      (!query || current.value.fields.some(field => String(display(field, item[field.key], item)).toLowerCase().includes(query))),
  )
})

const selectedRecords = computed(() => selection.value)

const visiblePositions = computed(() =>
  classroomPositions.value.filter(
    item => positionsStatusFilter.value === 'all' || String(item.active) === positionsStatusFilter.value,
  ),
)

function draftSnapshot() {
  if (form.value) {
    return JSON.stringify({
      form: form.value,
      ruleType: ruleType.value,
      ruleStudents: ruleStudents.value,
      ruleMaxRow: ruleMaxRow.value,
    })
  }
  if (copying.value) return JSON.stringify(copyOptions.value)
  return ''
}

function isBusy() {
  return saving.value || !!inlineLayout.value?.isBusy()
}

function hasUnsavedDraft() {
  const snapshot = draftSnapshot()
  return !!snapshot && snapshot !== initialDraft
}

async function canLeaveDraft() {
  if (!hasUnsavedDraft() && !studentPreview.value && !inlineLayout.value?.hasUnsavedChanges()) return true
  return confirmAction('当前表单有未保存的修改，确定放弃吗？', {
    title: '放弃未保存修改',
    confirmText: '放弃',
    type: 'warning',
    danger: true,
  })
}

async function cancelDraft() {
  if (!(await canLeaveDraft())) return
  form.value = null
  copying.value = false
  formErrors.value = {}
  copyErrors.value = {}
  initialDraft = ''
}

function handleFormDrawerClose(done) {
  void cancelDraft().then(() => {
    if (!form.value) done()
  })
}

function beforeUnload(event) {
  if (hasUnsavedDraft()) {
    event.preventDefault()
    event.returnValue = ''
  }
}

async function load() {
  const version = ++loadVersion
  const requestedSection = section.value
  const requestedClass = props.classId
  loading.value = true
  loadError.value = ''
  records.value = []
  try {
    const nextClasses = await invoke('list_records', { resource: 'classes', classId: null })
    if (version !== loadVersion) return null
    const [nextStudents, nextClassrooms, nextRecords] = requestedClass
      ? await Promise.all([
          invoke('list_records', { resource: 'students', classId: requestedClass }),
          invoke('list_records', { resource: 'classrooms', classId: requestedClass }),
          requestedSection === 'classes'
            ? Promise.resolve(nextClasses)
            : invoke('list_records', { resource: requestedSection, classId: requestedClass }),
        ])
      : [[], [], requestedSection === 'classes' ? nextClasses : []]
    if (version !== loadVersion) return null
    students.value = nextStudents
    classrooms.value = nextClassrooms
    records.value = nextRecords
    selection.value = []
    return true
  } catch (error) {
    if (version !== loadVersion) return null
    loadError.value = String(error)
    notifyError(error)
    return false
  } finally {
    if (version === loadVersion) loading.value = false
  }
}

async function retryLoad() {
  if (await load()) notifySuccess('列表已刷新')
}

async function edit(item = null) {
  if (!(await canLeaveDraft())) return
  copying.value = false
  studentImport.value = false
  studentPreview.value = null
  formErrors.value = {}
  editorRevision.value += 1
  form.value = item ? { ...item } : Object.fromEntries(current.value.fields.map(field => [field.key, field.initial]))
  originalRecord.value = item ? JSON.parse(JSON.stringify(item)) : null
  originalClassroom.value = section.value === 'classrooms' && item ? { rows: item.rows, columns: item.columns } : null
  if (section.value !== 'classes') form.value.class_id = props.classId
  if (section.value === 'rules') {
    const config = parsedRule(form.value)
    ruleType.value = item ? (knownRule(form.value) ?? 'legacy') : 'VISION_FRONT'
    ruleStudents.value = configuredStudents(config)
    ruleMaxRow.value = config.max_row ?? 1
    if (!item) updateRuleType()
  }
  initialDraft = draftSnapshot()
}

function updateRuleType() {
  const selected = ruleTypes.find(item => item.code === ruleType.value)
  if (!selected) return
  form.value.kind = selected.kind
  if (!form.value.id) {
    form.value.name = selected.label
    form.value.weight = selected.weight
  }
  clearError(formErrors, 'ruleStudents')
  clearError(formErrors, 'ruleMaxRow')
  clearError(formErrors, 'name')
}

function changeGrade(name) {
  const grade = classGrades.find(option => option.name === name)
  form.value.school_stage = grade?.stage ?? originalRecord.value?.school_stage ?? ''
  form.value.subjects = grade?.subjects.join(',') ?? originalRecord.value?.subjects ?? ''
  clearError(formErrors, 'grade')
  clearError(formErrors, 'subjects')
}

function isWholeNumberInRange(value, min, max) {
  if (value === '' || value === null || value === undefined) return false
  const parsed = Number(value)
  return Number.isInteger(parsed) && parsed >= min && parsed <= max
}

function validateMainForm() {
  const errors = {}
  const data = form.value
  if (section.value === 'classes') {
    if (!String(data.name ?? '').trim()) errors.name = '请填写班级名称'
    const gradeChanged = data.grade !== originalRecord.value?.grade
    if ((!originalRecord.value || gradeChanged) && !String(data.grade ?? '').trim()) errors.grade = '请选择年级'
    const termChanged = data.term !== originalRecord.value?.term
    if ((!originalRecord.value || termChanged) && !String(data.term ?? '').trim()) errors.term = '请选择学期'
    if ((!originalRecord.value || gradeChanged || data.subjects !== originalRecord.value?.subjects) && !selectedSubjects.value.length) {
      errors.subjects = '请至少选择一门班级科目'
    }
  } else if (section.value === 'students') {
    if (!String(data.student_no ?? '').trim()) errors.student_no = '请填写学号'
    if (!String(data.name ?? '').trim()) errors.name = '请填写学生姓名'
    if (data.height !== '' && data.height !== null && data.height !== undefined) {
      const h = Number(data.height)
      if (!Number.isFinite(h) || h <= 0) errors.height = '身高须留空或填写大于 0 的数字'
    }
    if (data.sort_no !== '' && data.sort_no !== null && data.sort_no !== undefined && !Number.isInteger(Number(data.sort_no))) {
      errors.sort_no = '排序号须为整数'
    }
  } else if (section.value === 'classrooms') {
    if (!String(data.name ?? '').trim()) errors.name = '请填写教室布局名称'
    if (!isWholeNumberInRange(data.rows, 1, 50)) errors.rows = '请填写 1 至 50 之间的整数行数'
    if (!isWholeNumberInRange(data.columns, 1, 50)) errors.columns = '请填写 1 至 50 之间的整数列数'
    if (!data.podium) errors.podium = '请选择讲台位置'
  } else if (section.value === 'rules') {
    if (!String(data.name ?? '').trim()) errors.name = '请填写规则名称'
    if (!isWholeNumberInRange(data.weight, 0, 1000)) errors.weight = '请填写 0 至 1000 之间的整数权重'
    if (ruleType.value !== 'legacy') {
      if (studentRules.includes(ruleType.value)) {
        const minimum = ruleType.value === 'FRONT_ROW' ? 1 : 2
        if (ruleStudents.value.length < minimum) errors.ruleStudents = `请至少选择 ${minimum} 名学生`
      }
      if (ruleType.value === 'FRONT_ROW' && !isWholeNumberInRange(ruleMaxRow.value, 1, 50)) {
        errors.ruleMaxRow = '请填写 1 至 50 之间的整数排数'
      }
    }
  } else if (section.value === 'relations') {
    if (!data.student_id) errors.student_id = '请选择学生'
    if (!data.other_student_id) errors.other_student_id = '请选择另一位学生'
    else if (data.student_id && data.student_id === data.other_student_id) errors.other_student_id = '不能选择同一位学生'
    if (!data.kind) errors.kind = '请选择关系类型'
  }
  formErrors.value = errors
  if (Object.keys(errors).length) {
    notifyError('请先完善标红的必填项或修正输入内容')
    return false
  }
  return true
}

function validateCopyForm() {
  const errors = {}
  if (!String(copyOptions.value?.name ?? '').trim()) errors.name = '请填写新班级名称'
  if (!String(copyOptions.value?.term ?? '').trim()) errors.term = '请选择目标学期'
  copyErrors.value = errors
  if (Object.keys(errors).length) {
    notifyError('请先完善标红的必填项')
    return false
  }
  return true
}

function validatePositionForm() {
  const errors = {}
  const maxRow = Number(positionsDrawerClassroom.value?.rows) || 50
  const maxCol = Number(positionsDrawerClassroom.value?.columns) || 50
  if (!isWholeNumberInRange(positionForm.value?.row_number, 1, maxRow)) {
    errors.row_number = `请填写 1 至 ${maxRow} 之间的整数行号`
  }
  if (!isWholeNumberInRange(positionForm.value?.column_number, 1, maxCol)) {
    errors.column_number = `请填写 1 至 ${maxCol} 之间的整数列号`
  }
  if (!positionForm.value?.kind) errors.kind = '请选择位置类型'
  positionErrors.value = errors
  if (Object.keys(errors).length) {
    notifyError('请先完善标红的必填项或修正输入内容')
    return false
  }
  return true
}

async function save() {
  if (saving.value) return
  if (!validateMainForm()) return
  if (
    section.value === 'classrooms' &&
    originalClassroom.value &&
    (Number(form.value.rows) < originalClassroom.value.rows || Number(form.value.columns) < originalClassroom.value.columns)
  ) {
    const ok = await confirmAction('缩小教室后，超出新范围的位置将不在网格中显示。请先在「座位明细」停用超出范围的位置；历史数据会保留。确定继续吗？', {
      title: '缩小教室尺寸',
      confirmText: '继续保存',
      type: 'warning',
    })
    if (!ok) return
  }
  saving.value = true
  try {
    if (section.value === 'classes') {
      const gradeChanged = form.value.grade !== originalRecord.value?.grade
      if (gradeChanged && !form.value.grade) throw new Error('请选择年级')
      if ((gradeChanged || form.value.subjects !== originalRecord.value?.subjects) && !selectedSubjects.value.length) {
        throw new Error('请至少选择一门班级科目')
      }
    }
    if (section.value === 'rules' && ruleType.value !== 'legacy') {
      const config = form.value.id ? parsedRule(form.value) : {}
      config.type = ruleType.value
      if (studentRules.includes(ruleType.value)) {
        const minimum = ruleType.value === 'FRONT_ROW' ? 1 : 2
        if (ruleStudents.value.length < minimum) throw new Error(`请选择至少 ${minimum} 名学生`)
        config.student_ids = ruleStudents.value
        delete config.student_id
      } else {
        delete config.student_ids
        delete config.student_id
      }
      if (ruleType.value === 'FRONT_ROW') {
        if (!Number.isInteger(Number(ruleMaxRow.value)) || Number(ruleMaxRow.value) < 1 || Number(ruleMaxRow.value) > 50) {
          throw new Error('请填写有效的前排排数')
        }
        config.max_row = Number(ruleMaxRow.value)
      } else {
        delete config.max_row
      }
      form.value.config = JSON.stringify(config)
    }
    for (const field of current.value.fields.filter(field => field.type === 'number')) {
      const value = form.value[field.key]
      form.value[field.key] = value === '' || value === null ? null : Number(value)
    }
    let savedId = null
    if (section.value === 'classrooms') {
      const changes = inlineLayout.value.changesForSave()
      savedId = await invoke('save_classroom_with_layout', { data: form.value, expected: originalRecord.value, changes })
    } else {
      savedId = await invoke('save_record', { resource: section.value, data: form.value, expected: originalRecord.value })
    }
    form.value = null
    initialDraft = ''
    if (section.value === 'classes') emit('classes-changed', savedId)
    const refreshed = await load()
    if (refreshed !== null) {
      if (refreshed) notifySuccess('保存成功')
      else notifyWarning('保存成功，但列表刷新失败，请点击重新加载')
    }
  } catch (error) {
    notifyError(`保存失败：${error}`)
  } finally {
    saving.value = false
  }
}

async function beginCopy(item) {
  if (!(await canLeaveDraft())) return
  copyOptions.value = {
    source_id: item.id,
    name: '',
    ...nextSchoolTerm(item),
    students: true,
    relations: true,
    rules: true,
    classrooms: true,
  }
  form.value = null
  copyErrors.value = {}
  copying.value = true
  initialDraft = draftSnapshot()
}

async function copy() {
  if (saving.value) return
  if (!validateCopyForm()) return
  saving.value = true
  try {
    const id = await invoke('copy_class', { options: copyOptions.value })
    copying.value = false
    initialDraft = ''
    emit('classes-changed', id)
    const refreshed = await load()
    if (refreshed !== null) {
      if (refreshed) notifySuccess('班级复制成功，已切换到新班级')
      else notifyWarning('班级复制成功，但列表刷新失败')
    }
  } catch (error) {
    notifyError(`复制失败：${error}`)
  } finally {
    saving.value = false
  }
}

async function workbookAction(action) {
  saving.value = true
  try {
    await action()
  } catch (error) {
    notifyError(error)
  } finally {
    saving.value = false
  }
}

async function openStudentImport() {
  if (!(await canLeaveDraft())) return
  form.value = null
  studentPreview.value = null
  studentImport.value = true
  updateStudents.value = false
}

function readStudentFile(event) {
  const file = event.target.files?.[0]
  event.target.value = ''
  if (!file) return
  studentPreview.value = null
  return workbookAction(async () => {
    studentPreview.value = await invoke('preview_students', {
      classId: props.classId,
      bytes: await workbookBytes(file),
      updateExisting: updateStudents.value,
    })
    if (studentPreview.value.errors.length) {
      notifyWarning(`预检发现 ${studentPreview.value.errors.length} 项错误`)
    } else {
      notifyInfo(`预检通过，待导入 ${studentPreview.value.students.length} 名学生`)
    }
  })
}

function importStudents() {
  const preview = studentPreview.value
  if (!preview || preview.errors.length || !preview.students.length) return
  return workbookAction(async () => {
    await invoke('import_students', {
      classId: props.classId,
      students: preview.students,
      expected: preview.snapshot,
      updateExisting: updateStudents.value,
    })
    studentPreview.value = null
    studentImport.value = false
    if (await load()) notifySuccess('学生导入成功')
  })
}

function exportRecords() {
  return workbookAction(async () => {
    const items = selectedRecords.value.length ? selectedRecords.value : visibleRecords.value
    const data = await invoke('export_records', {
      resource: section.value,
      classId: props.classId,
      ids: items.map(item => item.id),
    })
    downloadWorkbook(data, `${current.value.label}资料.xlsx`)
    notifySuccess(`已导出 ${items.length} 条${current.value.label}资料`)
  })
}

async function deactivateSelection() {
  const expected = selectedRecords.value.filter(item => item.active)
  if (!expected.length) return
  if (!(await canLeaveDraft())) return
  const ok = await confirmAction(`确定停用选中的 ${expected.length} 条${current.value.label}资料？停用后不会参与新排座，历史方案不受影响。`, {
    title: '批量停用确认',
    confirmText: '确认停用',
    type: 'warning',
    danger: true,
  })
  if (!ok) return
  await workbookAction(async () => {
    await invoke('deactivate_records', { resource: section.value, classId: props.classId, expected })
    selection.value = []
    tableRef.value?.clearSelection()
    form.value = null
    studentPreview.value = null
    if (section.value === 'classes') emit('classes-changed')
    if (await load()) notifySuccess(`已停用 ${expected.length} 条资料`)
  })
}

// Classroom raw positions detail drawer (folded "座位管理")
async function openPositionsDrawer(classroom) {
  if (!(await canLeaveDraft())) return
  positionsDrawerClassroom.value = classroom
  positionsStatusFilter.value = 'all'
  positionSelection.value = []
  positionForm.value = null
  await loadClassroomPositions()
}

async function loadClassroomPositions() {
  if (!positionsDrawerClassroom.value || !props.classId) return
  try {
    const all = await invoke('list_records', { resource: 'positions', classId: props.classId })
    classroomPositions.value = all.filter(item => item.classroom_id === positionsDrawerClassroom.value.id)
  } catch (error) {
    notifyError(`读取座位明细失败：${error}`)
  }
}

function editPositionRow(item = null) {
  originalPosition.value = item ? JSON.parse(JSON.stringify(item)) : null
  positionErrors.value = {}
  positionForm.value = item
    ? { ...item }
    : {
        class_id: props.classId,
        classroom_id: positionsDrawerClassroom.value.id,
        row_number: 1,
        column_number: 1,
        kind: 'seat',
        desk_group: '',
        active: 1,
      }
}

async function savePositionRow() {
  if (!validatePositionForm()) return
  await workbookAction(async () => {
    const payload = {
      ...positionForm.value,
      row_number: Number(positionForm.value.row_number),
      column_number: Number(positionForm.value.column_number),
    }
    await invoke('save_record', { resource: 'positions', data: payload, expected: originalPosition.value })
    positionForm.value = null
    await loadClassroomPositions()
    notifySuccess('座位记录已保存')
  })
}

async function deactivatePositions() {
  const expected = positionSelection.value.filter(item => item.active)
  if (!expected.length) return
  const ok = await confirmAction(`确定停用选中的 ${expected.length} 个座位位置？`, {
    title: '停用座位位置',
    confirmText: '确认停用',
    type: 'warning',
    danger: true,
  })
  if (!ok) return
  await workbookAction(async () => {
    await invoke('deactivate_records', { resource: 'positions', classId: props.classId, expected })
    positionSelection.value = []
    await loadClassroomPositions()
    notifySuccess(`已停用 ${expected.length} 个座位位置`)
  })
}

function clearFilters() {
  search.value = ''
  statusFilter.value = 'all'
  filters.value = {}
  selection.value = []
  tableRef.value?.clearSelection()
}

function colMinWidth(field) {
  const map = {
    name: 130,
    student_no: 110,
    subjects: 200,
    config: 230,
    notes: 150,
    remark: 140,
    grade: 110,
    school_year: 115,
    student_id: 150,
    other_student_id: 150,
  }
  return map[field.key] ?? 96
}

watch([section, () => props.classId], ([nextSection], [previousSection]) => {
  if (nextSection !== previousSection) {
    statusFilter.value = 'all'
    search.value = ''
  }
  filters.value = {}
  selection.value = []
  studentImport.value = false
  studentPreview.value = null
  form.value = null
  copying.value = false
  positionsDrawerClassroom.value = null
  positionForm.value = null
  formErrors.value = {}
  copyErrors.value = {}
  positionErrors.value = {}
  initialDraft = ''
  void load()
})

watch(
  () => props.openImport,
  value => {
    if (value && section.value === 'students' && props.classId) {
      void openStudentImport()
      emit('open-import-handled')
    }
  },
)

onMounted(async () => {
  window.addEventListener('beforeunload', beforeUnload)
  if ((await load()) && props.openImport && section.value === 'students' && props.classId) {
    await openStudentImport()
    emit('open-import-handled')
  }
})
onBeforeUnmount(() => window.removeEventListener('beforeunload', beforeUnload))
</script>

<template>
  <div class="page-container">
    <div v-if="section !== 'classes' && !classId" class="page-card">
      <ElEmpty description="请先在「班级管理」创建班级，或在顶部选择一个班级" />
    </div>

    <template v-else>
      <div class="action-bar">
        <div class="action-group">
          <ElButton
            type="primary"
            :icon="Plus"
            :disabled="loading || saving || (section !== 'classes' && !classId)"
            @click="edit()"
          >
            新增{{ current.label }}
          </ElButton>
          <ElButton v-if="section === 'students'" :icon="Upload" :disabled="saving || !classId" @click="openStudentImport">
            导入学生
          </ElButton>
          <ElButton
            v-if="['classes', 'students', 'classrooms', 'rules'].includes(section)"
            :icon="Download"
            :disabled="saving || !visibleRecords.length"
            @click="exportRecords"
          >
            {{ selectedRecords.length ? `导出选中 (${selectedRecords.length})` : '导出 Excel' }}
          </ElButton>
        </div>
        <span class="form-hint">共 {{ visibleRecords.length }} 条{{ current.label }}记录</span>
      </div>

      <div class="filter-bar">
        <ElInput
          v-model="search"
          clearable
          :placeholder="`搜索${current.label}资料`"
          :prefix-icon="Search"
          style="width: 210px"
        />
        <div class="filter-item">
          <span>状态</span>
          <ElSelect v-model="statusFilter" style="width: 100px">
            <ElOption label="全部" value="all" />
            <ElOption label="正常" value="1" />
            <ElOption label="停用" value="0" />
          </ElSelect>
        </div>
        <div v-for="field in filterFields" :key="field.key" class="filter-item">
          <span>{{ field.label }}</span>
          <ElSelect v-model="filters[field.key]" clearable placeholder="全部" style="width: 126px">
            <ElOption v-for="option in filterOptions(field)" :key="option" :label="option" :value="option" />
          </ElSelect>
        </div>
        <ElButton :icon="Refresh" :disabled="saving" @click="clearFilters">重置</ElButton>
      </div>

      <div v-if="selectedRecords.length" class="selection-bar">
        <span>已选择 <strong>{{ selectedRecords.length }}</strong> 条{{ current.label }}资料</span>
        <div class="action-group">
          <ElButton size="small" :disabled="saving || selectedRecords.length !== 1" @click="edit(selectedRecords[0])">
            修改选中
          </ElButton>
          <ElButton
            size="small"
            type="danger"
            plain
            :disabled="saving || !selectedRecords.some(item => item.active)"
            @click="deactivateSelection"
          >
            批量停用 ({{ selectedRecords.filter(item => item.active).length }})
          </ElButton>
          <ElButton size="small" text @click="tableRef?.clearSelection()">取消选择</ElButton>
        </div>
      </div>

      <ElAlert v-if="loadError" type="error" show-icon :closable="false" title="列表加载失败" style="margin-bottom: 12px">
        <template #default>
          <p style="margin: 4px 0 8px">{{ loadError }}</p>
          <ElButton size="small" @click="retryLoad">重新加载</ElButton>
        </template>
      </ElAlert>

      <div class="table-card">
        <ElTable
          ref="tableRef"
          v-loading="loading"
          :data="visibleRecords"
          row-key="id"
          stripe
          empty-text="暂无符合条件的记录"
          @selection-change="selection = $event"
        >
          <ElTableColumn type="selection" width="44" fixed="left" />

          <ElTableColumn
            v-for="(field, index) in current.fields"
            :key="field.key"
            :label="field.label"
            :min-width="colMinWidth(field)"
            :fixed="index === 0 ? 'left' : false"
            show-overflow-tooltip
          >
            <template #default="{ row }">
              <ElTag v-if="field.key === 'active'" size="small" :type="row.active ? 'success' : 'info'" effect="light">
                {{ row.active ? '正常' : '停用' }}
              </ElTag>
              <ElTag v-else-if="field.key === 'is_default'" size="small" :type="row.is_default ? 'success' : 'info'" effect="plain">
                {{ row.is_default ? '默认布局' : '普通' }}
              </ElTag>
              <ElTag
                v-else-if="section === 'rules' && field.key === 'kind'"
                size="small"
                :type="row.kind === 'HARD' ? 'danger' : 'primary'"
                effect="light"
              >
                {{ row.kind === 'HARD' ? '硬规则' : '软规则' }}
              </ElTag>
              <ElTag
                v-else-if="section === 'rules' && field.key === 'enabled'"
                size="small"
                :type="row.enabled ? 'success' : 'info'"
                effect="plain"
              >
                {{ row.enabled ? '已启用' : '未启用' }}
              </ElTag>
              <template v-else-if="section === 'rules' && field.key === 'config'">
                <ElTag v-if="!knownRule(row)" size="small" type="warning" effect="light">既有自定义／异常配置</ElTag>
                <span v-else>{{ ruleSummary(row) }}</span>
              </template>
              <ElTag
                v-else-if="field.key === 'score_level' && row.score_level"
                size="small"
                :type="row.score_level === 'A' ? 'success' : row.score_level === 'B' ? 'primary' : row.score_level === 'C' ? 'warning' : 'danger'"
                effect="light"
              >
                {{ row.score_level }}
              </ElTag>
              <span v-else>{{ display(field, row[field.key], row) }}</span>
            </template>
          </ElTableColumn>

          <ElTableColumn label="操作" :width="section === 'classes' ? 175 : section === 'classrooms' ? 170 : 90" fixed="right">
            <template #default="{ row }">
              <ElButton link type="primary" :disabled="saving" @click="edit(row)">
                {{ section === 'classrooms' ? '编辑与布局' : '编辑' }}
              </ElButton>
              <ElButton v-if="section === 'classes'" link type="primary" :icon="CopyDocument" :disabled="saving" @click="beginCopy(row)">
                复制到新学期
              </ElButton>
              <ElButton v-if="section === 'classrooms'" link type="info" :disabled="saving" @click="openPositionsDrawer(row)">
                座位明细
              </ElButton>
            </template>
          </ElTableColumn>
        </ElTable>
      </div>
    </template>

    <!-- 新增／编辑抽屉 -->
    <ElDrawer
      :model-value="!!form"
      :title="`${form?.id ? '编辑' : '新增'}${current.label}`"
      :size="section === 'classrooms' ? 'min(94vw, 960px)' : 'min(92vw, 560px)'"
      :before-close="handleFormDrawerClose"
      destroy-on-close
    >
      <div v-if="form" class="drawer-form">
        <p class="required-legend"><span>*</span> 为必填项</p>
        <div class="form-grid-2">
          <div v-if="section === 'rules'" class="form-field">
            <span class="form-label is-required">规则类型</span>
            <ElSelect v-model="ruleType" :disabled="saving || !!form.id" @change="updateRuleType">
              <ElOption v-if="ruleType === 'legacy'" label="既有自定义规则" value="legacy" />
              <ElOption v-for="type in ruleTypes" :key="type.code" :label="type.label" :value="type.code" />
            </ElSelect>
          </div>
          <div
            v-if="section === 'rules' && studentRules.includes(ruleType)"
            class="form-field full-span"
            :class="{ 'is-error': !!formErrors.ruleStudents }"
          >
            <span class="form-label is-required">
              指定学生
              <span class="form-label-note">{{ ruleType === 'FRONT_ROW' ? '至少选择 1 人' : '至少选择 2 人' }}</span>
            </span>
            <ElSelect
              v-model="ruleStudents"
              multiple
              filterable
              :disabled="saving"
              :placeholder="ruleType === 'FRONT_ROW' ? '请至少选择 1 名学生' : '请至少选择 2 名学生'"
              @change="clearError(formErrors, 'ruleStudents')"
            >
              <ElOption
                v-for="student in students"
                :key="student.id"
                :label="`${student.student_no} · ${student.name}${student.active ? '' : '（停用）'}`"
                :value="student.id"
                :disabled="!student.active && !ruleStudents.includes(student.id)"
              />
            </ElSelect>
            <p v-if="formErrors.ruleStudents" class="form-error">{{ formErrors.ruleStudents }}</p>
          </div>
          <div
            v-if="section === 'rules' && ruleType === 'FRONT_ROW'"
            class="form-field"
            :class="{ 'is-error': !!formErrors.ruleMaxRow }"
          >
            <span class="form-label is-required">限坐前几排</span>
            <ElInput
              v-model.number="ruleMaxRow"
              type="number"
              :min="1"
              :max="Math.max(10, parsedRule(originalRecord || {}).max_row || 0)"
              placeholder="请输入 1 至 50 的整数"
              :disabled="saving"
              @input="clearError(formErrors, 'ruleMaxRow')"
            />
            <p v-if="formErrors.ruleMaxRow" class="form-error">{{ formErrors.ruleMaxRow }}</p>
          </div>

          <template v-for="field in current.fields" :key="field.key">
            <div v-if="field.key === 'subjects'" class="form-field full-span" :class="{ 'is-error': !!formErrors.subjects }">
              <span class="form-label is-required">{{ field.label }}<span class="form-label-note">至少选择 1 门</span></span>
              <ElSelect v-model="selectedSubjects" multiple filterable :placeholder="field.placeholder || '请选择班级科目'" :disabled="saving">
                <ElOption v-for="subject in subjectChoices" :key="subject" :label="subject" :value="subject" />
              </ElSelect>
              <p v-if="formErrors.subjects" class="form-error">{{ formErrors.subjects }}</p>
            </div>
            <div
              v-else-if="section !== 'rules' || field.key !== 'config'"
              class="form-field"
              :class="{ 'is-error': !!formErrors[field.key] }"
            >
              <span class="form-label" :class="{ 'is-required': !!field.required }">
                {{ field.label }}
                <span v-if="field.note && !(section === 'rules' && field.key === 'kind' && ruleType === 'legacy')" class="form-label-note">
                  {{ field.note }}
                </span>
              </span>
              <ElSelect
                v-if="field.key === 'grade'"
                v-model="form.grade"
                :disabled="saving"
                :placeholder="field.placeholder || '请选择年级'"
                @change="changeGrade"
              >
                <ElOption v-for="grade in classGrades" :key="grade.name" :label="grade.name" :value="grade.name" />
                <ElOption v-if="legacyGrade" :label="`${legacyGrade}（原值）`" :value="legacyGrade" />
              </ElSelect>
              <ElSelect
                v-else-if="field.key === 'term'"
                v-model="form.term"
                :disabled="saving"
                :placeholder="field.placeholder || '请选择学期'"
                @change="clearError(formErrors, 'term')"
              >
                <ElOption v-for="term in terms" :key="term" :label="term" :value="term" />
                <ElOption
                  v-if="originalRecord?.term && !terms.includes(originalRecord.term)"
                  :label="`${termLabel(originalRecord.term)}（原值）`"
                  :value="originalRecord.term"
                />
              </ElSelect>
              <ElSelect
                v-else-if="field.options"
                v-model="form[field.key]"
                :filterable="['student_id', 'other_student_id'].includes(field.key)"
                :disabled="saving || (section === 'rules' && field.key === 'kind' && ruleType !== 'legacy')"
                :placeholder="field.placeholder || '请选择'"
                @change="clearError(formErrors, field.key); if (field.key === 'student_id') clearError(formErrors, 'other_student_id')"
              >
                <ElOption
                  v-for="option in choices(field)"
                  :key="String(option[0])"
                  :label="option[1]"
                  :value="option[0]"
                  :disabled="option[2]"
                />
                <ElOption
                  v-if="originalRecord && field.key in originalRecord && !choices(field).some(option => option[0] === originalRecord[field.key])"
                  :label="`${originalRecord[field.key] || '未填写'}（原值）`"
                  :value="originalRecord[field.key]"
                />
              </ElSelect>
              <ElInput
                v-else
                v-model="form[field.key]"
                :type="field.type === 'number' ? 'number' : 'text'"
                :step="field.key === 'height' ? '0.1' : '1'"
                :placeholder="field.placeholder || ''"
                :disabled="saving"
                :readonly="field.key === 'school_stage'"
                @input="clearError(formErrors, field.key)"
              />
              <p v-if="formErrors[field.key]" class="form-error">{{ formErrors[field.key] }}</p>
            </div>
          </template>
        </div>

        <LayoutEditor
          v-if="section === 'classrooms'"
          :key="editorRevision"
          ref="inlineLayout"
          :classroom="form"
          :class-id="classId"
          :disabled="saving || !form.active"
          inline
        />

        <ElAlert
          v-if="section === 'rules' && ruleType === 'legacy'"
          type="warning"
          show-icon
          :closable="false"
          title="既有自定义／异常规则配置会原样保留；如参数不完整，请将其停用后新建规则。"
        />
      </div>

      <template #footer>
        <div class="drawer-footer">
          <ElButton :disabled="saving" @click="cancelDraft">取消</ElButton>
          <ElButton type="primary" :loading="saving" @click="save">保存</ElButton>
        </div>
      </template>
    </ElDrawer>

    <!-- 复制班级到新学期弹窗 -->
    <ElDialog
      :model-value="copying"
      title="复制班级到新学期"
      width="min(92vw, 540px)"
      :before-close="done => { void cancelDraft().then(() => { if (!copying) done() }) }"
    >
      <div v-if="copyOptions" class="drawer-form">
        <p class="required-legend"><span>*</span> 为必填项</p>
        <div class="form-grid-2">
          <div class="form-field full-span" :class="{ 'is-error': !!copyErrors.name }">
            <span class="form-label is-required">新班级名称</span>
            <ElInput
              v-model="copyOptions.name"
              :disabled="saving"
              placeholder="请输入新班级名称，例如：高一（1）班"
              @input="clearError(copyErrors, 'name')"
            />
            <p v-if="copyErrors.name" class="form-error">{{ copyErrors.name }}</p>
          </div>
          <div class="form-field">
            <span class="form-label">目标学年<span class="form-label-note">选填</span></span>
            <ElInput v-model="copyOptions.school_year" :disabled="saving" placeholder="选填，例如：2026-2027" />
          </div>
          <div class="form-field" :class="{ 'is-error': !!copyErrors.term }">
            <span class="form-label is-required">目标学期</span>
            <ElSelect v-model="copyOptions.term" :disabled="saving" placeholder="请选择目标学期" @change="clearError(copyErrors, 'term')">
              <ElOption v-for="term in terms" :key="term" :label="term" :value="term" />
            </ElSelect>
            <p v-if="copyErrors.term" class="form-error">{{ copyErrors.term }}</p>
          </div>
        </div>
        <div class="copy-checks">
          <ElCheckbox
            v-model="copyOptions.students"
            :disabled="saving"
            @change="copyOptions.relations = copyOptions.students && copyOptions.relations"
          >
            复制在用学生
          </ElCheckbox>
          <ElCheckbox v-model="copyOptions.relations" :disabled="saving || !copyOptions.students">
            复制学生关系
          </ElCheckbox>
          <ElCheckbox v-model="copyOptions.rules" :disabled="saving">复制排座规则</ElCheckbox>
          <ElCheckbox v-model="copyOptions.classrooms" :disabled="saving">复制默认教室及座位</ElCheckbox>
        </div>
        <p class="form-hint">仅复制在用资料；成绩和历史排座方案不会复制到新班级。</p>
      </div>
      <template #footer>
        <div class="drawer-footer">
          <ElButton :disabled="saving" @click="cancelDraft">取消</ElButton>
          <ElButton type="primary" :loading="saving" @click="copy">确认复制</ElButton>
        </div>
      </template>
    </ElDialog>

    <!-- 导入学生弹窗 -->
    <ElDialog
      :model-value="studentImport"
      title="导入学生名单（.xlsx）"
      width="min(92vw, 600px)"
      :before-close="done => { studentImport = false; studentPreview = null; done() }"
    >
      <div class="drawer-form">
        <p class="form-hint">请使用标准 Excel 模板填写学号、姓名及排座特征。勾选「按学号更新」时，留空的可选列会保留原值。</p>
        <div class="action-group">
          <ElButton
            :icon="Download"
            :disabled="saving"
            @click="workbookAction(async () => downloadWorkbook(await invoke('student_template'), '学生导入模板.xlsx'))"
          >
            下载学生模板
          </ElButton>
          <input ref="studentFileInput" type="file" accept=".xlsx" class="hidden-file-input" @change="readStudentFile" />
          <ElButton type="primary" plain :icon="Upload" :loading="saving" @click="studentFileInput?.click()">
            选择 Excel 文件并预检
          </ElButton>
        </div>
        <ElCheckbox v-model="updateStudents" :disabled="saving" @change="studentPreview = null">
          按学号更新该班已有学生资料
        </ElCheckbox>
        <template v-if="studentPreview">
          <div class="action-group">
            <ElTag type="success" effect="light">待导入 {{ studentPreview.students.length }} 人</ElTag>
            <ElTag :type="studentPreview.errors.length ? 'danger' : 'success'" effect="dark">
              错误 {{ studentPreview.errors.length }} 项
            </ElTag>
          </div>
          <ElAlert v-if="studentPreview.errors.length" type="error" show-icon :closable="false" title="存在校验错误，请修改表格后重试">
            <ul class="error-list">
              <li v-for="(error, index) in studentPreview.errors" :key="index">{{ error }}</li>
            </ul>
          </ElAlert>
        </template>
      </div>
      <template #footer>
        <div class="drawer-footer">
          <ElButton :disabled="saving" @click="studentImport = false; studentPreview = null">取消</ElButton>
          <ElButton
            type="primary"
            :loading="saving"
            :disabled="!studentPreview?.students.length || !!studentPreview.errors.length"
            @click="importStudents"
          >
            确认导入
          </ElButton>
        </div>
      </template>
    </ElDialog>

    <!-- 教室座位明细抽屉（原「座位管理」收敛至此） -->
    <ElDrawer
      :model-value="!!positionsDrawerClassroom"
      :title="positionsDrawerClassroom ? `${positionsDrawerClassroom.name} · 座位明细（高级）` : '座位明细'"
      size="min(94vw, 820px)"
      :before-close="done => { positionsDrawerClassroom = null; positionForm = null; done() }"
    >
      <div v-if="positionsDrawerClassroom" class="drawer-form">
        <div class="action-bar">
          <div class="action-group">
            <ElButton type="primary" size="small" :icon="Plus" :disabled="saving" @click="editPositionRow()">新增单个位置</ElButton>
            <ElButton
              size="small"
              type="danger"
              plain
              :disabled="saving || !positionSelection.some(item => item.active)"
              @click="deactivatePositions"
            >
              批量停用 ({{ positionSelection.filter(item => item.active).length }})
            </ElButton>
          </div>
          <ElSelect v-model="positionsStatusFilter" size="small" style="width: 110px">
            <ElOption label="全部状态" value="all" />
            <ElOption label="正常" value="1" />
            <ElOption label="停用" value="0" />
          </ElSelect>
        </div>
        <p class="form-hint">
          日常绘制推荐使用「编辑与布局」中的可视化网格；此处用于核对单个坐标记录，或在缩小教室前批量停用超出范围的位置。
        </p>

        <div v-if="positionForm" class="page-card" style="background: #f8fbfa">
          <p class="required-legend" style="margin-bottom: 10px"><span>*</span> 为必填项</p>
          <div class="form-grid-2">
            <div class="form-field" :class="{ 'is-error': !!positionErrors.row_number }">
              <span class="form-label is-required">行号</span>
              <ElInput
                v-model.number="positionForm.row_number"
                type="number"
                :min="1"
                :placeholder="`请输入 1 至 ${positionsDrawerClassroom.rows} 的整数`"
                :disabled="saving"
                @input="clearError(positionErrors, 'row_number')"
              />
              <p v-if="positionErrors.row_number" class="form-error">{{ positionErrors.row_number }}</p>
            </div>
            <div class="form-field" :class="{ 'is-error': !!positionErrors.column_number }">
              <span class="form-label is-required">列号</span>
              <ElInput
                v-model.number="positionForm.column_number"
                type="number"
                :min="1"
                :placeholder="`请输入 1 至 ${positionsDrawerClassroom.columns} 的整数`"
                :disabled="saving"
                @input="clearError(positionErrors, 'column_number')"
              />
              <p v-if="positionErrors.column_number" class="form-error">{{ positionErrors.column_number }}</p>
            </div>
            <div class="form-field" :class="{ 'is-error': !!positionErrors.kind }">
              <span class="form-label is-required">类型</span>
              <ElSelect v-model="positionForm.kind" :disabled="saving" placeholder="请选择位置类型" @change="clearError(positionErrors, 'kind')">
                <ElOption label="座位" value="seat" />
                <ElOption label="过道" value="aisle" />
                <ElOption label="不可用" value="unavailable" />
              </ElSelect>
              <p v-if="positionErrors.kind" class="form-error">{{ positionErrors.kind }}</p>
            </div>
            <div class="form-field">
              <span class="form-label">同桌分组<span class="form-label-note">选填</span></span>
              <ElInput v-model="positionForm.desk_group" :disabled="saving" placeholder="选填，可留空" />
            </div>
            <div class="form-field">
              <span class="form-label">状态</span>
              <ElSelect v-model="positionForm.active" :disabled="saving">
                <ElOption label="正常" :value="1" />
                <ElOption label="停用" :value="0" />
              </ElSelect>
            </div>
          </div>
          <div class="drawer-footer" style="margin-top: 12px">
            <ElButton size="small" :disabled="saving" @click="positionForm = null">取消</ElButton>
            <ElButton size="small" type="primary" :loading="saving" @click="savePositionRow">保存位置</ElButton>
          </div>
        </div>

        <ElTable :data="visiblePositions" border size="small" max-height="460" @selection-change="positionSelection = $event">
          <ElTableColumn type="selection" width="42" />
          <ElTableColumn prop="row_number" label="行号" width="70" />
          <ElTableColumn prop="column_number" label="列号" width="70" />
          <ElTableColumn label="类型" width="90">
            <template #default="{ row }">
              {{ { seat: '座位', aisle: '过道', unavailable: '不可用' }[row.kind] ?? row.kind }}
            </template>
          </ElTableColumn>
          <ElTableColumn prop="desk_group" label="同桌分组" min-width="110" />
          <ElTableColumn label="状态" width="84">
            <template #default="{ row }">
              <ElTag size="small" :type="row.active ? 'success' : 'info'">{{ row.active ? '正常' : '停用' }}</ElTag>
            </template>
          </ElTableColumn>
          <ElTableColumn label="操作" width="80" fixed="right">
            <template #default="{ row }">
              <ElButton link type="primary" size="small" :disabled="saving" @click="editPositionRow(row)">编辑</ElButton>
            </template>
          </ElTableColumn>
        </ElTable>
      </div>
    </ElDrawer>
  </div>
</template>

<style scoped>
.copy-checks {
  display: flex;
  flex-wrap: wrap;
  gap: 12px 20px;
  padding: 10px 12px;
  background: #f7faf9;
  border-radius: 8px;
}

.error-list {
  margin: 4px 0 0;
  padding-left: 18px;
  font-size: 12px;
}
</style>
