export function downloadWorkbook(data, filename) {
  const blob = new Blob([new Uint8Array(data)], { type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet' })
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  link.click()
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}

export async function workbookBytes(file) {
  if (!file.name.toLowerCase().endsWith('.xlsx')) throw new Error('请选择 .xlsx 文件')
  if (file.size > 10 * 1024 * 1024) throw new Error('文件超过 10 MB')
  return Array.from(new Uint8Array(await file.arrayBuffer()))
}
