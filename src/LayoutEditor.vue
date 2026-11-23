<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { ElButton, ElInput, ElRadioButton, ElRadioGroup, ElTag, ElTooltip } from 'element-plus'
import { confirmAction, notifyError, notifyWarning } from './ui.js'

const props = defineProps({
  classroom: { type: Object, required: true },
  classId: { type: Number, required: true },
  inline: Boolean,
  disabled: Boolean,
})
const emit = defineEmits(['close', 'saved'])

const initial = ref(new Map())
const changes = ref(new Map())
const tool = ref('cycle')
const group = ref('')
const loading = ref(true)
const ready = ref(false)
const saving = ref(false)

const key = (row, column) => `${row}:${column}`
const rows = computed(() => Array.from({ length: Math.max(0, Math.min(50, Number(props.classroom.rows) || 0)) }, (_, index) => index + 1))
const columns = computed(() => Array.from({ length: Math.max(0, Math.min(50, Number(props.classroom.columns) || 0)) }, (_, index) => index + 1))
const labels = { seat: '座位', aisle: '过道', unavailable: '不可用' }
const podiumLabels = { front: '前方', back: '后方', left: '左侧', right: '右侧' }
const emptyLayout = computed(() => props.inline && !props.classroom.id)

const stats = computed(() => {
  let seat = 0
  let aisle = 0
  let unavailable = 0
  let disabledCount = 0
  let unset = 0
  for (const row of rows.value) {
    for (const column of columns.value) {
      const value = current(row, column)
      if (!value) unset += 1
      else if (!value.active) disabledCount += 1
      else if (value.kind === 'seat') seat += 1
      else if (value.kind === 'aisle') aisle += 1
      else unavailable += 1
    }
  }
  return { seat, aisle, unavailable, disabledCount, unset }
})

defineExpose({
  isBusy: () => saving.value,
  hasUnsavedChanges: () => changes.value.size > 0 || saving.value,
  changesForSave,
})

function beforeUnload(event) {
  if (changes.value.size) {
    event.preventDefault()
    event.returnValue = ''
  }
}

onMounted(async () => {
  window.addEventListener('beforeunload', beforeUnload)
  try {
    const positions = emptyLayout.value ? [] : await invoke('list_records', { resource: 'positions', classId: props.classId })
    initial.value = new Map(
      positions
        .filter(item => item.classroom_id === props.classroom.id)
        .map(item => [
          key(item.row_number, item.column_number),
          { id: item.id, kind: item.kind, desk_group: item.desk_group, active: item.active },
        ]),
    )
    ready.value = true
  } catch (error) {
    notifyError(`读取布局失败：${error}`)
  } finally {
    loading.value = false
  }
})
onBeforeUnmount(() => window.removeEventListener('beforeunload', beforeUnload))

function current(row, column) {
  const coordinate = key(row, column)
  return changes.value.get(coordinate) ?? initial.value.get(coordinate) ?? (emptyLayout.value ? { kind: 'seat', desk_group: '', active: 1 } : null)
}

function changesForSave() {
  if (!ready.value) throw new Error('请等教室布局读取完成')
  const next = [...changes.value.values()].filter(
    change => change.row_number <= Number(props.classroom.rows) && change.column_number <= Number(props.classroom.columns),
  )
  if (emptyLayout.value) {
    for (const row of rows.value) {
      for (const column of columns.value) {
        if (!changes.value.has(key(row, column))) {
          next.push({ row_number: row, column_number: column, expected: null, kind: 'seat', desk_group: '', active: 1 })
        }
      }
    }
  }
  return next
}

function applyCellToMap(map, row, column, kind, deskGroup, active) {
  const coordinate = key(row, column)
  const expected = initial.value.get(coordinate) ?? null
  if (expected && expected.kind === kind && expected.desk_group === deskGroup && expected.active === active) {
    map.delete(coordinate)
  } else {
    map.set(coordinate, { row_number: row, column_number: column, expected, kind, desk_group: deskGroup, active })
  }
}

