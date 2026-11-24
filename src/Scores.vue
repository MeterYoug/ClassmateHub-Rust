<script setup>
import { computed, onMounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  ElAlert,
  ElButton,
  ElCheckbox,
  ElDrawer,
  ElEmpty,
  ElInput,
  ElInputNumber,
  ElOption,
  ElSelect,
  ElSwitch,
  ElTable,
  ElTableColumn,
  ElTabPane,
  ElTabs,
  ElTag,
} from 'element-plus'
import { Check, Download, Edit, Plus, Search, Upload } from '@element-plus/icons-vue'
import { classGrades, classSubjects, subjectsForClass } from './classOptions.js'
import { downloadWorkbook, workbookBytes } from './workbook.js'
import { confirmAction, notifyError, notifyInfo, notifySuccess, notifyWarning } from './ui.js'

const props = defineProps({
  classId: { type: Number, default: null },
  classes: { type: Array, default: () => [] },
})

const activeTab = ref('scores')
const exams = ref([])
const examId = ref(null)
const rows = ref([])
const templates = ref([])
const editor = ref(null)
const originalExam = ref(null)
const templateEditor = ref(null)
const originalTemplate = ref(null)
const preview = ref(null)
const updateExisting = ref(false)
const exportByStudentNo = ref(false)
const scoreSearch = ref('')
const levelFilter = ref('')
const saving = ref(false)
const scoreDraft = ref(null)
const scoreSnapshot = ref(null)
const fileInput = ref(null)
const examErrors = ref({})
const templateErrors = ref({})

function clearExamError(key) {
  if (examErrors.value[key]) {
    const next = { ...examErrors.value }
    delete next[key]
    examErrors.value = next
  }
}

function clearTemplateError(key) {
  if (templateErrors.value[key]) {
    const next = { ...templateErrors.value }
    delete next[key]
    templateErrors.value = next
  }
}

function validateExam() {
  const errors = {}
  if (!(editor.value?.name ?? '').trim()) errors.name = '请填写考试名称'
  if (!editor.value?.rank_grading) {
    const [a, b, c, d] = (editor.value?.thresholds ?? []).map(Number)
    const validThresholds =
      [a, b, c, d].every(n => Number.isFinite(n)) &&
      100 >= a && a > b && b > c && c > d && d >= 0
    if (!validThresholds) errors.thresholds = '请填写 0 至 100 之间且按 A、B、C、D 严格递减的得分率下限'
  }
  const subjects = editor.value?.subjects ?? []
  if (!subjects.length) {
    errors.subjects = '请至少添加一门考试科目'
  } else {
    const seen = new Set()
    subjects.forEach((subject, index) => {
      const name = (subject.name ?? '').trim()
      if (!name) {
        errors[`subject_name_${index}`] = '请填写科目名称'
      } else if (seen.has(name)) {
        errors[`subject_name_${index}`] = '科目名称不能重复'
      } else {
        seen.add(name)
      }
      try {
        cents(subject.display_full)
      } catch (error) {
        errors[`subject_full_${index}`] = error.message || '请填写大于 0 的有效满分'
      }
    })
  }
  examErrors.value = errors
  if (Object.keys(errors).length) {
    notifyError('请先完善标红的必填项或修正输入内容')
    return false
  }
  return true
}

function validateTemplate() {
  const errors = {}
  if (!(templateEditor.value?.name ?? '').trim()) errors.name = '请选择或输入科目名称'
  try {
    cents(templateEditor.value?.display_full ?? '')
  } catch (error) {
    errors.display_full = error.message || '请填写大于 0 的有效满分'
  }
  templateErrors.value = errors
  if (Object.keys(errors).length) {
    notifyError('请先完善标红的必填项或修正输入内容')
    return false
  }
  return true
}

const activeExam = computed(() => exams.value.find(exam => exam.id === examId.value) ?? null)

const filteredRows = computed(() => {
  const query = scoreSearch.value.trim().toLowerCase()
  return rows.value
    .filter(
      row =>
        (!levelFilter.value || row.total_grade === levelFilter.value) &&
        (!query || row.student_no.toLowerCase().includes(query) || row.name.toLowerCase().includes(query)),
    )
    .sort((left, right) => (left.total_rank ?? Infinity) - (right.total_rank ?? Infinity))
})

const scoredCount = computed(() => rows.value.filter(row => row.details.length > 0).length)
const initialForm = ref('')
const dirty = computed(() => {
  if (preview.value) return true
  if (scoreDraft.value) {
    const { input, remarks } = computeScoreDiff()
    return input.length > 0 || remarks.length > 0
  }
  const form = editor.value ?? templateEditor.value
  return form != null && JSON.stringify(form) !== initialForm.value
})

const nextActiveStudentRow = computed(() => {
  if (!scoreDraft.value) return null
  const activeList = filteredRows.value.filter(row => row.active)
  const index = activeList.findIndex(row => row.student_id === scoreDraft.value.student_id)
  if (index === -1 || index + 1 >= activeList.length) return null
  return activeList[index + 1]
})

defineExpose({
  isBusy: () => saving.value,
  hasUnsavedChanges: () => dirty.value || saving.value,
})

