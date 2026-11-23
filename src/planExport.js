const font = '"Microsoft YaHei", "PingFang SC", "Noto Sans CJK SC", sans-serif'
const cellWidth = 144
const cellHeight = 96

function lines(context, text, width, limit) {
  const result = ['']
  for (const char of String(text)) {
    const last = result.length - 1
    if (!result[last] || (char !== '\n' && context.measureText(result[last] + char).width <= width)) {
      if (char !== '\n') result[last] += char
    } else {
      if (result.length === limit) {
        while (result[last] && context.measureText(result[last] + '…').width > width) result[last] = result[last].slice(0, -1)
        result[last] += '…'
        break
      }
      result.push(char === '\n' ? '' : char)
    }
  }
  return result
}

function text(context, value, x, y, width, size, limit = 1, color = '#204136') {
  context.font = size + 'px ' + font
  context.fillStyle = color
  context.textBaseline = 'top'
  lines(context, value, width, limit).forEach((line, index) => context.fillText(line, x, y + index * (size + 2)))
}

function canvasFor(width, height) {
  if (width * height > 40_000_000 || width > 16384 || height > 16384) {
    throw new Error('座位表尺寸超出图像导出范围，请使用 PDF 或 Excel')
  }
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const context = canvas.getContext('2d')
  if (!context) throw new Error('无法创建图像')
  context.fillStyle = '#ffffff'
  context.fillRect(0, 0, width, height)
  return { canvas, context }
}

function drawGrid(sheet, cells, firstRow, firstColumn, rowCount, columnCount, ratings = []) {
  const podiumSide = sheet.podium === 'left' || sheet.podium === 'right'
  const sideWidth = podiumSide ? 60 : 0
  const width = Math.max(columnCount * cellWidth + 32 + sideWidth, 330)
  const height = rowCount * cellHeight + 116 + (ratings.length ? 42 + 26 * ratings.length : 0)
  const { canvas, context } = canvasFor(width, height)
  text(context, sheet.class_name, 16, 15, width - 32, 24)
  text(context, '第 ' + firstRow + '–' + (firstRow + rowCount - 1) + ' 排 · 第 ' + firstColumn + '–' + (firstColumn + columnCount - 1) + ' 座',
    16, 48, width - 32, 14)
  const gridTop = sheet.podium_front ? 97 : 64
  const startX = Math.floor((width - columnCount * cellWidth - sideWidth) / 2) + (sheet.podium === 'left' ? sideWidth : 0)
  context.fillStyle = '#e0ede6'
  if (podiumSide) {
    const x = sheet.podium === 'left' ? startX - sideWidth : startX + columnCount * cellWidth
    context.fillRect(x, gridTop, sideWidth - 8, rowCount * cellHeight - 4)
    text(context, sheet.podium === 'left' ? '左侧讲台' : '右侧讲台', x + 8, gridTop + 12, sideWidth - 18, 15, 2)
  } else {
    const podium = sheet.podium_front ? 70 : gridTop + rowCount * cellHeight + 8
    context.fillRect(16, podium, width - 32, 22)
    text(context, '讲台 · ' + (sheet.podium_front ? '第 1 排一侧' : '第 ' + sheet.rows + ' 排一侧'), 24, podium + 2, width - 48, 15)
  }
  for (let dy = 0; dy < rowCount; dy++) {
    const row = firstRow + dy
    for (let dx = 0; dx < columnCount; dx++) {
      const column = firstColumn + dx
      const item = cells.get(row + ':' + column)
      if (!item) continue
      const x = startX + dx * cellWidth
      const y = gridTop + dy * cellHeight
      const width = cellWidth - 18
      context.fillStyle = item.kind === 'seat' ? '#edf6f0' : '#eef1f4'
      context.fillRect(x, y, cellWidth - 4, cellHeight - 4)
      if (item.kind !== 'seat') {
        text(context, item.kind === 'aisle' ? '过道' : '不可用', x + 8, y + 32, width, 18)
        continue
      }
      text(context, row + '排 ' + column + '座', x + 8, y + 5, width, 13, 1, '#435951')
      text(context, item.name ?? '空座', x + 8, y + 23, width, 16, 3)
      if (item.number != null) text(context, item.number + (item.locked ? ' · 锁定' : ''), x + 8, y + 79, width, 12)
    }
  }
  if (ratings.length) {
    let y = rowCount * cellHeight + 112
    text(context, '规则评分', 16, y, width - 32, 20)
    for (const rating of ratings) {
      y += 26
      text(context, rating.label + '  ' + rating.score + ' / ' + rating.maximum, 16, y, width - 32, 16)
    }
  }
  return canvas
}

function encodePng(canvas) {
  return new Promise((resolve, reject) => canvas.toBlob(blob => {
    if (blob) resolve(blob.arrayBuffer())
    else reject(new Error('无法生成 PNG'))
  }, 'image/png'))
}

export async function renderPlan(sheet, format) {
  await document.fonts.ready
  const cells = new Map(sheet.cells.map(cell => [cell.row + ':' + cell.column, cell]))
  const ratings = sheet.ratings ?? []
  if (format === 'png') return encodePng(drawGrid(sheet, cells, 1, 1, sheet.rows, sheet.columns, ratings))

  const { jsPDF } = await import('jspdf')
  const pdf = new jsPDF({ orientation: 'landscape', unit: 'mm', format: 'a4', compress: true })
  let first = true
  const addPage = canvas => {
    if (!first) pdf.addPage()
    first = false
    const scale = Math.min(277 / canvas.width, 190 / canvas.height, 0.25)
    pdf.addImage(canvas.toDataURL('image/jpeg', 0.95), 'JPEG', 10, 10, canvas.width * scale, canvas.height * scale)
  }
  for (let row = 1; row <= sheet.rows; row += 7) {
    for (let column = 1; column <= sheet.columns; column += 8) {
      addPage(drawGrid(sheet, cells, row, column, Math.min(7, sheet.rows - row + 1), Math.min(8, sheet.columns - column + 1)))
    }
  }
  for (let index = 0; index < ratings.length; index += 23) {
    const { canvas, context } = canvasFor(1140, 730)
    text(context, sheet.class_name, 20, 20, 1100, 24)
    text(context, '规则评分', 20, 60, 1100, 22)
    ratings.slice(index, index + 23).forEach((rating, line) =>
      text(context, rating.label + '  ' + rating.score + ' / ' + rating.maximum, 20, 96 + 26 * line, 1100, 16))
    addPage(canvas)
  }
  return pdf.output('arraybuffer')
}
