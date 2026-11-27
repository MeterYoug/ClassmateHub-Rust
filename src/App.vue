<script setup>
import { elementLocale, languages, locale, localizeMessage, setLocale, t } from './i18n.js'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { ElButton, ElConfigProvider, ElIcon, ElOption, ElSelect, ElTag } from 'element-plus'
import {
  Cpu,
  DataAnalysis,
  FolderOpened,
  Grid,
  House,
  InfoFilled,
  Lock,
  UploadFilled,
} from '@element-plus/icons-vue'
import Management from './Management.vue'
import Scores from './Scores.vue'
import Seating from './Seating.vue'
import DataTools from './DataTools.vue'
import LockScreen from './LockScreen.vue'
import SecurityLock from './SecurityLock.vue'
import { termLabel } from './classOptions.js'
import { confirmAction, notifyError } from './ui.js'

const desktopStatus = ref('正在连接本地服务…')
const serviceConnected = ref(false)
const page = ref('home')
const managementSection = ref('classes')
const openImport = ref(false)
const dashboard = ref([])
const classes = ref([])
const selectedClassId = ref(null)
const activePage = ref(null)
const main = ref(null)
const desktop = isTauri()
const appLock = ref(null)
const workspaceReady = ref(false)
const checkingLock = ref(false)
const startupError = ref('')
const lockPending = ref(false)
const closeBlocked = ref(false)
const activityEvents = ['pointerdown', 'keydown', 'wheel', 'touchstart']
const navigationBlocked = computed(() => activePage.value?.isBusy() ?? false)
let idleTimer
let deferredRefresh
let unlistenClose
let closingPrompt = false

const pages = [
  { id: 'home', label: '工作台', icon: House },
  { id: 'management', label: '基础资料', icon: FolderOpened },
  { id: 'scores', label: '考试与成绩', icon: DataAnalysis },
  { id: 'seating', label: '智能排座', icon: Grid },
  { id: 'backup', label: '备份与恢复', icon: UploadFilled },
  { id: 'ai', label: 'AI 设置', icon: Cpu },
  { id: 'security', label: '应用锁', icon: Lock },
  { id: 'about', label: '关于与更新', icon: InfoFilled },
]

const managementSections = [
  { id: 'classes', label: '班级管理' },
  { id: 'students', label: '学生管理' },
  { id: 'classrooms', label: '教室管理' },
  { id: 'rules', label: '规则管理' },
  { id: 'relations', label: '学生关系管理' },
]

const steps = [
  { index: '01', title: '班级', description: '设置年级、学期和默认科目', target: 'management', section: 'classes' },
  { index: '02', title: '学生', description: '导入 Excel 名单或录入学生档案', target: 'management', section: 'students', count: 'students', unit: '名学生' },
  { index: '03', title: '成绩', description: '录入／导入成绩并同步四档等级', target: 'scores', count: 'exams', unit: '次考试' },
  { index: '04', title: '教室', description: '绘制座位、过道与同桌分组', target: 'management', section: 'classrooms', count: 'layouts', unit: '个可用布局' },
  { index: '05', title: '规则', description: '配置前排、互斥与均衡权重', target: 'management', section: 'rules', count: 'rules', unit: '条规则' },
  { index: '06', title: '方案', description: '智能／AI 排座、拖拽微调与导出', target: 'seating', count: 'plans', unit: '份方案' },
]

const showGlobalClassPicker = computed(() => ['home', 'management', 'scores', 'seating'].includes(page.value))

const currentTopbar = computed(() => {
  if (page.value === 'management') {
    const sub = managementSections.find(item => item.id === managementSection.value)?.label ?? '基础资料'
    return { title: sub, subtitle: '基础资料 · 数据保存在当前电脑' }
  }
  const map = {
    home: { title: '工作台', subtitle: '班级排座与成绩管理概览' },
    scores: { title: '考试与成绩', subtitle: '支持原版 Excel 导入导出与等级同步' },
    seating: { title: '智能排座', subtitle: '本地规则引擎与可选 AI 排座，支持拖拽调座与快照历史' },
    backup: { title: '备份与恢复', subtitle: 'SQLite 一致性整库备份与中断回退保护' },
    ai: { title: 'AI 设置', subtitle: '可选联网排座配置，按次确认脱敏字段' },
    security: { title: '应用锁', subtitle: '' },
    about: { title: '关于与更新', subtitle: '版本信息、开源许可与 GitHub Releases 在线更新检查' },
  }
  return map[page.value] ?? { title: '班伴', subtitle: '' }
})

