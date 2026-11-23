<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  ElAlert,
  ElButton,
  ElCheckbox,
  ElCollapse,
  ElCollapseItem,
  ElDrawer,
  ElInput,
  ElInputNumber,
  ElOption,
  ElSelect,
  ElSwitch,
  ElTable,
  ElTableColumn,
  ElTag,
} from 'element-plus'
import { CopyDocument, Download, Plus, Refresh, Search, Upload } from '@element-plus/icons-vue'
import { confirmAction, notifyError, notifyInfo, notifySuccess } from './ui.js'

const props = defineProps({
  mode: { type: String, required: true },
  dashboard: { type: Array, default: () => [] },
})
const emit = defineEmits(['data-restored'])

const busy = ref(false)
const summary = ref(null)
const recovery = ref(null)
const localDashboard = ref([])
const profiles = ref([])
const defaultProfileId = ref(null)
const editor = ref(null)
const original = ref('')
const modelOptions = ref([])
const advancedOpen = ref([])
const providerFilter = ref('')
const profileSearch = ref('')
const profileStatus = ref('')
const key = ref('')
const forgetKey = ref(false)
const hasKey = ref(false)
const makeDefault = ref(false)
const profileErrors = ref({})
const about = ref(null)
const updateResult = ref(null)
const customUpdateUrl = ref('')
const updateAdvancedOpen = ref([])
const lastCheckedTime = ref('')

function clearProfileError(field) {
  if (profileErrors.value[field]) {
    const next = { ...profileErrors.value }
    delete next[field]
    profileErrors.value = next
  }
}

function validateProfile() {
  const errors = {}
  const name = (editor.value?.name ?? '').trim()
  const endpoint = (editor.value?.settings?.endpoint ?? '').trim()
  const model = (editor.value?.settings?.model ?? '').trim()
  const timeout = editor.value?.settings?.timeout_seconds

  if (!name || name.length > 100) errors.name = '请填写 1 至 100 个字符的配置名称'
  if (!endpoint) {
    errors.endpoint = '请填写对话接口地址'
  } else if (!/^https?:\/\/.+\/chat\/completions$/i.test(endpoint)) {
    errors.endpoint = '请填写以 /chat/completions 结尾的完整接口地址（外网须使用 HTTPS）'
  }
  if (!model || model.length > 100) errors.model = '请填写或选择 1 至 100 个字符的模型名称'
  if (timeout === null || timeout === '' || !Number.isInteger(Number(timeout)) || Number(timeout) < 1 || Number(timeout) > 120) {
    errors.timeout_seconds = '请填写 1 至 120 之间的整数秒数'
    if (!advancedOpen.value.includes('advanced')) advancedOpen.value = [...advancedOpen.value, 'advanced']
  }

  profileErrors.value = errors
  if (Object.keys(errors).length) {
    notifyError('请先完善标红的必填项或修正输入格式')
    return false
  }
  return true
}

const presets = [
  ['DEEPSEEK', 'DeepSeek', 'https://api.deepseek.com/v1'],
  ['BYTEDANCE', '字节跳动（豆包 / 火山方舟）', 'https://ark.cn-beijing.volces.com/api/v3'],
  ['ZHIPU', '智谱 AI（BigModel / GLM）', 'https://open.bigmodel.cn/api/paas/v4', 0.3, 4095],
  ['KIMI', 'Kimi（月之暗面）', 'https://api.moonshot.cn/v1', 1],
  ['XIAOMI', '小米（MiMo 大模型）', 'https://api.xiaomimimo.com/v1'],
  ['QWEN', '通义千问（阿里云百炼）', 'https://dashscope.aliyuncs.com/compatible-mode/v1'],
  ['SILICONFLOW', '硅基流动（SiliconFlow）', 'https://api.siliconflow.cn/v1'],
  ['OPENAI', 'OpenAI', 'https://api.openai.com/v1'],
  ['OLLAMA', '本地 Ollama', 'http://127.0.0.1:11434/v1'],
  ['CUSTOM', '自定义兼容接口', ''],
]

const filteredProfiles = computed(() =>
  profiles.value.filter(({ profile }) =>
    (!providerFilter.value || profile.provider === providerFilter.value) &&
    (profileStatus.value === '' || String(profile.active) === profileStatus.value) &&
    profile.name.toLowerCase().includes(profileSearch.value.trim().toLowerCase()),
  ),
)

const dirty = computed(
  () =>
    !!editor.value &&
    (JSON.stringify(editor.value) !== original.value ||
      !!key.value ||
      forgetKey.value ||
      makeDefault.value !== (defaultProfileId.value === editor.value.id)),
)

