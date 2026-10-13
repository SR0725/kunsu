// 證據畫布 iframe → Kunsu 的訊息白名單（AC-EC-06；INV inv_canvas_sandboxed）。
// 只接受同一次渲染 token 的 resize／ready／error；其餘（包括看起來像作答的）一律忽略。

export interface CanvasIssue {
  file: string
  line?: number
  column?: number
  code: string
  message: string
  hint: string
}

export type CanvasMessage =
  | { type: 'resize'; height: number }
  /** ready 一律帶版面數據（橫向溢出 px、是否超過高度上限）；檢查與顯示收到的一樣，由外層判斷（AC-EC-06）。 */
  | { type: 'ready'; height: number; overflowX: number; tall: boolean }
  | { type: 'error'; issues: CanvasIssue[] }

export const MAX_CANVAS_HEIGHT = 2400
const MAX_ISSUES = 20

function text(value: unknown, max: number): string {
  return typeof value === 'string' ? value.slice(0, max) : ''
}

function position(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isInteger(value) && value > 0 ? value : undefined
}

function issueFrom(raw: unknown): CanvasIssue | null {
  if (!raw || typeof raw !== 'object') return null
  const r = raw as Record<string, unknown>
  const issue: CanvasIssue = {
    file: text(r.file, 200) || 'canvas',
    code: text(r.code, 64) || 'canvas_error',
    message: text(r.message, 500),
    hint: text(r.hint, 500),
  }
  const line = position(r.line)
  const column = position(r.column)
  if (line !== undefined) issue.line = line
  if (column !== undefined) issue.column = column
  // 欄位順序固定，方便比對與顯示。
  return {
    file: issue.file,
    ...(issue.line !== undefined ? { line: issue.line } : {}),
    ...(issue.column !== undefined ? { column: issue.column } : {}),
    code: issue.code,
    message: issue.message,
    hint: issue.hint,
  }
}

export function parseCanvasMessage(data: unknown, token: string): CanvasMessage | null {
  if (!data || typeof data !== 'object') return null
  const d = data as Record<string, unknown>
  if (d.kunsu !== token) return null
  switch (d.type) {
    case 'resize':
    case 'ready': {
      const height = Number(d.height)
      if (!Number.isFinite(height) || height < 0) return null
      const clamped = Math.min(MAX_CANVAS_HEIGHT, Math.ceil(height))
      if (d.type === 'resize') return { type: 'resize', height: clamped }
      const overflow = Number(d.overflowX)
      const overflowX = Number.isFinite(overflow) && overflow > 0 ? Math.ceil(overflow) : 0
      return { type: 'ready', height: clamped, overflowX, tall: Math.ceil(height) > MAX_CANVAS_HEIGHT }
    }
    case 'error': {
      const list = Array.isArray(d.issues) ? d.issues.slice(0, MAX_ISSUES) : []
      return { type: 'error', issues: list.map(issueFrom).filter((i): i is CanvasIssue => i !== null) }
    }
    default:
      return null
  }
}

/**
 * 只收這個 iframe、這一次 token 的訊息（INV-20）。唯一例外：畫布文件被換掉時，加固腳本在 pagehide 送出的
 * `canvas_navigation`——WebKit 送到時 `event.source` 仍是這個 iframe，Chromium 已把來源文件丟掉、`source` 是 null。
 * 這一則只能讓這張畫布失敗（畫布本來就能自己回報錯誤），仍要同一次的 token；iframe 已卸下（`frame` 是 null）時什麼都不收。
 */
export function frameMessage(source: unknown, frame: unknown, data: unknown, token: string): CanvasMessage | null {
  if (frame === null || frame === undefined) return null
  const message = parseCanvasMessage(data, token)
  if (!message) return null
  if (source === frame) return message
  const navigated = message.type === 'error' && message.issues.length > 0 && message.issues.every((i) => i.code === 'canvas_navigation')
  return source === null && navigated ? message : null
}

/** 註冊檢查：ready 的版面數據 → 版面問題（橫向溢出、超過高度上限）。正式顯示不用。 */
export function layoutIssues(message: CanvasMessage, file: string): CanvasIssue[] {
  if (message.type !== 'ready') return []
  const issues: CanvasIssue[] = []
  if (message.overflowX > 1) {
    issues.push({
      file,
      code: 'canvas_overflow_x',
      message: `畫布比面板寬 ${message.overflowX}px，會出現橫向捲動`,
      hint: '寬度用 100%／Grid／ChartFrame，不要寫死大於 560px 的寬度',
    })
  }
  if (message.tall) {
    issues.push({
      file,
      code: 'canvas_too_tall',
      message: `畫布超過 ${MAX_CANVAS_HEIGHT}px`,
      hint: '只留決策需要的圖；多張圖改成並排',
    })
  }
  return issues
}
