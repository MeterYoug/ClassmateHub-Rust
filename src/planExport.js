const fontFamily = '"Microsoft YaHei", "PingFang SC", "Noto Sans CJK SC", sans-serif'
const CELL_WIDTH = 136
const CELL_HEIGHT = 82
const ROW_HEADER_WIDTH = 64
const COL_HEADER_HEIGHT = 42
const PODIUM_BAR_HEIGHT = 40
const PODIUM_SIDE_WIDTH = 52
const MARGIN_X = 36
const MARGIN_Y = 28
const TITLE_HEIGHT = 48
const PODIUM_GAP = 14
const MAX_CANVAS_PIXELS = 36_000_000
const MAX_CANVAS_EDGE = 16_384

function wrapLines(context, text, maxWidth, limit) {
  const result = ['']
  for (const char of String(text)) {
    const last = result.length - 1
    if (!result[last] || (char !== '\n' && context.measureText(result[last] + char).width <= maxWidth)) {
      if (char !== '\n') result[last] += char
    } else {
      if (result.length === limit) {
        while (result[last] && context.measureText(result[last] + '…').width > maxWidth) {
          result[last] = result[last].slice(0, -1)
        }
        result[last] += '…'
        break
      }
      result.push(char === '\n' ? '' : char)
    }
  }
  return result
}

function drawCenteredText(context, value, centerX, centerY, maxWidth, size, options = {}) {
  const { weight = 'normal', color = '#111111', limit = 1, lineGap = 4 } = options
  context.font = `${weight} ${size}px ${fontFamily}`
  context.fillStyle = color
  context.textAlign = 'center'
  context.textBaseline = 'middle'
  const lines = wrapLines(context, value, maxWidth, limit)
  const step = size + lineGap
  const startY = centerY - ((lines.length - 1) * step) / 2
  lines.forEach((line, index) => {
    context.fillText(line, centerX, startY + index * step)
  })
}

function createCanvas(logicalWidth, logicalHeight, preferredScale = 2) {
  let scale = preferredScale
  while (
    scale > 1 &&
    (logicalWidth * scale > MAX_CANVAS_EDGE ||
      logicalHeight * scale > MAX_CANVAS_EDGE ||
      logicalWidth * logicalHeight * scale * scale > MAX_CANVAS_PIXELS)
  ) {
    scale = Math.max(1, scale - 0.5)
  }
  const pixelWidth = Math.ceil(logicalWidth * scale)
  const pixelHeight = Math.ceil(logicalHeight * scale)
  if (pixelWidth * pixelHeight > 40_000_000 || pixelWidth > MAX_CANVAS_EDGE || pixelHeight > MAX_CANVAS_EDGE) {
    throw new Error('座位表尺寸超出图像导出范围，请使用 Excel 导出')
  }
  const canvas = document.createElement('canvas')
  canvas.width = pixelWidth
  canvas.height = pixelHeight
  const context = canvas.getContext('2d')
  if (!context) throw new Error('无法创建图像')
  context.fillStyle = '#ffffff'
  context.fillRect(0, 0, pixelWidth, pixelHeight)
  if (typeof context.scale === 'function') {
    context.scale(scale, scale)
  }
  return { canvas, context }
}

function strokeBox(context, x, y, width, height, lineWidth = 1, color = '#333333') {
  context.strokeStyle = color
  context.lineWidth = lineWidth
  context.strokeRect(x, y, width, height)
}