const overview = computed(() => {
  const source = localDashboard.value.length ? localDashboard.value : props.dashboard
  return source.reduce(
    (acc, item) => ({
      classes: acc.classes + 1,
      students: acc.students + (item.students || 0),
      exams: acc.exams + (item.exams || 0),
      plans: acc.plans + (item.plans || 0),
    }),
    { classes: 0, students: 0, exams: 0, plans: 0 },
  )
})

const summaryRows = computed(() => {
  if (!summary.value) return []
  const labels = [
    ['classes', '班级'],
    ['students', '学生'],
    ['exams', '考试'],
    ['plans', '排座方案'],
  ]
  return labels.map(([field, label]) => ({
    label,
    current: summary.value.current[field],
    incoming: summary.value.incoming[field],
  }))
})

defineExpose({
  isBusy: () => busy.value,
  hasUnsavedChanges: () => busy.value || (props.mode === 'ai' && dirty.value),
})

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

async function loadRecovery() {
  const [status, dash] = await Promise.all([
    invoke('recovery_status'),
    invoke('dashboard_summary').catch(() => props.dashboard),
  ])
  recovery.value = status
  localDashboard.value = dash ?? []
  if (recovery.value?.notice) notifyInfo(recovery.value.notice)
}

function applyProfiles(view) {
  profiles.value = view.profiles
  defaultProfileId.value = view.default_id
}

function providerLabel(code) {
  return presets.find(item => item[0] === code)?.[1] ?? code
}

async function openProfile(item = null) {
  if (dirty.value) {
    const ok = await confirmAction('当前模型配置有未保存的修改，确定放弃吗？', {
      title: '放弃修改',
      confirmText: '放弃',
      type: 'warning',
      danger: true,
    })
    if (!ok) return
  }
  editor.value = item
    ? JSON.parse(JSON.stringify(item.profile))
    : {
        id: 0,
        name: '',
        provider: 'CUSTOM',
        active: true,
        remark: '',
        settings: { endpoint: '', model: '', timeout_seconds: 30, temperature: null, max_tokens: null, reasoning_effort: null },
      }
  original.value = JSON.stringify(editor.value)
  key.value = ''
  forgetKey.value = false
  hasKey.value = !!item?.has_key
  makeDefault.value = defaultProfileId.value === editor.value.id
  modelOptions.value = []
  advancedOpen.value = []
  profileErrors.value = {}
}

function applyPreset() {
  const preset = presets.find(item => item[0] === editor.value.provider)
  if (!preset || !preset[2]) return
  const [, name, base, temperature = 0.3, tokens = 8192] = preset
  if (!editor.value.name) editor.value.name = name
  editor.value.settings.endpoint = base + '/chat/completions'
  editor.value.settings.model = ''
  editor.value.settings.temperature = temperature
  editor.value.settings.max_tokens = tokens
  modelOptions.value = []
}

async function cancelEditor() {
  if (dirty.value) {
    const ok = await confirmAction('有未保存的模型配置修改，确定放弃吗？', {
      title: '放弃修改',
      confirmText: '放弃',
      type: 'warning',
      danger: true,
    })
    if (!ok) return
  }
  editor.value = null
  key.value = ''
  profileErrors.value = {}
}

function handleDrawerClose(done) {
  void cancelEditor().then(() => {
    if (!editor.value) done()
  })
}

function currentProfile() {
  const profile = JSON.parse(JSON.stringify(editor.value))
  for (const field of ['temperature', 'max_tokens', 'reasoning_effort']) {
    if (profile.settings[field] === '' || profile.settings[field] === undefined) profile.settings[field] = null
  }
  return profile
}

function backup() {
  return run(async () => {
    if (await invoke('export_backup')) notifySuccess('整库备份已保存')
  })
}

function inspect() {
  summary.value = null
  return run(async () => {
    summary.value = await invoke('inspect_backup')
    if (summary.value) notifyInfo('备份预检通过，请核对下方数据后确认是否覆盖恢复')
  })
}

function discardPreview() {
  const token = summary.value?.token
  summary.value = null
  if (token) void invoke('discard_backup', { token }).catch(() => {})
}
onBeforeUnmount(discardPreview)

async function restore() {
  if (!summary.value) return
  const ok = await confirmAction('恢复将覆盖当前电脑中的全部班级、学生、成绩和排座方案。应用会先保存恢复前快照，确定继续吗？', {
    title: '危险操作：覆盖整库数据',
    confirmText: '确认覆盖恢复',
    cancelText: '取消',
    type: 'error',
    danger: true,
  })
  if (!ok) return
  const token = summary.value.token
  summary.value = null
  await run(async () => {
    const result = await invoke('apply_backup', { token })
    await loadRecovery()
    emit('data-restored')
    notifySuccess(result.message || '恢复成功')
  })
}