const dashboardClass = computed(() => dashboard.value.find(item => item.id === selectedClassId.value) ?? null)

const readiness = computed(() => {
  if (!dashboardClass.value) {
    return {
      ready: false,
      tagType: 'info',
      tagLabel: '尚未建班',
      title: '请先创建一个班级',
      description: '创建班级后即可导入学生名单、配置教室布局并开始排座。',
      ctaLabel: '创建第一个班级',
      ctaTarget: () => navigate('management', 'classes'),
    }
  }
  const item = dashboardClass.value
  if (!item.students) {
    return {
      ready: false,
      tagType: 'warning',
      tagLabel: '缺少学生',
      title: `${item.name} · 待导入学生名单`,
      description: '当前班级暂无在用学生，请先导入或录入学生档案。',
      ctaLabel: '去导入学生',
      ctaTarget: () => navigate('management', 'students', true),
    }
  }
  if (!item.layouts || item.max_seats < item.students) {
    const gapHint = item.layouts
      ? `当前教室最大可用座位数为 ${item.max_seats}，少于在用学生数 ${item.students}。`
      : '当前班级尚未配置可用教室布局。'
    return {
      ready: false,
      tagType: 'warning',
      tagLabel: '缺少座位',
      title: `${item.name} · 教室座位待补齐`,
      description: gapHint,
      ctaLabel: '去配置教室布局',
      ctaTarget: () => navigate('management', 'classrooms'),
    }
  }
  return {
    ready: true,
    tagType: 'success',
    tagLabel: '已具备排座条件',
    title: `${item.name} · 已就绪，可开始排座`,
    description: '学生档案与教室可用座位数满足排座要求，可直接生成或调整座位方案。',
    ctaLabel: '开始排座',
    ctaTarget: () => navigate('seating'),
  }
})

function deferRefreshWhileLocked(preferClassId) {
  if (!appLock.value?.locked) return false
  deferredRefresh = preferClassId ?? deferredRefresh ?? null
  return true
}

async function refreshGlobalData(preferClassId = null) {
  if (deferRefreshWhileLocked(preferClassId)) return
  try {
    desktopStatus.value = await invoke('desktop_status')
    serviceConnected.value = true
  } catch {
    if (deferRefreshWhileLocked(preferClassId)) return
    desktopStatus.value = '本地服务未连接'
    serviceConnected.value = false
    return
  }
  if (deferRefreshWhileLocked(preferClassId)) return
  try {
    const [summary, allClasses] = await Promise.all([
      invoke('dashboard_summary'),
      invoke('list_records', { resource: 'classes', classId: null }),
    ])
    if (deferRefreshWhileLocked(preferClassId)) return
    dashboard.value = summary
    classes.value = allClasses

    if (preferClassId && allClasses.some(item => item.id === preferClassId)) {
      selectedClassId.value = preferClassId
    } else if (!allClasses.some(item => item.id === selectedClassId.value)) {
      selectedClassId.value = (allClasses.find(item => item.active) ?? allClasses[0])?.id ?? null
    }
  } catch {
    if (deferRefreshWhileLocked(preferClassId)) return
    dashboard.value = []
    classes.value = []
  }
}

async function confirmLeave(message) {
  if (!activePage.value?.hasUnsavedChanges()) return true
  return confirmAction(message, {
    title: '未保存的修改',
    confirmText: '放弃并继续',
    cancelText: '留在当前页',
    type: 'warning',
    danger: true,
  })
}

async function handleGlobalClassChange(nextId) {
  const normalized = nextId ? Number(nextId) : null
  if (normalized === selectedClassId.value || navigationBlocked.value) return
  if (!(await confirmLeave('切换班级会放弃当前页面未保存的修改，确定继续吗？')) || appLock.value?.locked) return
  selectedClassId.value = normalized
}