function drawSheet(sheet, cells) {
  const rows = sheet.rows
  const columns = sheet.columns
  const podium = sheet.podium || (sheet.podium_front ? 'front' : 'back')
  const podiumSide = podium === 'left' || podium === 'right'
  const seatCoordinate = (index, size) => (sheet.teacher ? size - index + 1 : index)

  const tableWidth = ROW_HEADER_WIDTH + columns * CELL_WIDTH
  const tableHeight = COL_HEADER_HEIGHT + rows * CELL_HEIGHT
  const sideSpace = podiumSide ? PODIUM_SIDE_WIDTH + PODIUM_GAP : 0
  const topPodiumSpace = podium === 'front' ? PODIUM_BAR_HEIGHT + PODIUM_GAP : 0
  const bottomPodiumSpace = podium === 'back' ? PODIUM_BAR_HEIGHT + PODIUM_GAP : 0

  const logicalWidth = Math.max(tableWidth + sideSpace + MARGIN_X * 2, 520)
  const logicalHeight = MARGIN_Y * 2 + TITLE_HEIGHT + topPodiumSpace + tableHeight + bottomPodiumSpace
  const { canvas, context } = createCanvas(logicalWidth, logicalHeight, 2)

  const contentWidth = tableWidth + sideSpace
  const contentLeft = Math.floor((logicalWidth - contentWidth) / 2)
  const tableLeft = contentLeft + (podium === 'left' ? sideSpace : 0)
  const tableTop = MARGIN_Y + TITLE_HEIGHT + topPodiumSpace
  const seatGridLeft = tableLeft + ROW_HEADER_WIDTH
  const seatGridTop = tableTop + COL_HEADER_HEIGHT
  const seatGridWidth = columns * CELL_WIDTH
  const seatGridHeight = rows * CELL_HEIGHT

  // 顶部标题（纯黑白打印风格，无底色、无视角副标题）
  const titleText = `${sheet.class_name} 座位表`
  drawCenteredText(context, titleText, logicalWidth / 2, MARGIN_Y + 20, logicalWidth - MARGIN_X * 2, 28, {
    weight: 'bold',
    color: '#111111',
  })

  // 讲台（纯白底 + 清晰深色边框，支持四向讲台与 180° 视角翻转）
  if (podiumSide) {
    const podiumX = podium === 'left' ? contentLeft : tableLeft + tableWidth + PODIUM_GAP
    strokeBox(context, podiumX, seatGridTop, PODIUM_SIDE_WIDTH, seatGridHeight, 1.8, '#222222')
    drawCenteredText(context, '讲\n台', podiumX + PODIUM_SIDE_WIDTH / 2, seatGridTop + seatGridHeight / 2, PODIUM_SIDE_WIDTH - 8, 18, {
      weight: 'bold',
      color: '#111111',
      limit: 2,
      lineGap: 14,
    })
  } else {
    const podiumY = podium === 'front' ? tableTop - PODIUM_GAP - PODIUM_BAR_HEIGHT : tableTop + tableHeight + PODIUM_GAP
    strokeBox(context, seatGridLeft, podiumY, seatGridWidth, PODIUM_BAR_HEIGHT, 1.8, '#222222')
    drawCenteredText(
      context,
      '讲台',
      seatGridLeft + seatGridWidth / 2,
      podiumY + PODIUM_BAR_HEIGHT / 2,
      seatGridWidth - 24,
      18,
      { weight: 'bold', color: '#111111' },
    )
  }

  // 左上角表头：排 / 列
  strokeBox(context, tableLeft, tableTop, ROW_HEADER_WIDTH, COL_HEADER_HEIGHT, 1, '#333333')
  drawCenteredText(context, '排 \\ 列', tableLeft + ROW_HEADER_WIDTH / 2, tableTop + COL_HEADER_HEIGHT / 2, ROW_HEADER_WIDTH - 8, 14, {
    weight: 'bold',
    color: '#222222',
  })

  // 顶部列号表头（显示原始列号数字）
  for (let dx = 0; dx < columns; dx++) {
    const column = seatCoordinate(dx + 1, columns)
    const x = seatGridLeft + dx * CELL_WIDTH
    strokeBox(context, x, tableTop, CELL_WIDTH, COL_HEADER_HEIGHT, 1, '#333333')
    drawCenteredText(context, String(column), x + CELL_WIDTH / 2, tableTop + COL_HEADER_HEIGHT / 2, CELL_WIDTH - 12, 16, {
      weight: 'bold',
      color: '#111111',
    })
  }

  // 左侧排号表头 + 座位单元格（纯白底、清晰黑框、姓名居中）
  for (let dy = 0; dy < rows; dy++) {
    const row = seatCoordinate(dy + 1, rows)
    const y = seatGridTop + dy * CELL_HEIGHT

    strokeBox(context, tableLeft, y, ROW_HEADER_WIDTH, CELL_HEIGHT, 1, '#333333')
    drawCenteredText(context, String(row), tableLeft + ROW_HEADER_WIDTH / 2, y + CELL_HEIGHT / 2, ROW_HEADER_WIDTH - 8, 16, {
      weight: 'bold',
      color: '#111111',
    })

    for (let dx = 0; dx < columns; dx++) {
      const column = seatCoordinate(dx + 1, columns)
      const x = seatGridLeft + dx * CELL_WIDTH
      strokeBox(context, x, y, CELL_WIDTH, CELL_HEIGHT, 1, '#333333')

      const item = cells.get(`${row}:${column}`)
      if (!item) continue

      const centerX = x + CELL_WIDTH / 2
      const centerY = y + CELL_HEIGHT / 2
      const innerWidth = CELL_WIDTH - 16

      if (item.kind !== 'seat') {
        drawCenteredText(context, item.kind === 'aisle' ? '过道' : '不可用', centerX, centerY, innerWidth, 16, {
          color: '#666666',
        })
        continue
      }

      if (!item.name) {
        drawCenteredText(context, '空座', centerX, centerY, innerWidth, 15, {
          color: '#777777',
        })
        continue
      }

      if (item.number != null) {
        const subText = item.number + (item.locked ? ' · 锁' : '')
        drawCenteredText(context, item.name, centerX, centerY - 11, innerWidth, 20, {
          weight: 'bold',
          color: '#111111',
          limit: 2,
          lineGap: 2,
        })
        drawCenteredText(context, subText, centerX, centerY + 18, innerWidth, 12, {
          color: '#444444',
        })
      } else {
        drawCenteredText(context, item.name, centerX, centerY, innerWidth, 22, {
          weight: 'bold',
          color: '#111111',
          limit: 2,
          lineGap: 3,
        })
      }
    }
  }

  // 整张表格加粗外框，打印更有层次
  strokeBox(context, tableLeft, tableTop, tableWidth, tableHeight, 2, '#222222')

  return { canvas, logicalWidth, logicalHeight }
}