function rollback() {
  return run(async () => {
    await invoke('retry_rollback')
    await loadRecovery()
    emit('data-restored')
    notifySuccess('已回退到恢复前的数据')
  })
}

function saveSettings() {
  if (!validateProfile()) return
  return run(async () => {
    const profile = currentProfile()
    applyProfiles(
      await invoke('save_ai_profile', {
        profile,
        key: forgetKey.value ? null : key.value || null,
        forgetKey: forgetKey.value,
        makeDefault: makeDefault.value && profile.active,
      }),
    )
    editor.value = null
    key.value = ''
    notifySuccess('AI 模型配置已保存')
  })
}

function testConnection(item = null) {
  return run(async () => {
    await invoke('test_ai_profile', {
      profile: item?.profile ?? currentProfile(),
      key: !item && !forgetKey.value ? key.value || null : null,
      useSavedKey: !!item || !forgetKey.value,
    })
    notifySuccess('连接测试通过')
  })
}

function fetchModels() {
  return run(async () => {
    modelOptions.value = await invoke('fetch_ai_models', {
      profile: currentProfile(),
      key: forgetKey.value ? null : key.value || null,
      useSavedKey: !forgetKey.value,
    })
    if (modelOptions.value.length) {
      notifySuccess(`已获取 ${modelOptions.value.length} 个可用模型，请在模型下拉框中选择`)
    } else {
      notifyInfo('服务未返回模型列表，可直接在模型输入框中手动填写')
    }
  })
}

async function changeProfile(item, remove) {
  if (remove) {
    const ok = await confirmAction(`确定删除「${item.profile.name}」及该配置保存的凭据？`, {
      title: '删除模型配置',
      confirmText: '确认删除',
      type: 'warning',
      danger: true,
    })
    if (!ok) return
  }
  await run(async () => {
    applyProfiles(await invoke('change_ai_profile', { id: item.profile.id, remove }))
    notifySuccess(remove ? '已删除模型配置' : '已设为默认模型')
  })
}

function resetAiFilters() {
  profileSearch.value = ''
  providerFilter.value = ''
  profileStatus.value = ''
}

function formatByteSize(bytes) {
  if (!bytes || bytes <= 0) return ''
  const mb = bytes / (1024 * 1024)
  return mb >= 1 ? `${mb.toFixed(2)} MB` : `${(bytes / 1024).toFixed(1)} KB`
}

async function copyText(text, label) {
  if (!text) return
  try {
    await navigator.clipboard.writeText(text)
    notifySuccess(`已复制${label}`)
  } catch {
    notifyInfo(`${label}：${text}`)
  }
}

async function loadAbout() {
  about.value = await invoke('about_info')
}

async function checkUpdate() {
  await run(async () => {
    const endpoint = customUpdateUrl.value.trim() || null
    updateResult.value = await invoke('check_app_update', { endpoint })
    const now = new Date()
    const pad = n => String(n).padStart(2, '0')
    lastCheckedTime.value = `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())} ${pad(now.getHours())}:${pad(now.getMinutes())}`
    if (updateResult.value.has_update) {
      notifySuccess(updateResult.value.message)
    } else if (updateResult.value.status === 'up_to_date') {
      notifyInfo(updateResult.value.message)
    } else {
      notifyInfo(updateResult.value.message)
    }
  })
}

async function downloadUpdate() {
  if (!updateResult.value?.download_url) return
  await run(async () => {
    const savedPath = await invoke('download_update_asset', {
      url: updateResult.value.download_url,
      suggestedName: updateResult.value.asset_name ?? null,
    })
    if (savedPath) {
      notifySuccess(`新版安装包已保存到：${savedPath}。请先关闭班伴，再运行安装包覆盖升级。`)
    }
  })
}

onMounted(() =>
  run(async () => {
    if (props.mode === 'backup') await loadRecovery()
    else if (props.mode === 'about') await loadAbout()
    else applyProfiles(await invoke('list_ai_profiles'))
  }),
)
</script>