function paint(row, column) {
  if (saving.value || props.disabled) return
  const value = current(row, column)
  const next = new Map(changes.value)
  if (tool.value === 'cycle') {
    const cycleNext = { seat: 'aisle', aisle: 'unavailable', unavailable: 'seat' }
    applyCellToMap(next, row, column, value?.active ? cycleNext[value.kind] : 'seat', '', 1)
  } else if (tool.value === 'disable') {
    if (!value) {
      notifyWarning('该位置尚未设置，无需停用')
      return
    }
    applyCellToMap(next, row, column, value.kind, value.desk_group, 0)
  } else if (tool.value === 'group') {
    if (!value || value.kind !== 'seat' || value.active !== 1) {
      notifyWarning('请先将该位置设置为正常座位，再设置同桌分组')
      return
    }
    applyCellToMap(next, row, column, value.kind, group.value.trim(), value.active)
  } else {
    applyCellToMap(next, row, column, tool.value, tool.value === 'seat' && value?.kind === 'seat' ? value.desk_group : '', 1)
  }
  changes.value = next
}

function paintLine(coords) {
  if (saving.value || props.disabled || !coords.length) return
  const next = new Map(changes.value)
  let targetKind = tool.value
  if (tool.value === 'cycle') {
    const seatCount = coords.filter(([r, c]) => {
      const value = current(r, c)
      return value?.active && value.kind === 'seat'
    }).length
    targetKind = seatCount >= coords.length / 2 ? 'aisle' : 'seat'
  }
  for (const [row, column] of coords) {
    const value = current(row, column)
    if (targetKind === 'disable') {
      if (value) applyCellToMap(next, row, column, value.kind, value.desk_group, 0)
    } else if (targetKind === 'group') {
      if (value?.active === 1 && value.kind === 'seat') {
        applyCellToMap(next, row, column, 'seat', group.value.trim(), 1)
      }
    } else {
      applyCellToMap(next, row, column, targetKind, targetKind === 'seat' && value?.kind === 'seat' ? value.desk_group : '', 1)
    }
  }
  changes.value = next
}

function paintRow(row) {
  paintLine(columns.value.map(column => [row, column]))
}

function paintColumn(column) {
  paintLine(rows.value.map(row => [row, column]))
}

function fillEmpty() {
  const next = new Map(changes.value)
  for (const row of rows.value) {
    for (const column of columns.value) {
      const coordinate = key(row, column)
      if (!next.has(coordinate) && !initial.value.has(coordinate)) {
        next.set(coordinate, { row_number: row, column_number: column, expected: null, kind: 'seat', desk_group: '', active: 1 })
      }
    }
  }
  changes.value = next
}

function resetChanges() {
  changes.value = new Map()
}

function cellClass(row, column) {
  const value = current(row, column)
  if (!value) return 'empty-cell'
  if (!value.active) return 'disabled-cell'
  return value.kind
}

function cellText(row, column) {
  const value = current(row, column)
  if (!value) return '未设置'
  if (!value.active) return '已停用'
  return value.desk_group && value.kind === 'seat' ? `座位 · ${value.desk_group}` : labels[value.kind]
}

async function close() {
  if (changes.value.size) {
    const ok = await confirmAction('布局有未保存的修改，确定放弃吗？', {
      title: '放弃布局修改',
      confirmText: '放弃修改',
      type: 'warning',
      danger: true,
    })
    if (!ok) return
  }
  emit('close')
}