async function canLeave() {
  if (!dirty.value) return true
  return confirmAction('当前有未保存的编辑或预检内容，确定放弃吗？', {
    title: '放弃未保存修改',
    confirmText: '放弃',
    type: 'warning',
    danger: true,
  })
}

let closePending = false
async function closeDrawer(done) {
  if (saving.value || closePending) return
  closePending = true
  try {
    if (!(await canLeave()) || saving.value) return
    scoreDraft.value = null
    editor.value = null
    templateEditor.value = null
    done?.()
  } finally {
    closePending = false
  }
}

function money(value) {
  return value == null ? '—' : (value / 100).toFixed(2).replace(/\.00$/, '')
}

function cents(value) {
  const text = String(value).trim()
  if (!/^\d+(\.\d{1,2})?$/.test(text)) throw new Error('分数必须为最多两位小数的非负数字')
  const [whole, fraction = ''] = text.split('.')
  const result = Number(whole) * 100 + Number(fraction.padEnd(2, '0'))
  if (!Number.isSafeInteger(result) || result < 1 || result > 100000000) throw new Error('满分必须大于零且不超过 1000000')
  return result
}

function gradeTagType(grade) {
  if (grade === 'A') return 'success'
  if (grade === 'B') return 'primary'
  if (grade === 'C') return 'warning'
  if (grade === 'D' || grade === 'E') return 'danger'
  return 'info'
}

function detailFor(row, subjectId) {
  return row.details.find(item => item.subject_id === subjectId) ?? null
}

function initialSubjectValue(row, subjectId) {
  const detail = detailFor(row, subjectId)
  if (!detail) return ''
  if (detail.status === 'absent') return '缺考'
  return detail.score == null ? '' : money(detail.score)
}

async function run(action) {
  saving.value = true
  try {
    await action()
  } catch (error) {
    notifyError(error)
  } finally {
    saving.value = false
  }
}

async function refreshExams() {
  exams.value = props.classId ? await invoke('list_exams', { classId: Number(props.classId) }) : []
  if (!exams.value.some(exam => exam.id === examId.value)) {
    examId.value = exams.value.find(exam => exam.is_current)?.id ?? exams.value[0]?.id ?? null
  }
  await refreshRows()
}

async function refreshRows() {
  rows.value = examId.value ? await invoke('list_scores', { examId: Number(examId.value) }) : []
  scoreSnapshot.value = activeExam.value ? JSON.stringify([activeExam.value, rows.value]) : null
}

async function changeExam(nextId) {
  if (nextId === examId.value) return
  if (!(await canLeave())) return
  examId.value = nextId ? Number(nextId) : null
  editor.value = null
  templateEditor.value = null
  preview.value = null
  scoreDraft.value = null
  await run(refreshRows)
}

async function newExam() {
  if (!props.classId) return
  if (!(await canLeave())) return
  const selected = props.classes.find(item => item.id === props.classId)
  editor.value = {
    id: 0,
    class_id: Number(props.classId),
    name: '',
    exam_date: '',
    remark: '',
    is_current: exams.value.length === 0,
    thresholds: [90, 80, 70, 60],
    rank_grading: true,
    subjects: [],
  }
  for (const name of subjectsForClass(selected)) {
    const template = templates.value
      .filter(
        item =>
          item.active &&
          item.name === name &&
          (!item.school_stage || item.school_stage === selected?.school_stage) &&
          (!item.grade || item.grade === selected?.grade),
      )
      .sort((a, b) => Number(!!b.grade) - Number(!!a.grade) || Number(!!b.school_stage) - Number(!!a.school_stage))[0]
    addSubject(template ?? { name, full_score: 15000 })
  }
  originalExam.value = null
  initialForm.value = JSON.stringify(editor.value)
  examErrors.value = {}
  templateEditor.value = null
  preview.value = null
  scoreDraft.value = null
}

async function editExam() {
  if (!activeExam.value) return
  if (!(await canLeave())) return
  editor.value = JSON.parse(JSON.stringify(activeExam.value))
  originalExam.value = JSON.parse(JSON.stringify(activeExam.value))
  examErrors.value = {}
  scoreDraft.value = null
  preview.value = null
  templateEditor.value = null
  editor.value.subjects = editor.value.subjects.map(subject => ({ ...subject, display_full: money(subject.full_score) }))
  initialForm.value = JSON.stringify(editor.value)
}

function addSubject(template) {
  editor.value.subjects.push({
    id: 0,
    name: template?.name ?? '',
    full_score: template?.full_score ?? 10000,
    display_full: money(template?.full_score ?? 10000),
    counts_toward_total: true,
  })
}

async function saveExam() {
  if (!validateExam()) return
  if (
    editor.value.id &&
    activeExam.value &&
    (editor.value.rank_grading !== activeExam.value.rank_grading ||
      JSON.stringify(editor.value.thresholds) !== JSON.stringify(activeExam.value.thresholds))
  ) {
    const ok = await confirmAction('本次保存会按新的分级规则重算这场考试的成绩等级，确定继续吗？', {
      title: '重算成绩等级',
      confirmText: '确认保存并重算',
      type: 'warning',
    })
    if (!ok) return
  }
  await run(async () => {
    const exam = JSON.parse(JSON.stringify(editor.value))
    exam.thresholds = exam.thresholds.map(Number)
    exam.subjects = exam.subjects.map(subject => ({ ...subject, full_score: cents(subject.display_full) }))
    const id = await invoke('save_exam', { exam, expected: originalExam.value })
    editor.value = null
    examId.value = Number(id)
    await refreshExams()
    notifySuccess('考试配置已保存')
  })
}