function encodePng(canvas) {
  return new Promise((resolve, reject) =>
    canvas.toBlob(blob => {
      if (blob) resolve(blob.arrayBuffer())
      else reject(new Error('无法生成 PNG'))
    }, 'image/png'),
  )
}

export async function renderPlan(sheet, format) {
  await document.fonts.ready
  const cells = new Map(sheet.cells.map(cell => [`${cell.row}:${cell.column}`, cell]))
  const { canvas, logicalWidth, logicalHeight } = drawSheet(sheet, cells)

  if (format === 'png') return encodePng(canvas)

  const { jsPDF } = await import('jspdf')
  const pageWidth = 297
  const pageHeight = 210
  const margin = 10
  const pdf = new jsPDF({ orientation: 'landscape', unit: 'mm', format: 'a4', compress: true })

  const maxW = pageWidth - margin * 2
  const maxH = pageHeight - margin * 2
  const scale = Math.min(maxW / logicalWidth, maxH / logicalHeight)
  const renderW = logicalWidth * scale
  const renderH = logicalHeight * scale
  const offsetX = (pageWidth - renderW) / 2
  const offsetY = (pageHeight - renderH) / 2

  pdf.addImage(canvas.toDataURL('image/png'), 'PNG', offsetX, offsetY, renderW, renderH)
  return pdf.output('arraybuffer')
}