async function save() {
  if (saving.value || !changes.value.size) return
  const existing = [...changes.value.values()].filter(change => change.expected).length
  if (existing) {
    const ok = await confirmAction(`将修改 ${existing} 个已有位置的设置，确定保存吗？`, {
      title: '保存教室布局',
      confirmText: '确认保存',
      type: 'warning',
    })
    if (!ok) return
  }
  saving.value = true
  try {
    await invoke('save_layout', {
      classId: props.classId,
      classroomId: props.classroom.id,
      expectedRows: props.classroom.rows,
      expectedColumns: props.classroom.columns,
      changes: [...changes.value.values()],
    })
    changes.value = new Map()
    emit('saved')
  } catch (error) {
    notifyError(`保存布局失败：${error}`)
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <section class="layout-editor-panel" :class="{ 'inline-mode': inline }">
    <div class="layout-top">
      <div>
        <h3>{{ inline ? '可视化教室布局' : `${classroom.name} · 教室布局` }}</h3>
        <p class="form-hint">
          {{ classroom.rows }} 行 × {{ classroom.columns }} 列 · 讲台在{{ podiumLabels[classroom.podium] ?? classroom.podium }}。点击格子绘制，点击行号／列号可整排／整列批量设置。
        </p>
      </div>
      <div class="layout-stats">
        <ElTag type="success" effect="light">有效座位 {{ stats.seat }}</ElTag>
        <ElTag type="info" effect="light">过道 {{ stats.aisle }}</ElTag>
        <ElTag v-if="stats.unavailable" type="danger" effect="light">不可用 {{ stats.unavailable }}</ElTag>
        <ElTag v-if="stats.disabledCount" type="warning" effect="light">已停用 {{ stats.disabledCount }}</ElTag>
        <ElTag v-if="changes.size" type="warning" effect="dark">待保存 {{ changes.size }} 处</ElTag>
      </div>
    </div>

    <p v-if="loading" class="form-hint">正在读取布局…</p>
    <template v-else-if="ready">
      <div class="brush-bar">
        <ElRadioGroup v-model="tool" size="small" :disabled="saving || disabled">
          <ElRadioButton value="cycle">点击切换</ElRadioButton>
          <ElRadioButton value="seat">设座位</ElRadioButton>
          <ElRadioButton value="aisle">设过道</ElRadioButton>
          <ElRadioButton value="unavailable">不可用</ElRadioButton>
          <ElRadioButton value="group">同桌分组</ElRadioButton>
          <ElRadioButton value="disable">停用位置</ElRadioButton>
        </ElRadioGroup>
        <ElInput
          v-if="tool === 'group'"
          v-model="group"
          size="small"
          maxlength="32"
          placeholder="分组名（留空可清除）"
          style="width: 180px"
          :disabled="saving || disabled"
        />
        <div class="brush-actions">
          <ElButton v-if="!emptyLayout && stats.unset > 0" size="small" :disabled="saving || disabled" @click="fillEmpty">
            填充未设为座位 ({{ stats.unset }})
          </ElButton>
          <ElButton v-if="changes.size" size="small" plain :disabled="saving || disabled" @click="resetChanges">
            撤销本次改动
          </ElButton>
        </div>
      </div>

      <div v-if="classroom.podium === 'front'" class="podium">讲 台</div>
      <div class="layout-scroll">
        <div class="layout-body">
          <div v-if="classroom.podium === 'left'" class="podium podium-side">讲台</div>
          <div
            class="editor-grid"
            :style="{ gridTemplateColumns: `58px repeat(${columns.length}, minmax(82px, 1fr))` }"
          >
            <div class="grid-corner">排＼列</div>
            <ElTooltip
              v-for="column in columns"
              :key="'col-' + column"
              :content="tool === 'cycle' ? `整列切换座位／过道（第 ${column} 列）` : `用当前画笔整列设置第 ${column} 列`"
              placement="top"
              :show-after="350"
            >
              <button
                type="button"
                class="axis-btn"
                :disabled="saving || disabled"
                @click="paintColumn(column)"
              >
                第 {{ column }} 列
              </button>
            </ElTooltip>

            <template v-for="row in rows" :key="'row-' + row">
              <ElTooltip
                :content="tool === 'cycle' ? `整排切换座位／过道（第 ${row} 排）` : `用当前画笔整排设置第 ${row} 排`"
                placement="left"
                :show-after="350"
              >
                <button
                  type="button"
                  class="axis-btn row-axis"
                  :disabled="saving || disabled"
                  @click="paintRow(row)"
                >
                  第 {{ row }} 排
                </button>
              </ElTooltip>
              <button
                v-for="column in columns"
                :key="`${row}:${column}`"
                type="button"
                class="layout-cell"
                :class="[cellClass(row, column), { modified: changes.has(key(row, column)) }]"
                :aria-label="`第 ${row} 行第 ${column} 列：${cellText(row, column)}`"
                :disabled="saving || disabled"
                @click="paint(row, column)"
              >
                <span class="cell-coord">{{ row }}-{{ column }}</span>
                <strong class="cell-label">{{ cellText(row, column) }}</strong>
              </button>
            </template>
          </div>
          <div v-if="classroom.podium === 'right'" class="podium podium-side">讲台</div>
        </div>
      </div>
      <div v-if="classroom.podium === 'back'" class="podium">讲 台</div>

      <div v-if="!inline" class="layout-footer">
        <ElButton :disabled="saving" @click="close">取消</ElButton>
        <ElButton type="primary" :loading="saving" :disabled="!changes.size" @click="save">
          保存布局（{{ changes.size }} 处修改）
        </ElButton>
      </div>
    </template>
  </section>
</template>

<style scoped>
.layout-editor-panel {
  display: grid;
  gap: 12px;
}

.layout-editor-panel.inline-mode {
  margin-top: 8px;
  padding-top: 16px;
  border-top: 1px solid #e7efec;
}

.layout-top {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: flex-start;
  gap: 10px;
}

.layout-top h3 {
  margin: 0 0 4px;
  font-size: 15px;
  color: #1f3c35;
}

.layout-stats {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.brush-bar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  background: #f7faf9;
  border: 1px solid #e7efec;
  border-radius: 10px;
}

.brush-actions {
  margin-left: auto;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.editor-grid {
  display: grid;
  gap: 6px;
  min-width: max-content;
  flex: 1;
}

.grid-corner {
  display: grid;
  place-items: center;
  font-size: 11px;
  color: #7c9189;
  background: #f5f8f7;
  border-radius: 6px;
}

.axis-btn {
  padding: 6px 4px;
  border: 1px dashed #c7dad2;
  border-radius: 6px;
  background: #f7fbf9;
  color: #366456;
  font: inherit;
  font-size: 11px;
  font-weight: 600;
  cursor: pointer;
  transition: background-color 0.14s ease, border-color 0.14s ease;
}

.axis-btn:hover:not(:disabled) {
  background: #e2f2ea;
  border-color: #398771;
  border-style: solid;
}

.axis-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.row-axis {
  min-height: 54px;
}

.layout-cell {
  display: grid;
  align-content: center;
  justify-items: center;
  gap: 3px;
  min-height: 54px;
  padding: 6px 4px;
  border: 1px solid #cbe2d7;
  border-radius: 8px;
  background: #ecf7f1;
  color: #245748;
  font: inherit;
  font-size: 12px;
  cursor: pointer;
  transition: transform 0.1s ease, border-color 0.14s ease, box-shadow 0.14s ease;
}

.layout-cell:hover:not(:disabled) {
  border-color: #398771;
  box-shadow: 0 2px 8px rgba(57, 135, 113, 0.14);
}

.layout-cell:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.layout-cell.aisle {
  background: #f1f4f6;
  border-color: #d7dfe4;
  border-style: dashed;
  color: #6b7d85;
}

.layout-cell.unavailable {
  background: #fdf2f2;
  border-color: #f0cccc;
  color: #b44c4c;
}

.layout-cell.disabled-cell {
  background: #f6f6f7;
  border-color: #dcdfe2;
  color: #8c969b;
  text-decoration: line-through;
}

.layout-cell.empty-cell {
  background: #ffffff;
  border-color: #d6e1dc;
  border-style: dashed;
  color: #899d95;
}

.layout-cell.modified {
  box-shadow: inset 0 0 0 1.5px #d9822b;
}

.cell-coord {
  font-size: 10px;
  color: #7c9189;
}

.cell-label {
  font-size: 12px;
  font-weight: 600;
  line-height: 1.2;
}

.layout-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding-top: 12px;
  border-top: 1px solid #ebf1ef;
}
</style>