function buildScoreDraft(row) {
  const initialValues = Object.fromEntries(
    activeExam.value.subjects.map(subject => [subject.id, initialSubjectValue(row, subject.id)]),
  )
  return {
    student_id: row.student_id,
    student_no: row.student_no,
    name: row.name,
    remark: row.remark,
    originalRemark: row.remark,
    values: { ...initialValues },
    originalValues: { ...initialValues },
  }
}

async function editScore(row) {
  if (!(await canLeave())) return
  editor.value = null
  templateEditor.value = null
  preview.value = null
  scoreDraft.value = buildScoreDraft(row)
}

function computeScoreDiff() {
  const input = []
  for (const subject of activeExam.value.subjects) {
    const orig = scoreDraft.value.originalValues[subject.id] ?? ''
    const curr = String(scoreDraft.value.values[subject.id] ?? '').trim()
    if (curr === orig) continue
    const submitValue = curr === '' ? '清空' : curr
    input.push({
      student_id: scoreDraft.value.student_id,
      subject_id: subject.id,
      value: submitValue,
    })
  }
  const remarks =
    scoreDraft.value.remark !== scoreDraft.value.originalRemark
      ? [{ student_id: scoreDraft.value.student_id, remark: scoreDraft.value.remark }]
      : []
  return { input, remarks }
}

async function saveScore(advanceNext = false) {
  const nextId = advanceNext ? nextActiveStudentRow.value?.student_id ?? null : null
  const { input, remarks } = computeScoreDiff()
  if (!input.length && !remarks.length) {
    if (advanceNext && nextId) {
      const nextRow = rows.value.find(row => row.student_id === nextId)
      if (nextRow) {
        scoreDraft.value = buildScoreDraft(nextRow)
        notifyInfo(`已跳过无变动的记录，切换到「${nextRow.name}」`)
        return
      }
    }
    notifyWarning('成绩和备注没有变化')
    return
  }
  await run(async () => {
    const studentName = scoreDraft.value.name
    await invoke('save_scores', {
      examId: Number(examId.value),
      input,
      remarks,
      expected: scoreSnapshot.value,
    })
    await refreshRows()
    if (advanceNext && nextId) {
      const nextRow = rows.value.find(row => row.student_id === nextId)
      if (nextRow) {
        scoreDraft.value = buildScoreDraft(nextRow)
        notifySuccess(`已保存「${studentName}」成绩，继续录入「${nextRow.name}」`)
        return
      }
    }
    scoreDraft.value = null
    notifySuccess(`「${studentName}」成绩已保存`)
  })
}

async function removeScore(row) {
  if (!(await canLeave())) return
  const ok = await confirmAction(`确定删除「${row.name}」在本次考试中的全部成绩与备注？删除后班级排名会重新计算。`, {
    title: '删除学生成绩',
    confirmText: '确认删除',
    type: 'warning',
    danger: true,
  })
  if (!ok) return
  await run(async () => {
    await invoke('remove_score', {
      examId: Number(examId.value),
      studentId: row.student_id,
      expected: scoreSnapshot.value,
    })
    editor.value = null
    templateEditor.value = null
    preview.value = null
    scoreDraft.value = null
    await refreshRows()
    notifySuccess('成绩已删除，排名已更新')
  })
}

async function editTemplate(item = null) {
  if (!(await canLeave())) return
  originalTemplate.value = item ? JSON.parse(JSON.stringify(item)) : null
  templateEditor.value = item
    ? { ...item, display_full: money(item.full_score) }
    : { id: 0, name: '', school_stage: '', grade: '', display_full: '100', active: true }
  initialForm.value = JSON.stringify(templateEditor.value)
  templateErrors.value = {}
  editor.value = null
  preview.value = null
  scoreDraft.value = null
}

async function saveTemplate() {
  if (!validateTemplate()) return
  await run(async () => {
    const subject = { ...templateEditor.value, full_score: cents(templateEditor.value.display_full) }
    await invoke('save_subject_template', { subject, expected: originalTemplate.value })
    templateEditor.value = null
    templates.value = await invoke('list_subject_templates')
    notifySuccess('科目模板已保存')
  })
}

async function download(exportScores = false) {
  if (!examId.value) return
  await run(async () => {
    const data = await invoke(exportScores ? 'export_scores' : 'score_template', {
      examId: Number(examId.value),
      byStudentNo: exportByStudentNo.value,
    })
    downloadWorkbook(data, exportScores ? '考试成绩.xlsx' : '成绩导入模板.xlsx')
    notifySuccess(exportScores ? '成绩表已导出' : '成绩导入模板已下载')
  })
}