<template>
  <div class="page-container">
    <template v-if="mode === 'backup'">
      <ElAlert
        v-if="recovery?.pending"
        type="error"
        show-icon
        :closable="false"
        title="检测到上次恢复中断或校验失败，当前业务访问已暂停"
        style="margin-bottom: 16px"
      >
        <template #default>
          <p style="margin: 4px 0 10px">恢复前快照路径：{{ recovery.recovery_point || '未知' }}</p>
          <ElButton type="warning" :loading="busy" @click="rollback">重试回退到恢复前数据</ElButton>
        </template>
      </ElAlert>

      <section class="page-card">
        <div class="section-head-row">
          <div>
            <h2>当前本地数据库概览</h2>
            <p class="form-hint">整库备份不包含 AI 凭据与运行日志，但含学生与成绩隐私信息，请妥善保管备份文件。</p>
          </div>
          <span v-if="recovery?.recovery_point && !recovery?.pending" class="form-hint">
            最近恢复点已保留
          </span>
        </div>
        <div class="overview-grid">
          <div class="stat-tile"><span>班级</span><strong>{{ overview.classes }}</strong></div>
          <div class="stat-tile"><span>在用学生</span><strong>{{ overview.students }}</strong></div>
          <div class="stat-tile"><span>考试批次</span><strong>{{ overview.exams }}</strong></div>
          <div class="stat-tile"><span>排座方案</span><strong>{{ overview.plans }}</strong></div>
        </div>
      </section>

      <div class="backup-split">
        <section class="page-card safe-card">
          <h3>导出整库备份</h3>
          <p class="form-hint">使用 SQLite 在线备份接口生成一致性 `.classmatehub-backup` 文件，不影响当前使用。</p>
          <div class="card-cta">
            <ElButton type="primary" :icon="Download" :loading="busy" :disabled="recovery?.pending" @click="backup">
              导出备份文件
            </ElButton>
          </div>
        </section>

        <section class="page-card danger-card">
          <h3>从备份覆盖恢复</h3>
          <p class="form-hint">
            <strong style="color: #c44545">高风险操作：</strong>确认恢复后会覆盖当前全部业务数据。恢复前会先只读预检并自动留存恢复前快照。
          </p>
          <div class="card-cta">
            <ElButton type="danger" plain :icon="Upload" :loading="busy" :disabled="recovery?.pending" @click="inspect">
              选择备份并预检
            </ElButton>
          </div>
        </section>
      </div>

      <section v-if="summary" class="page-card preview-card">
        <div class="section-head-row">
          <div>
            <h2>备份预检通过</h2>
            <p class="form-hint">{{ summary.source_name }} · 数据库结构版本 v{{ summary.version }}</p>
          </div>
          <ElTag type="danger" effect="light">覆盖前请核对记录数</ElTag>
        </div>
        <ElAlert
          type="warning"
          show-icon
          :closable="false"
          title="执行覆盖恢复将用备份内容完整替换下方当前库数据，操作前应用会自动创建恢复点。"
          style="margin: 12px 0 14px"
        />
        <ElTable :data="summaryRows" border size="small">
          <ElTableColumn prop="label" label="数据类型" width="160" />
          <ElTableColumn prop="current" label="当前库记录数" />
          <ElTableColumn prop="incoming" label="备份文件记录数">
            <template #default="{ row }">
              <strong>{{ row.incoming }}</strong>
            </template>
          </ElTableColumn>
        </ElTable>
        <div class="drawer-footer" style="margin-top: 16px">
          <ElButton :disabled="busy" @click="discardPreview">取消</ElButton>
          <ElButton type="danger" :loading="busy" @click="restore">确认覆盖恢复</ElButton>
        </div>
      </section>
    </template>

    <template v-else-if="mode === 'about'">
      <section class="page-card about-hero">
        <div class="about-brand-row">
          <div class="about-brand-main">
            <span class="about-logo">班</span>
            <div>
              <div class="about-title-line">
                <h2>{{ about?.product_title || '班伴 ClassmateHub' }}</h2>
                <ElTag v-if="about?.version" type="success" effect="dark">v{{ about.version }}</ElTag>
                <ElTag type="info" effect="plain">{{ about?.license || 'MIT' }} 许可证</ElTag>
              </div>
              <p class="form-hint">
                面向一线教师的本地桌面排座与成绩管理应用。核心流程离线运行，数据保存在本机 SQLite 数据库中。
              </p>
            </div>
          </div>
          <div class="action-group">
            <ElButton type="primary" :icon="Refresh" :loading="busy" @click="checkUpdate">
              检查更新
            </ElButton>
            <ElButton
              :icon="CopyDocument"
              :disabled="!about?.releases_url"
              @click="copyText(about?.releases_url, 'GitHub 发布页地址')"
            >
              复制发布页地址
            </ElButton>
          </div>
        </div>

        <div class="about-meta-grid">
          <div class="meta-item">
            <span class="meta-label">运行平台与架构</span>
            <strong>{{ about ? `${about.os} (${about.arch})` : '读取中…' }}</strong>
          </div>
          <div class="meta-item">
            <span class="meta-label">应用标识</span>
            <code>{{ about?.identifier || 'com.classmatehub.desktop' }}</code>
          </div>
          <div class="meta-item">
            <span class="meta-label">数据库结构版本</span>
            <strong>SQLite v{{ about?.sqlite_schema_version ?? 5 }}</strong>
          </div>
          <div class="meta-item">
            <span class="meta-label">离线排座引擎</span>
            <strong>引擎 v{{ about?.engine_version ?? 1 }}</strong>
          </div>
          <div class="meta-item full-span">
            <span class="meta-label">本地数据存储目录（升级与默认卸载均保留此目录数据）</span>
            <div class="path-copy-row">
              <code>{{ about?.data_directory || '读取中…' }}</code>
              <ElButton
                size="small"
                :icon="CopyDocument"
                :disabled="!about?.data_directory"
                @click="copyText(about.data_directory, '本地数据目录路径')"
              >
                复制路径
              </ElButton>
            </div>
          </div>
          <div class="meta-item full-span">
            <span class="meta-label">安装包发布仓库（GitHub）</span>
            <div class="path-copy-row">
              <code>{{ about?.repository_url || '读取中…' }}</code>
              <ElButton
                size="small"
                :icon="CopyDocument"
                :disabled="!about?.repository_url"
                @click="copyText(about.repository_url, '发布仓库地址')"
              >
                复制仓库地址
              </ElButton>
            </div>
          </div>
        </div>
      </section>

      <section class="page-card" style="margin-top: 16px">
        <div class="section-head-row">
          <div>
            <h2>在线更新检查（直连 GitHub Releases）</h2>
            <p class="form-hint">
              仅在主动点击「检查更新」时向 GitHub Releases 发起只读查询（兼容 <code>latest.json</code> 清单与 Releases 接口），无需单独部署更新服务器，也不发送任何学生或班级资料。
            </p>
          </div>
          <span v-if="lastCheckedTime" class="form-hint">最近检查：{{ lastCheckedTime }}</span>
        </div>

        <div v-if="updateResult" class="update-result-card" :class="updateResult.status">
          <div class="update-head">
            <div class="action-group">
              <ElTag
                :type="updateResult.has_update ? 'warning' : updateResult.status === 'up_to_date' ? 'success' : 'info'"
                effect="dark"
              >
                {{
                  updateResult.has_update
                    ? `发现新版本 v${updateResult.latest_version}`
                    : updateResult.status === 'up_to_date'
                      ? '已是最新版本'
                      : '暂未发布公开版本'
                }}
              </ElTag>
              <strong>{{ updateResult.release_title || updateResult.message }}</strong>
              <span v-if="updateResult.published_at" class="form-hint">发布时间：{{ updateResult.published_at }}</span>
            </div>
            <div class="action-group">
              <ElButton
                v-if="updateResult.has_update && updateResult.download_url"
                type="primary"
                size="small"
                :icon="Download"
                :loading="busy"
                @click="downloadUpdate"
              >
                下载新版安装包{{ updateResult.asset_size ? ` (${formatByteSize(updateResult.asset_size)})` : '' }}
              </ElButton>
              <ElButton
                v-if="updateResult.download_url"
                size="small"
                :icon="CopyDocument"
                @click="copyText(updateResult.download_url, '安装包下载链接')"
              >
                复制安装包链接
              </ElButton>
              <ElButton
                size="small"
                :icon="CopyDocument"
                @click="copyText(updateResult.release_page_url, '版本发布页地址')"
              >
                复制发布页链接
              </ElButton>
            </div>
          </div>

          <p class="update-message">{{ updateResult.message }}</p>

          <div v-if="updateResult.notes" class="release-notes-box">
            <span class="form-label">更新说明</span>
            <pre>{{ updateResult.notes }}</pre>
          </div>

          <ElAlert
            v-if="updateResult.has_update"
            type="info"
            show-icon
            :closable="false"
            title="覆盖升级提示：下载新版安装包后，请先关闭正在运行的班伴，再运行新版安装程序；向导会默认选中「覆盖升级（保留本地数据）」。"
            style="margin-top: 10px"
          />
        </div>

        <ElCollapse v-model="updateAdvancedOpen" style="margin-top: 14px">
          <ElCollapseItem title="高级选项：自定义更新清单地址／镜像源（选填）" name="endpoint">
            <div class="form-field">
              <span class="form-label">
                自定义更新地址
                <span class="form-label-note">留空则默认直连 GitHub Releases</span>
              </span>
              <div class="model-row">
                <ElInput
                  v-model="customUpdateUrl"
                  clearable
                  :disabled="busy"
                  :placeholder="about?.default_manifest_url"
                />
                <ElButton :loading="busy" @click="checkUpdate">用此地址检查</ElButton>
              </div>
              <p class="form-hint">
                如校园网直连 GitHub 较慢，可在此填写加速镜像的 HTTPS <code>latest.json</code> 地址。
              </p>
            </div>
          </ElCollapseItem>
        </ElCollapse>
      </section>
    </template>

    <template v-else>
      <div class="action-bar">
        <div class="action-group">
          <ElButton type="primary" :icon="Plus" :disabled="busy" @click="openProfile()">新增模型配置</ElButton>
        </div>
        <span class="form-hint">AI 排座仅在老师逐次确认后发送一次性临时编号、座位布局与必要硬约束。</span>
      </div>

      <div class="filter-bar">
        <ElInput
          v-model="profileSearch"
          clearable
          placeholder="搜索配置名称"
          :prefix-icon="Search"
          style="width: 220px"
        />
        <div class="filter-item">
          <span>供应商</span>
          <ElSelect v-model="providerFilter" clearable placeholder="全部供应商" style="width: 200px">
            <ElOption v-for="[id, label] in presets" :key="id" :label="label" :value="id" />
          </ElSelect>
        </div>
        <div class="filter-item">
          <span>状态</span>
          <ElSelect v-model="profileStatus" clearable placeholder="全部" style="width: 110px">
            <ElOption label="正常" value="true" />
            <ElOption label="停用" value="false" />
          </ElSelect>
        </div>
        <ElButton :icon="Refresh" @click="resetAiFilters">重置</ElButton>
      </div>

      <div class="table-card">
        <ElTable :data="filteredProfiles" empty-text="暂无模型配置，点击上方「新增模型配置」添加">
          <ElTableColumn label="配置名称" min-width="180">
            <template #default="{ row }">
              <div class="name-cell">
                <strong>{{ row.profile.name }}</strong>
                <ElTag v-if="row.profile.id === defaultProfileId" size="small" type="success" effect="dark">默认</ElTag>
              </div>
            </template>
          </ElTableColumn>
          <ElTableColumn label="供应商" min-width="170">
            <template #default="{ row }">{{ providerLabel(row.profile.provider) }}</template>
          </ElTableColumn>
          <ElTableColumn label="模型" min-width="180">
            <template #default="{ row }">
              <code>{{ row.profile.settings.model }}</code>
            </template>
          </ElTableColumn>
          <ElTableColumn label="凭据" width="110">
            <template #default="{ row }">
              <ElTag size="small" :type="row.has_key ? 'success' : 'info'" effect="plain">
                {{ row.has_key ? '已存凭据' : '无密钥' }}
              </ElTag>
            </template>
          </ElTableColumn>
          <ElTableColumn label="状态" width="90">
            <template #default="{ row }">
              <ElTag size="small" :type="row.profile.active ? 'success' : 'info'" effect="light">
                {{ row.profile.active ? '正常' : '停用' }}
              </ElTag>
            </template>
          </ElTableColumn>
          <ElTableColumn label="操作" width="260" fixed="right">
            <template #default="{ row }">
              <ElButton link type="primary" :disabled="busy || !!editor" @click="testConnection(row)">测试连接</ElButton>
              <ElButton
                link
                type="primary"
                :disabled="busy || !!editor || !row.profile.active || row.profile.id === defaultProfileId"
                @click="changeProfile(row, false)"
              >
                设为默认
              </ElButton>
              <ElButton link type="primary" :disabled="busy" @click="openProfile(row)">编辑</ElButton>
              <ElButton link type="danger" :disabled="busy || !!editor" @click="changeProfile(row, true)">删除</ElButton>
            </template>
          </ElTableColumn>
        </ElTable>
      </div>

      <ElDrawer
        :model-value="!!editor"
        :title="editor?.id ? '编辑 AI 模型配置' : '新增 AI 模型配置'"
        size="min(92vw, 560px)"
        :before-close="handleDrawerClose"
        destroy-on-close
      >
        <div v-if="editor" class="drawer-form">
          <p class="required-legend"><span>*</span> 为必填项</p>
          <div class="form-field">
            <span class="form-label">AI 供应商预设</span>
            <ElSelect v-model="editor.provider" :disabled="busy" @change="applyPreset">
              <ElOption v-for="[id, label] in presets" :key="id" :label="label" :value="id" />
            </ElSelect>
          </div>
          <div class="form-field" :class="{ 'is-error': !!profileErrors.name }">
            <span class="form-label is-required">配置名称</span>
            <ElInput
              v-model="editor.name"
              :disabled="busy"
              placeholder="请输入配置名称，例如：深度求索排座主力"
              @input="clearProfileError('name')"
            />
            <p v-if="profileErrors.name" class="form-error">{{ profileErrors.name }}</p>
          </div>
          <div class="form-field" :class="{ 'is-error': !!profileErrors.endpoint }">
            <span class="form-label is-required">对话接口地址</span>
            <ElInput
              v-model="editor.settings.endpoint"
              :disabled="busy"
              placeholder="请输入完整地址，例如：https://服务地址/v1/chat/completions"
              @input="modelOptions = []; clearProfileError('endpoint')"
            />
            <p v-if="profileErrors.endpoint" class="form-error">{{ profileErrors.endpoint }}</p>
          </div>
          <div class="form-field">
            <span class="form-label">接口密钥<span class="form-label-note">选填，视服务商要求</span></span>
            <ElInput
              v-model="key"
              type="password"
              show-password
              autocomplete="off"
              :disabled="busy || forgetKey"
              :placeholder="hasKey ? '已保存系统凭据（留空沿用原密钥）' : '选填，服务要求鉴权时填写'"
            />
            <ElCheckbox v-if="hasKey" v-model="forgetKey" :disabled="busy">移除此配置已保存的密钥</ElCheckbox>
          </div>
          <div class="form-field" :class="{ 'is-error': !!profileErrors.model }">
            <span class="form-label is-required">模型名称</span>
            <div class="model-row">
              <ElSelect
                v-model="editor.settings.model"
                filterable
                allow-create
                default-first-option
                :disabled="busy"
                placeholder="请输入或获取模型名称"
                style="flex: 1"
                @change="clearProfileError('model')"
              >
                <ElOption v-for="model in modelOptions" :key="model" :label="model" :value="model" />
              </ElSelect>
              <ElButton :loading="busy" :disabled="!editor.settings.endpoint" @click="fetchModels">获取模型</ElButton>
            </div>
            <p v-if="profileErrors.model" class="form-error">{{ profileErrors.model }}</p>
            <p v-else-if="modelOptions.length" class="form-hint">已加载 {{ modelOptions.length }} 个可选模型，可直接搜索选择或手动输入。</p>
          </div>
          <div class="switch-row">
            <div class="switch-item">
              <span class="form-label">启用状态</span>
              <ElSwitch v-model="editor.active" :disabled="busy" active-text="正常" inactive-text="停用" />
            </div>
            <ElCheckbox v-model="makeDefault" :disabled="busy || !editor.active">设为排座默认模型</ElCheckbox>
          </div>

          <ElCollapse v-model="advancedOpen">
            <ElCollapseItem title="高级参数（超时、采样温度、最大输出长度、思考程度、备注）" name="advanced">
              <div class="form-grid-2">
                <div class="form-field" :class="{ 'is-error': !!profileErrors.timeout_seconds }">
                  <span class="form-label is-required">超时秒数</span>
                  <ElInputNumber
                    v-model="editor.settings.timeout_seconds"
                    :min="1"
                    :max="120"
                    :disabled="busy"
                    controls-position="right"
                    placeholder="1 至 120 的整数"
                    style="width: 100%"
                    @change="clearProfileError('timeout_seconds')"
                  />
                  <p v-if="profileErrors.timeout_seconds" class="form-error">{{ profileErrors.timeout_seconds }}</p>
                </div>
                <div class="form-field">
                  <span class="form-label">采样温度</span>
                  <ElInputNumber
                    v-model="editor.settings.temperature"
                    :min="0"
                    :max="2"
                    :step="0.1"
                    :precision="1"
                    :disabled="busy"
                    controls-position="right"
                    placeholder="选填，跟随默认"
                    style="width: 100%"
                  />
                </div>
                <div class="form-field">
                  <span class="form-label">最大输出长度</span>
                  <ElInputNumber
                    v-model="editor.settings.max_tokens"
                    :min="1"
                    :max="131072"
                    :disabled="busy"
                    controls-position="right"
                    placeholder="选填，跟随默认"
                    style="width: 100%"
                  />
                </div>
                <div class="form-field">
                  <span class="form-label">思考程度</span>
                  <ElSelect v-model="editor.settings.reasoning_effort" :disabled="busy">
                    <ElOption label="跟随模型默认" :value="null" />
                    <ElOption label="关闭思考" value="off" />
                    <ElOption label="轻量思考" value="low" />
                    <ElOption label="适中思考" value="medium" />
                    <ElOption label="深度思考" value="high" />
                  </ElSelect>
                </div>
                <div class="form-field full-span">
                  <span class="form-label">备注</span>
                  <ElInput v-model="editor.remark" :disabled="busy" placeholder="选填说明" />
                </div>
              </div>
            </ElCollapseItem>
          </ElCollapse>
        </div>

        <template #footer>
          <div class="drawer-footer-split">
            <ElButton
              :loading="busy"
              :disabled="!editor?.settings?.endpoint || !editor?.settings?.model"
              @click="testConnection()"
            >
              测试连接
            </ElButton>
            <div class="drawer-footer">
              <ElButton :disabled="busy" @click="cancelEditor">取消</ElButton>
              <ElButton type="primary" :loading="busy" @click="saveSettings">保存配置</ElButton>
            </div>
          </div>
        </template>
      </ElDrawer>
    </template>
  </div>