async function navigate(target, section = 'classes', importStudents = false) {
  if (target === page.value && (target !== 'management' || section === managementSection.value)) {
    if (importStudents) openImport.value = true
    else if (target === 'seating') await activePage.value?.returnToList?.()
    return
  }
  if (navigationBlocked.value) return
  if (!(await confirmLeave('有未保存的修改，确定放弃并切换页面吗？')) || appLock.value?.locked) return

  if (target === 'management') managementSection.value = section
  openImport.value = importStudents
  page.value = target
  await nextTick()
  if (main.value) main.value.scrollTop = 0
  if (target === 'home') void refreshGlobalData()
}

function enterWorkspace(status) {
  appLock.value = status
  if (!status.locked) closeBlocked.value = false
  if (status.locked || (workspaceReady.value && deferredRefresh === undefined)) return
  workspaceReady.value = true
  const preferClassId = deferredRefresh
  deferredRefresh = undefined
  return refreshGlobalData(preferClassId)
}

async function checkAppLock() {
  if (checkingLock.value) return
  checkingLock.value = true
  let status
  try {
    status = desktop ? await invoke('app_lock_status') : {
      enabled: false, locked: false, idle_timeout_minutes: 0,
    }
  } catch (error) {
    startupError.value = error?.message ?? String(error)
    return
  } finally {
    checkingLock.value = false
  }
  startupError.value = ''
  return enterWorkspace(status)
}

function updateLockSettings(status) {
  if (!appLock.value.locked) appLock.value = status
}

async function lockWorkspace() {
  if (lockPending.value || !appLock.value?.enabled || appLock.value.locked) return
  lockPending.value = true
  clearTimeout(idleTimer)
  try {
    appLock.value = await invoke('lock_app')
  } catch (error) {
    closeBlocked.value = false
    notifyError(error)
    scheduleIdleLock()
  } finally {
    lockPending.value = false
  }
}

function scheduleIdleLock() {
  clearTimeout(idleTimer)
  const status = appLock.value
  if (status?.enabled && !status.locked && status.idle_timeout_minutes > 0) {
    idleTimer = setTimeout(lockWorkspace, status.idle_timeout_minutes * 60_000)
  }
}

function lockShortcut(event) {
  if (!appLock.value?.enabled || appLock.value.locked || event.repeat || event.isComposing) return
  if ((event.ctrlKey || event.metaKey) && event.shiftKey && !event.altKey && event.code === 'KeyL') {
    event.preventDefault()
    void lockWorkspace()
  }
}

watch(appLock, scheduleIdleLock)

onMounted(async () => {
  const startup = checkAppLock()
  if (desktop) {
    for (const event of activityEvents) window.addEventListener(event, scheduleIdleLock, { capture: true, passive: true })
    window.addEventListener('keydown', lockShortcut, { capture: true })
    const appWindow = getCurrentWindow()
    unlistenClose = await appWindow.onCloseRequested(async event => {
      if (!workspaceReady.value || !activePage.value?.hasUnsavedChanges()) return
      event.preventDefault()
      if (appLock.value?.locked || lockPending.value) {
        closeBlocked.value = true
        return
      }
      if (closingPrompt) return
      closingPrompt = true
      try {
        const confirmed = await confirmAction('当前页面有未保存的修改，确定关闭窗口并放弃吗？', {
          title: '关闭应用',
          confirmText: '放弃并关闭',
          cancelText: '继续编辑',
          type: 'warning',
          danger: true,
        })
        if (confirmed && !appLock.value?.locked && !lockPending.value) await appWindow.destroy()
      } finally {
        closingPrompt = false
      }
    })
  }
  await startup
})

onBeforeUnmount(() => {
  clearTimeout(idleTimer)
  if (desktop) {
    for (const event of activityEvents) window.removeEventListener(event, scheduleIdleLock, { capture: true })
    window.removeEventListener('keydown', lockShortcut, { capture: true })
  }
  unlistenClose?.()
})
</script>