async function syncLevels() {
  if (!examId.value || !(await canLeave())) return
  const ok = await confirmAction('将本场考试的总成绩等级同步写入学生档案，供「成绩强弱均衡」规则使用，确定继续吗？', {
    title: '同步成绩等级',
    confirmText: '确认同步',
    type: 'info',
  })
  if (!ok) return
  await run(async () => {
    const count = await invoke('sync_score_levels', { examId: Number(examId.value) })
    notifySuccess(`已同步 ${count} 名学生的成绩等级到学生档案`)
  })
}

async function setCurrent() {
  if (!activeExam.value || !(await canLeave())) return
  await run(async () => {
    const exam = activeExam.value
    await invoke('save_exam', { exam: { ...exam, is_current: true }, expected: exam })
    await refreshExams()
    notifySuccess('已设为本班当前考试')
  })
}

function triggerFilePick() {
  fileInput.value?.click()
}

async function readFile(event) {
  const file = event.target.files?.[0]
  event.target.value = ''
  if (!file) return
  if (!(await canLeave())) return
  await run(async () => {
    preview.value = null
    preview.value = await invoke('preview_scores', { examId: Number(examId.value), bytes: await workbookBytes(file) })
    if (!updateExisting.value) {
      const existing = new Set(rows.value.filter(row => row.details.length > 0 || row.remark).map(row => row.student_id))
      const count = new Set(
        [...preview.value.changes, ...preview.value.remarks]
          .filter(change => existing.has(change.student_id))
          .map(change => change.student_id),
      ).size
      if (count) preview.value.errors.push(`${count} 名学生已有成绩，勾选「按学号更新已有成绩」后重新选择文件。`)
    }
    editor.value = null
    templateEditor.value = null
    scoreDraft.value = null
    if (preview.value.errors.length) notifyWarning(`预检发现 ${preview.value.errors.length} 项错误，请修正后重新导入`)
    else notifyInfo('预检通过，核对统计后点击「确认导入」')
  })
}

async function importPreview() {
  if (!preview.value || preview.value.errors.length) return
  await run(async () => {
    await invoke('save_scores', {
      examId: Number(examId.value),
      input: preview.value.changes,
      remarks: preview.value.remarks,
      expected: preview.value.snapshot,
    })
    preview.value = null
    await refreshRows()
    activeTab.value = 'scores'
    notifySuccess('成绩导入成功，排名与等级已更新')
  })
}

watch(
  () => props.classId,
  () => {
    examId.value = null
    editor.value = null
    templateEditor.value = null
    preview.value = null
    scoreDraft.value = null
    void run(refreshExams)
  },
)

onMounted(() =>
  run(async () => {
    templates.value = await invoke('list_subject_templates')
    await refreshExams()
  }),
)
</script>