</template>

<style scoped>
.section-head-row {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: flex-start;
  gap: 10px;
}

.section-head-row h2 {
  margin: 0 0 4px;
  font-size: 16px;
}

.overview-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 12px;
  margin-top: 14px;
}

.stat-tile {
  padding: 14px 16px;
  background: #f7faf9;
  border: 1px solid #e6efeb;
  border-radius: 10px;
  display: grid;
  gap: 6px;
}

.stat-tile span {
  font-size: 12px;
  color: #637971;
}

.stat-tile strong {
  font-size: 24px;
  color: #1f4c3f;
}

.backup-split {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 16px;
  margin-top: 16px;
}

.safe-card h3,
.danger-card h3 {
  margin: 0 0 6px;
  font-size: 15px;
}

.danger-card {
  border-color: #f3d4d4;
  background: #fffcfc;
}

.card-cta {
  margin-top: 16px;
}

.preview-card {
  margin-top: 16px;
  border-color: #f0d7b8;
}

.name-cell {
  display: flex;
  align-items: center;
  gap: 8px;
}

.model-row {
  display: flex;
  gap: 8px;
}

.switch-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  background: #f7faf9;
  border-radius: 8px;
}

.switch-item {
  display: flex;
  align-items: center;
  gap: 10px;
}

.drawer-footer-split {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
}