<template>
  <ElConfigProvider :locale="elementLocale()">
    <div
      v-if="workspaceReady"
      v-show="!appLock.locked"
      :inert="appLock.locked"
      class="app-shell"
    >
      <aside class="sidebar">
        <div class="brand">
          <span class="brand-icon">班</span>
          <span>班伴<small>CLASSMATE HUB</small></span>
        </div>
        <div class="nav-label">{{ t('工作空间') }}</div>
        <nav class="workspace-nav" :aria-label="t('工作空间')">
          <template v-for="item in pages" :key="item.id">
            <button
              v-if="item.id === 'management'"
              type="button"
              class="nav-item nav-group"
              aria-controls="management-navigation"
              :aria-expanded="page === 'management'"
              :disabled="navigationBlocked"
              @click="navigate('management', managementSection)"
            >
              <span class="nav-group-left">
                <ElIcon class="nav-icon" :size="16"><component :is="item.icon" /></ElIcon>
                <span>{{ t(item.label) }}</span>
              </span>
              <span class="nav-chevron" aria-hidden="true"></span>
            </button>
            <button
              v-else
              type="button"
              class="nav-item"
              :aria-current="page === item.id ? 'page' : undefined"
              :disabled="navigationBlocked"
              @click="navigate(item.id)"
            >
              <ElIcon class="nav-icon" :size="16"><component :is="item.icon" /></ElIcon>
              <span>{{ t(item.label) }}</span>
            </button>
            <div
              v-if="item.id === 'management'"
              v-show="page === 'management'"
              id="management-navigation"
              class="nav-children"
            >
              <button
                v-for="section in managementSections"
                :key="section.id"
                type="button"
                class="nav-item"
                :aria-current="page === 'management' && managementSection === section.id ? 'page' : undefined"
                :disabled="navigationBlocked"
                @click="navigate('management', section.id)"
              >
                {{ t(section.label) }}
              </button>
            </div>
          </template>
        </nav>
      </aside>

      <div class="workspace-column">
        <header class="topbar">
          <div class="topbar-title">
            <h1>{{ t(currentTopbar.title) }}</h1>
            <span v-if="currentTopbar.subtitle" class="topbar-subtitle">{{ t(currentTopbar.subtitle) }}</span>
          </div>
          <div class="topbar-right">
            <ElButton
              v-if="appLock.enabled && !appLock.locked"
              class="lock-workspace-button"
              :icon="Lock"
              :loading="lockPending"
              size="small"
              aria-keyshortcuts="Control+Shift+L Meta+Shift+L"
              @click="lockWorkspace"
            >{{ t('锁定') }}</ElButton>
            <div v-if="showGlobalClassPicker" class="global-class-picker">
              <span>{{ t('当前班级') }}</span>
              <ElSelect
                :model-value="selectedClassId"
                :aria-label="t('当前班级')"
                :disabled="navigationBlocked || !classes.length"
                :placeholder="t('暂无班级')"
                size="default"
                @change="handleGlobalClassChange"
              >
                <ElOption
                  v-for="item in classes"
                  :key="item.id"
                  :label="`${item.name} · ${t(termLabel(item.term))}${item.active ? '' : t('（停用）')}`"
                  :value="item.id"
                />
              </ElSelect>
            </div>
            <span class="status-pill" :class="{ offline: !serviceConnected }" :title="localizeMessage(desktopStatus)" role="status">
              <i aria-hidden="true"></i><span class="status-text">{{ localizeMessage(desktopStatus) }}</span>
            </span>
            <ElSelect
              class="language-picker"
              :model-value="locale"
              :aria-label="t('界面语言')"
              size="small"
              @change="setLocale"
            >
              <ElOption v-for="language in languages" :key="language.id" :label="language.name" :value="language.id" />
            </ElSelect>
          </div>
        </header>

        <main ref="main" class="main">
          <Management
            v-if="page === 'management'"
            ref="activePage"
            :section="managementSection"
            :class-id="selectedClassId"
            :classes="classes"
            :open-import="openImport"
            @classes-changed="refreshGlobalData"
            @open-import-handled="openImport = false"
          />
          <Scores
            v-else-if="page === 'scores'"
            ref="activePage"
            :class-id="selectedClassId"
            :classes="classes"
          />
          <Seating
            v-else-if="page === 'seating'"
            ref="activePage"
            :class-id="selectedClassId"
            :classes="classes"
          />
          <SecurityLock
            v-else-if="page === 'security'"
            ref="activePage"
            :status="appLock"
            :desktop="desktop"
            :locking="lockPending"
            @status-change="updateLockSettings"
            @lock="lockWorkspace"
          />
          <DataTools
            v-else-if="page === 'backup' || page === 'ai' || page === 'about'"
            :key="page"
            ref="activePage"
            :mode="page"
            :dashboard="dashboard"
            @recovery-changed="refreshGlobalData"
          />
          <section v-else class="content">
            <div class="hero">
              <div class="hero-body">
                <div class="hero-top">
                  <ElTag :type="readiness.tagType" effect="dark" round>{{ t(readiness.tagLabel) }}</ElTag>
                </div>
                <h2>{{ localizeMessage(readiness.title) }}</h2>
                <p class="hero-desc">{{ localizeMessage(readiness.description) }}</p>
                <div v-if="dashboardClass" class="hero-metrics">
                  <span class="metric-chip"><strong>{{ dashboardClass.students }}</strong>{{ t('名学生') }}</span>
                  <span class="metric-chip"><strong>{{ dashboardClass.max_seats }}</strong>{{ t('最大座位数') }}</span>
                  <span class="metric-chip"><strong>{{ dashboardClass.layouts }}</strong>{{ t('可用布局') }}</span>
                  <span class="metric-chip"><strong>{{ dashboardClass.exams }}</strong>{{ t('次考试') }}</span>
                  <span class="metric-chip"><strong>{{ dashboardClass.plans }}</strong>{{ t('份方案') }}</span>
                </div>
                <div class="hero-actions">
                  <ElButton type="primary" size="large" @click="readiness.ctaTarget()">
                    {{ t(readiness.ctaLabel) }}
                  </ElButton>
                  <ElButton v-if="dashboardClass" size="large" @click="navigate('management', 'students', true)">
                    {{ t('导入学生') }}
                  </ElButton>
                  <ElButton v-if="dashboardClass" size="large" @click="navigate('scores')">
                    {{ t('管理成绩') }}
                  </ElButton>
                  <ElButton v-if="dashboardClass && !readiness.ready" size="large" @click="navigate('seating')">
                    {{ t('进入排座页') }}
                  </ElButton>
                </div>
              </div>
            </div>

            <div class="section-heading">
              <h2>{{ t('排座准备流程') }}</h2>
              <span>{{ t('按步骤补齐资料，点击卡片直达对应模块') }}</span>
            </div>

            <ol class="steps">
              <li v-for="step in steps" :key="step.title" class="step-card">
                <button type="button" class="dashboard-step" @click="navigate(step.target, step.section)">
                  <div class="step-head">
                    <span class="step-index">
                      <span class="step-badge">{{ step.index }}</span>
                      {{ t(step.title) }}
                    </span>
                    <ElTag
                      v-if="step.count"
                      size="small"
                      :type="dashboardClass?.[step.count] ? 'success' : 'info'"
                      effect="light"
                      round
                    >
                      {{ dashboardClass?.[step.count] ? t('已就绪') : t('待准备') }}
                    </ElTag>
                    <ElTag
                      v-else
                      size="small"
                      :type="classes.length ? 'success' : 'warning'"
                      effect="light"
                      round
                    >
                      {{ classes.length ? t('{0} 个班级', classes.length) : t('待创建') }}
                    </ElTag>
                  </div>
                  <span class="step-desc">{{ t(step.description) }}</span>
                  <div class="step-foot">
                    <span v-if="step.count && dashboardClass">{{ t('当前：') }}{{ dashboardClass[step.count] }} {{ t(step.unit) }}</span>
                    <span v-else-if="!step.count">{{ t('管理学期、年级与科目') }}</span>
                    <span v-else>{{ t('请先创建班级') }}</span>
                    <span>{{ t('前往 →') }}</span>
                  </div>
                </button>
              </li>
            </ol>
          </section>
        </main>
      </div>
    </div>
    <LockScreen
      v-if="appLock?.locked || startupError"
      :status="appLock"
      :startup-error="startupError"
      :checking="checkingLock"
      :close-blocked="closeBlocked"
      @retry="checkAppLock"
      @unlocked="enterWorkspace"
    />
  </ElConfigProvider>
</template>