<template>
  <div class="page-container">
    <div v-if="!classId" class="page-card">
      <ElEmpty description="请先创建或选择一个班级后再管理考试与成绩" />
    </div>

    <template v-else>
      <section class="exam-bar page-card">
        <div class="exam-bar-left">
          <span class="form-label">当前考试</span>
          <ElSelect
            :model-value="examId"
            :disabled="saving || !exams.length"
            placeholder="暂无考试，请先新建"
            style="width: 260px"
            @change="changeExam"
          >
            <ElOption
              v-for="exam in exams"
              :key="exam.id"
              :label="`${exam.name}${exam.is_current ? '（当前）' : ''}`"
              :value="exam.id"
            />
          </ElSelect>
          <ElTag v-if="activeExam" size="small" :type="activeExam.rank_grading ? 'success' : 'info'" effect="light">
            {{ activeExam.rank_grading ? '四档排名分级' : '得分率分级' }}
          </ElTag>
          <span v-if="activeExam?.exam_date" class="form-hint">{{ activeExam.exam_date }}</span>
        </div>

        <div class="action-group">
          <ElButton type="primary" :icon="Plus" :disabled="saving" @click="newExam">新建考试</ElButton>
          <ElButton :icon="Edit" :disabled="saving || !activeExam" @click="editExam">编辑考试</ElButton>
          <ElButton :disabled="saving || !activeExam || activeExam.is_current" @click="setCurrent">设为当前</ElButton>
          <ElButton type="success" plain :icon="Check" :disabled="saving || !activeExam" @click="syncLevels">
            同步等级到学生档案
          </ElButton>
        </div>
      </section>

      <div class="page-card tabs-shell">
        <ElTabs v-model="activeTab">
          <ElTabPane :label="`逐生成绩 (${scoredCount}/${rows.length})`" name="scores">
            <template v-if="activeExam">
              <div class="filter-bar">
                <ElInput
                  v-model="scoreSearch"
                  clearable
                  placeholder="搜索学号或姓名"
                  :prefix-icon="Search"
                  style="width: 220px"
                />
                <div class="filter-item">
                  <span>成绩等级</span>
                  <ElSelect v-model="levelFilter" clearable placeholder="全部" style="width: 110px">
                    <ElOption label="A" value="A" />
                    <ElOption label="B" value="B" />
                    <ElOption label="C" value="C" />
                    <ElOption label="D" value="D" />
                    <ElOption v-if="!activeExam.rank_grading" label="E" value="E" />
                  </ElSelect>
                </div>
                <span class="form-hint" style="margin-left: auto">
                  已录入 {{ scoredCount }} / {{ rows.length }} 人
                </span>
              </div>

              <ElTable :data="filteredRows" border stripe empty-text="当前筛选条件下暂无学生记录">
                <ElTableColumn prop="student_no" label="学号" width="110" fixed="left" />
                <ElTableColumn label="姓名" width="110" fixed="left">
                  <template #default="{ row }">
                    <span>{{ row.name }}</span>
                    <ElTag v-if="!row.active" size="small" type="info" style="margin-left: 4px">停用</ElTag>
                  </template>
                </ElTableColumn>

                <ElTableColumn
                  v-for="subject in activeExam.subjects"
                  :key="subject.id"
                  :label="`${subject.name} (${money(subject.full_score)})`"
                  min-width="116"
                >
                  <template #default="{ row }">
                    <template v-if="detailFor(row, subject.id)">
                      <ElTag v-if="detailFor(row, subject.id).status === 'absent'" type="warning" size="small">缺考</ElTag>
                      <div v-else class="score-cell">
                        <strong>{{ money(detailFor(row, subject.id).score) }}</strong>
                        <span class="score-sub">
                          第 {{ detailFor(row, subject.id).rank }} 名 · {{ detailFor(row, subject.id).grade }}
                        </span>
                      </div>
                    </template>
                    <span v-else class="unentered">未录入</span>
                  </template>
                </ElTableColumn>

                <ElTableColumn label="总分" width="88">
                  <template #default="{ row }">
                    <strong>{{ money(row.total_score) }}</strong>
                  </template>
                </ElTableColumn>
                <ElTableColumn label="排名" width="76">
                  <template #default="{ row }">{{ row.total_rank ?? '—' }}</template>
                </ElTableColumn>
                <ElTableColumn label="等级" width="76">
                  <template #default="{ row }">
                    <ElTag v-if="row.total_grade" :type="gradeTagType(row.total_grade)" size="small" effect="dark">
                      {{ row.total_grade }}
                    </ElTag>
                    <span v-else>—</span>
                  </template>
                </ElTableColumn>
                <ElTableColumn prop="remark" label="备注" min-width="120" show-overflow-tooltip />
                <ElTableColumn label="操作" width="140" fixed="right">
                  <template #default="{ row }">
                    <ElButton link type="primary" :disabled="saving || !row.active" @click="editScore(row)">
                      {{ row.details.length ? '修改' : '录分' }}
                    </ElButton>
                    <ElButton
                      v-if="row.details.length || row.remark"
                      link
                      type="danger"
                      :disabled="saving"
                      @click="removeScore(row)"
                    >
                      删除
                    </ElButton>
                  </template>
                </ElTableColumn>
              </ElTable>
            </template>
            <ElEmpty v-else description="当前班级还没有考试，点击上方「新建考试」开始录入成绩" />
          </ElTabPane>

          <ElTabPane label="导入与导出" name="io">
            <template v-if="activeExam">
              <div class="io-grid">
                <div class="io-card">
                  <h3>批量导入成绩（.xlsx）</h3>
                  <p class="form-hint">
                    支持原版成绩表表头，学号请保留文本格式。空白单元格保留原成绩，填写「缺考」或「清空」可修改状态。导入先预检，存在错误时整批不写入。
                  </p>
                  <div class="io-controls">
                    <ElCheckbox v-model="updateExisting" :disabled="saving" @change="preview = null">
                      按学号更新已有成绩
                    </ElCheckbox>
                    <input ref="fileInput" type="file" accept=".xlsx" class="hidden-file-input" @change="readFile" />
                    <ElButton type="primary" :icon="Upload" :loading="saving" @click="triggerFilePick">
                      选择 Excel 成绩文件并预检
                    </ElButton>
                  </div>
                </div>

                <div class="io-card">
                  <h3>下载模板与导出成绩</h3>
                  <p class="form-hint">可下载当前考试对应科目的导入模板，或导出包含总分、排名与等级的成绩表。</p>
                  <div class="io-controls">
                    <ElCheckbox v-model="exportByStudentNo" :disabled="saving">按学号排序导出</ElCheckbox>
                    <div class="action-group">
                      <ElButton :icon="Download" :disabled="saving" @click="download(false)">下载成绩模板</ElButton>
                      <ElButton type="primary" plain :icon="Download" :disabled="saving" @click="download(true)">
                        导出当前考试成绩
                      </ElButton>
                    </div>
                  </div>
                </div>
              </div>

              <div v-if="preview" class="preview-box">
                <div class="preview-head">
                  <h3>导入预检结果</h3>
                  <div class="action-group">
                    <ElTag type="success" effect="light">新增 {{ preview.additions }}</ElTag>
                    <ElTag type="primary" effect="light">覆盖 {{ preview.replacements }}</ElTag>
                    <ElTag type="warning" effect="light">清空 {{ preview.clears }}</ElTag>
                    <ElTag type="info" effect="light">备注 {{ preview.remarks.length }}</ElTag>
                    <ElTag :type="preview.errors.length ? 'danger' : 'success'" effect="dark">
                      错误 {{ preview.errors.length }}
                    </ElTag>
                  </div>
                </div>
                <ElAlert v-if="preview.errors.length" type="error" show-icon :closable="false" title="存在以下错误，修复后请重新选择文件">
                  <ul class="error-list">
                    <li v-for="(error, index) in preview.errors" :key="index">{{ error }}</li>
                  </ul>
                </ElAlert>
                <div class="drawer-footer" style="margin-top: 14px">
                  <ElButton :disabled="saving" @click="preview = null">取消</ElButton>
                  <ElButton type="primary" :loading="saving" :disabled="preview.errors.length > 0" @click="importPreview">
                    确认导入（{{ preview.changes.length }} 项变更）
                  </ElButton>
                </div>
              </div>
            </template>
            <ElEmpty v-else description="请先选择或新建一场考试" />
          </ElTabPane>

          <ElTabPane :label="`科目模板 (${templates.length})`" name="templates">
            <div class="action-bar">
              <ElButton type="primary" :icon="Plus" :disabled="saving" @click="editTemplate()">新增科目模板</ElButton>
              <span class="form-hint">新建考试时会按班级的学段和年级自动匹配对应科目的默认满分。</span>
            </div>
            <ElTable :data="templates" border empty-text="暂无科目模板">
              <ElTableColumn prop="name" label="科目名称" min-width="140" />
              <ElTableColumn label="适用学段" width="120">
                <template #default="{ row }">{{ row.school_stage || '不限' }}</template>
              </ElTableColumn>
              <ElTableColumn label="适用年级" width="130">
                <template #default="{ row }">{{ row.grade || '不限' }}</template>
              </ElTableColumn>
              <ElTableColumn label="默认满分" width="110">
                <template #default="{ row }">{{ money(row.full_score) }}</template>
              </ElTableColumn>
              <ElTableColumn label="状态" width="90">
                <template #default="{ row }">
                  <ElTag :type="row.active ? 'success' : 'info'" size="small">{{ row.active ? '正常' : '停用' }}</ElTag>
                </template>
              </ElTableColumn>
              <ElTableColumn label="操作" width="100" fixed="right">
                <template #default="{ row }">
                  <ElButton link type="primary" :disabled="saving" @click="editTemplate(row)">编辑</ElButton>
                </template>
              </ElTableColumn>
            </ElTable>
          </ElTabPane>
        </ElTabs>
      </div>
    </template>

    <!-- 单人录分抽屉（回显原分 + 快捷缺考/清空 + 保存并下一位） -->
    <ElDrawer
      :model-value="!!scoreDraft"
      :title="scoreDraft ? `录入成绩 · ${scoreDraft.name}（${scoreDraft.student_no}）` : '录入成绩'"
      size="min(92vw, 520px)"
      :before-close="closeDrawer"
    >
      <div v-if="scoreDraft && activeExam" class="drawer-form">
        <p class="form-hint">已回显当前已有分数。仅提交改动科目；清空输入框或点击「清空」可删除该科成绩。</p>
        <div v-for="subject in activeExam.subjects" :key="subject.id" class="subject-score-row">
          <div class="subject-meta">
            <strong>{{ subject.name }}</strong>
            <span class="form-hint">满分 {{ money(subject.full_score) }}</span>
          </div>
          <div class="subject-input-group">
            <ElInput
              v-model="scoreDraft.values[subject.id]"
              placeholder="输入分数、缺考或留空清空"
              :disabled="saving"
              style="flex: 1"
            />
            <ElButton size="small" plain :disabled="saving" @click="scoreDraft.values[subject.id] = '缺考'">缺考</ElButton>
            <ElButton size="small" plain :disabled="saving" @click="scoreDraft.values[subject.id] = ''">清空</ElButton>
          </div>
        </div>
        <div class="form-field">
          <span class="form-label">考试备注</span>
          <ElInput v-model="scoreDraft.remark" maxlength="500" :disabled="saving" placeholder="可选备注" />
        </div>
      </div>
      <template #footer>
        <div class="drawer-footer">
          <ElButton :disabled="saving" @click="closeDrawer()">取消</ElButton>
          <template v-if="nextActiveStudentRow">
            <ElButton :loading="saving" @click="saveScore(false)">仅保存</ElButton>
            <ElButton type="primary" :loading="saving" @click="saveScore(true)">
              保存并录入下一位（{{ nextActiveStudentRow.name }}）
            </ElButton>
          </template>
          <ElButton v-else type="primary" :loading="saving" @click="saveScore(false)">保存成绩</ElButton>
        </div>
      </template>
    </ElDrawer>

    <!-- 新建／编辑考试抽屉 -->
    <ElDrawer
      :model-value="!!editor"
      :title="editor?.id ? '编辑考试' : '新建考试'"
      size="min(94vw, 660px)"
      :before-close="closeDrawer"
    >
      <div v-if="editor" class="drawer-form">
        <p class="required-legend"><span>*</span> 为必填项</p>
        <div class="form-grid-2">
          <div class="form-field" :class="{ 'is-error': !!examErrors.name }">
            <span class="form-label is-required">考试名称</span>
            <ElInput
              v-model="editor.name"
              :disabled="saving"
              placeholder="请输入考试名称，例如：2026 秋季期中考试"
              @input="clearExamError('name')"
            />
            <p v-if="examErrors.name" class="form-error">{{ examErrors.name }}</p>
          </div>
          <div class="form-field">
            <span class="form-label">考试日期<span class="form-label-note">选填</span></span>
            <ElInput v-model="editor.exam_date" type="date" :disabled="saving" />
          </div>
          <div class="form-field full-span">
            <span class="form-label">备注<span class="form-label-note">选填</span></span>
            <ElInput v-model="editor.remark" :disabled="saving" placeholder="选填说明" />
          </div>
        </div>

        <div class="inline-switches">
          <ElCheckbox v-model="editor.is_current" :disabled="saving">设为本班当前考试</ElCheckbox>
          <ElCheckbox v-if="editor.id && !activeExam?.rank_grading" v-model="editor.rank_grading" :disabled="saving" @change="clearExamError('thresholds')">
            改用按班级排名四档分级（A／B／C／D）
          </ElCheckbox>
        </div>

        <ElAlert
          v-if="editor.rank_grading"
          type="info"
          show-icon
          :closable="false"
          title="总成绩将按班级总排名平均划分为 A、B、C、D 四档。"
        />
        <div v-else class="threshold-box" :class="{ 'is-field-error': !!examErrors.thresholds }">
          <span class="form-label is-required">等级得分率下限<span class="form-label-note">按 A、B、C、D 严格递减填写，单位 %</span></span>
          <div class="threshold-grid">
            <div v-for="(_, index) in editor.thresholds" :key="index" class="form-field">
              <span class="form-hint">{{ ['A', 'B', 'C', 'D'][index] }} 档下限</span>
              <ElInputNumber
                v-model="editor.thresholds[index]"
                :min="0"
                :max="100"
                controls-position="right"
                style="width: 100%"
                @change="clearExamError('thresholds')"
              />
            </div>
          </div>
          <p v-if="examErrors.thresholds" class="form-error">{{ examErrors.thresholds }}</p>
        </div>

        <div class="subjects-section">
          <div class="subjects-head">
            <div>
              <span class="form-label is-required">考试科目快照<span class="form-label-note">至少 1 门科目</span></span>
              <p class="form-hint">创建后科目与满分将锁定为快照，防止历史成绩漂移。</p>
            </div>
            <ElButton v-if="!editor.id" size="small" type="primary" plain :icon="Plus" @click="addSubject(); clearExamError('subjects')">
              新增空白科目
            </ElButton>
          </div>
          <div v-if="!editor.id && templates.some(t => t.active)" class="template-chips">
            <span class="form-hint">快捷添加模板：</span>
            <ElButton
              v-for="item in templates.filter(t => t.active)"
              :key="item.id"
              size="small"
              round
              @click="addSubject(item); clearExamError('subjects')"
            >
              + {{ item.name }} ({{ money(item.full_score) }})
            </ElButton>
          </div>

          <p v-if="examErrors.subjects" class="form-error">{{ examErrors.subjects }}</p>
          <div class="subject-rows">
            <div class="subject-row-head">
              <span class="form-label is-required">科目名称</span>
              <span class="form-label is-required">满分</span>
              <span>计入总分</span>
              <span></span>
            </div>
            <div v-for="(subject, index) in editor.subjects" :key="index" class="subject-row-block">
              <div class="subject-row-item">
                <ElInput
                  v-model="subject.name"
                  size="small"
                  :disabled="!!editor.id"
                  :class="{ 'is-field-error': !!examErrors[`subject_name_${index}`] }"
                  placeholder="请输入科目名称"
                  @input="clearExamError(`subject_name_${index}`)"
                />
                <ElInput
                  v-model="subject.display_full"
                  size="small"
                  :disabled="!!editor.id"
                  :class="{ 'is-field-error': !!examErrors[`subject_full_${index}`] }"
                  placeholder="请输入满分"
                  @input="clearExamError(`subject_full_${index}`)"
                />
                <ElSwitch v-model="subject.counts_toward_total" size="small" :disabled="!!editor.id" />
                <ElButton v-if="!editor.id" link type="danger" size="small" @click="editor.subjects.splice(index, 1); examErrors = {}">移除</ElButton>
              </div>
              <p v-if="examErrors[`subject_name_${index}`] || examErrors[`subject_full_${index}`]" class="form-error">
                {{ [examErrors[`subject_name_${index}`], examErrors[`subject_full_${index}`]].filter(Boolean).join('；') }}
              </p>
            </div>
          </div>
        </div>
      </div>
      <template #footer>
        <div class="drawer-footer">
          <ElButton :disabled="saving" @click="closeDrawer()">取消</ElButton>
          <ElButton type="primary" :loading="saving" @click="saveExam">保存考试</ElButton>
        </div>
      </template>
    </ElDrawer>

    <!-- 科目模板抽屉 -->
    <ElDrawer
      :model-value="!!templateEditor"
      :title="templateEditor?.id ? '编辑科目模板' : '新增科目模板'"
      size="min(92vw, 480px)"
      :before-close="closeDrawer"
    >
      <div v-if="templateEditor" class="drawer-form">
        <p class="required-legend"><span>*</span> 为必填项</p>
        <div class="form-field" :class="{ 'is-error': !!templateErrors.name }">
          <span class="form-label is-required">科目名称</span>
          <ElSelect
            v-model="templateEditor.name"
            filterable
            allow-create
            default-first-option
            placeholder="请选择或输入科目名称"
            @change="clearTemplateError('name')"
          >
            <ElOption v-for="item in classSubjects" :key="item" :label="item" :value="item" />
            <ElOption
              v-if="originalTemplate?.name && !classSubjects.includes(originalTemplate.name)"
              :label="originalTemplate.name"
              :value="originalTemplate.name"
            />
          </ElSelect>
          <p v-if="templateErrors.name" class="form-error">{{ templateErrors.name }}</p>
        </div>
        <div class="form-field">
          <span class="form-label">适用年级<span class="form-label-note">选填</span></span>
          <ElSelect
            v-model="templateEditor.grade"
            clearable
            placeholder="不限年级"
            @change="templateEditor.school_stage = classGrades.find(item => item.name === templateEditor.grade)?.stage ?? ''"
          >
            <ElOption label="不限年级" value="" />
            <ElOption v-for="item in classGrades" :key="item.name" :label="item.name" :value="item.name" />
            <ElOption
              v-if="originalTemplate?.grade && !classGrades.some(item => item.name === originalTemplate.grade)"
              :label="originalTemplate.grade"
              :value="originalTemplate.grade"
            />
          </ElSelect>
        </div>
        <div class="form-field">
          <span class="form-label">适用学段<span class="form-label-note">选填</span></span>
          <ElSelect v-model="templateEditor.school_stage" :disabled="!!templateEditor.grade" placeholder="不限学段">
            <ElOption label="不限学段" value="" />
            <ElOption label="小学" value="小学" />
            <ElOption label="初中" value="初中" />
            <ElOption label="高中" value="高中" />
            <ElOption
              v-if="originalTemplate?.school_stage && !['小学', '初中', '高中'].includes(originalTemplate.school_stage)"
              :label="originalTemplate.school_stage"
              :value="originalTemplate.school_stage"
            />
          </ElSelect>
        </div>
        <div class="form-field" :class="{ 'is-error': !!templateErrors.display_full }">
          <span class="form-label is-required">默认满分</span>
          <ElInput
            v-model="templateEditor.display_full"
            placeholder="请输入大于 0 的满分，例如：150"
            @input="clearTemplateError('display_full')"
          />
          <p v-if="templateErrors.display_full" class="form-error">{{ templateErrors.display_full }}</p>
        </div>
        <div class="form-field">
          <span class="form-label">状态</span>
          <ElSwitch v-model="templateEditor.active" active-text="正常" inactive-text="停用" />
        </div>
      </div>
      <template #footer>
        <div class="drawer-footer">
          <ElButton :disabled="saving" @click="closeDrawer()">取消</ElButton>
          <ElButton type="primary" :loading="saving" @click="saveTemplate">保存模板</ElButton>
        </div>
      </template>
    </ElDrawer>
  </div>