.about-brand-row {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 16px;
  padding-bottom: 18px;
  border-bottom: 1px solid #ebf1ef;
}

.about-brand-main {
  display: flex;
  align-items: center;
  gap: 14px;
}

.about-logo {
  display: grid;
  place-items: center;
  width: 48px;
  height: 48px;
  border-radius: 12px;
  background: linear-gradient(135deg, #398771, #2c6d5a);
  color: #ffffff;
  font-size: 22px;
  font-weight: 700;
  flex: none;
}

.about-title-line {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
  margin-bottom: 4px;
}

.about-title-line h2 {
  margin: 0;
  font-size: 18px;
  color: #1f3d36;
}

.about-meta-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 12px;
  margin-top: 16px;
}

.meta-item {
  padding: 12px 14px;
  background: #f8fbfa;
  border: 1px solid #e7efec;
  border-radius: 10px;
  display: grid;
  gap: 6px;
  min-width: 0;
}

.meta-item.full-span {
  grid-column: 1 / -1;
}

.meta-label {
  font-size: 12px;
  color: #647b73;
}

.path-copy-row {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.path-copy-row code {
  font-size: 12px;
  color: #24473e;
  word-break: break-all;
}

.update-result-card {
  margin-top: 14px;
  padding: 14px 16px;
  border-radius: 10px;
  background: #f7faf9;
  border: 1px solid #dce8e3;
  display: grid;
  gap: 10px;
}

.update-result-card.update_available {
  background: #fffaf2;
  border-color: #f0d7b0;
}

.update-head {
  display: flex;
  flex-wrap: wrap;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
}

.update-message {
  margin: 0;
  font-size: 13px;
  color: #304a42;
}

.release-notes-box {
  display: grid;
  gap: 6px;
  padding: 10px 12px;
  background: #ffffff;
  border: 1px solid #e5eeea;
  border-radius: 8px;
}

.release-notes-box pre {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
  font-family: inherit;
  font-size: 13px;
  line-height: 1.55;
  color: #2c443d;
}

@media (max-width: 900px) {
  .overview-grid,
  .about-meta-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .backup-split {
    grid-template-columns: 1fr;
  }
}
</style>
