import { ElMessage, ElMessageBox } from 'element-plus'

export function formatError(error) {
  if (!error) return '操作失败'
  return typeof error === 'string' ? error : (error.message ?? String(error))
}

export function notifySuccess(message) {
  ElMessage.success({ message, duration: 2600, grouping: true })
}

export function notifyError(error) {
  ElMessage.error({ message: formatError(error), duration: 4500, showClose: true, grouping: true })
}

export function notifyWarning(message) {
  ElMessage.warning({ message, duration: 3600, showClose: true, grouping: true })
}

export function notifyInfo(message) {
  ElMessage.info({ message, duration: 2800, grouping: true })
}

export async function confirmAction(message, options = {}) {
  const {
    title = '操作确认',
    confirmText = '确定',
    cancelText = '取消',
    type = 'warning',
    danger = false,
  } = options
  try {
    await ElMessageBox.confirm(message, title, {
      confirmButtonText: confirmText,
      cancelButtonText: cancelText,
      type,
      confirmButtonClass: danger ? 'el-button--danger' : '',
      closeOnClickModal: false,
      distinguishCancelAndClose: true,
    })
    return true
  } catch {
    return false
  }
}