</template>

<style scoped>
.exam-bar {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  margin-bottom: 16px;
}

.exam-bar-left {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
}

.tabs-shell {
  padding-top: 10px;
}

.score-cell {
  display: flex;
  flex-direction: column;
  line-height: 1.3;
}

.score-sub {
  font-size: 11px;
  color: #70857e;
}

.unentered {
  font-size: 12px;
  color: #a3b3ad;
}

.io-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
}

.io-card {
  padding: 18px;
  background: #f8fbfa;
  border: 1px solid #e6efeb;
  border-radius: 10px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.io-card h3 {
  margin: 0;
  font-size: 15px;
}

.io-controls {
  margin-top: auto;
  display: grid;
  gap: 10px;
}

.preview-box {
  margin-top: 16px;
  padding: 16px;
  border: 1px solid #e2ece8;
  border-radius: 10px;
  background: #ffffff;
}

.preview-head {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
  margin-bottom: 12px;
}

.preview-head h3 {
  margin: 0;
  font-size: 15px;
}

.error-list {
  margin: 4px 0 0;
  padding-left: 18px;
  font-size: 12px;
}

.subject-score-row {
  display: grid;
  gap: 6px;
  padding: 10px 12px;
  background: #f8fbfa;
  border: 1px solid #e8f0ed;
  border-radius: 8px;
}

.subject-meta {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  font-size: 13px;
}

.subject-input-group {
  display: flex;
  gap: 6px;
}

.inline-switches {
  display: flex;
  flex-wrap: wrap;
  gap: 16px;
}

.threshold-box {
  display: grid;
  gap: 8px;
  padding: 12px;
  background: #f7faf9;
  border-radius: 8px;
}

.threshold-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 10px;
}

.subjects-section {
  display: grid;
  gap: 10px;
  padding-top: 8px;
  border-top: 1px solid #ebf1ef;
}

.subjects-head {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  gap: 10px;
}

.template-chips {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 6px;
}

.subject-rows {
  display: grid;
  gap: 8px;
}

.subject-row-block {
  display: grid;
  gap: 4px;
}

.subject-row-head,
.subject-row-item {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 104px 72px 54px;
  align-items: center;
  gap: 8px;
}

.subject-row-head {
  font-size: 12px;
  color: #6b8079;
  padding: 0 2px;
}

@media (max-width: 900px) {
  .io-grid {
    grid-template-columns: 1fr;
  }
  .threshold-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}
</style>
