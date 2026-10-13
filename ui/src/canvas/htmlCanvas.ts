// HTML 畫布的檢查與錯誤（BDD evidence-canvas 場景 2、9；AC-EC-02、09；CONTRACT §12「HTML 畫布的檢查與錯誤」）。
// QA 第 6 輪起 runtime 在畫布 HTML 之前執行（畫布 HTML 是沙盒文件的最後一段）：一開始先跑同一套靜態檢查；畫布 <script> 的
// 錯誤由 runtime 的監聽當下回報；文件 load 時再讀一次加固腳本收集的錯誤（後備），都沒有才等版面穩定回報 ready。
// 沙盒文件時間到 HTML_LOAD_DEADLINE_MS 還沒 load（主執行緒沒卡住，計時器才跑得到）→ canvas_incomplete。
import { lintCanvas } from './lint'
import type { CanvasIssue } from './messages'

export interface EarlyError {
  message: string
  /** 錯誤行號（1 起算；0＝不知道）：data: 腳本是腳本內的行號，其他是沙盒文件的行號。 */
  line: number
  /** 錯誤來源（event.filename）：data: 腳本帶 `kunsu-line=N`（內容第一行在 canvas.html 的行號）。 */
  file?: string
}

const HTML_FILE = 'canvas.html'
const DATA_SCRIPT_LINE = /^data:[^,]*;kunsu-line=(\d+)[;,]/

/** 錯誤位置 → canvas.html 的行號；換算不出來就不帶，不亂指。 */
export function htmlLine(line: number, lineOffset: number, file = ''): number | undefined {
  if (!Number.isFinite(line) || line <= 0) return undefined
  const data = file.match(DATA_SCRIPT_LINE)
  const mapped = data ? Number(data[1]) + Math.floor(line) - 1 : Math.floor(line) - Math.floor(lineOffset)
  return Number.isFinite(mapped) && mapped > 0 ? mapped : undefined
}

export function htmlRuntimeIssue(message: unknown, line: number, lineOffset: number, file = ''): CanvasIssue {
  const at = htmlLine(line, lineOffset, file)
  return {
    file: HTML_FILE,
    ...(at ? { line: at } : {}),
    code: 'canvas_runtime',
    message: String(message || '畫布執行出錯').slice(0, 400),
    hint: '在 canvas.html 的 <script> 修正這個錯誤；常見原因：變數還沒定義、元素 id 打錯、資料是空的',
  }
}

/** HTML 畫布的檢查：靜態檢查有問題就回報它；否則回報加固腳本收集到的錯誤。空陣列＝可以等版面回報 ready。 */
export function htmlCanvasIssues(source: string, early: EarlyError[], lineOffset: number): CanvasIssue[] {
  const lint = lintCanvas('html', source)
  if (lint.length > 0) return lint
  return early.map((error) => htmlRuntimeIssue(error.message, error.line, lineOffset, error.file ?? ''))
}

/**
 * 沙盒文件時間（`performance.now()`，從畫布文件開始載入算起）到這裡還沒 load、也還沒回報：回 `canvas_incomplete`。
 * 比面板的 2 秒逾時早，錯誤來得及送到；load 晚於這個時間的畫布本來就趕不上 2 秒（之後還要等版面穩定）。
 */
export const HTML_LOAD_DEADLINE_MS = 1700

/** 畫布 HTML 讓文件沒在時限內載入完成（主執行緒沒卡住）：能讓 agent 修的原因，不是「無窮迴圈」。 */
export function htmlIncompleteIssue(readyState: string): CanvasIssue {
  return {
    file: HTML_FILE,
    code: 'canvas_incomplete',
    message: `畫布 HTML 沒有載入完成（文件狀態 ${String(readyState).slice(0, 20)}）`,
    hint: '檢查 canvas.html：不要呼叫 window.stop() 或 document.open()（會中斷或換掉文件）；圖片、腳本都寫在畫布裡或放 assets/；<script> 不要做太重的計算',
  }
}

/** runtime 提供的環境（測試用假的：可注入時鐘）。 */
export interface HtmlCanvasEnv {
  /** `document.readyState`。 */
  readyState(): string
  /** 文件 `load` 時呼叫（capture 監聽）。 */
  onLoad(fn: () => void): void
  /** 沙盒文件時間（ms）。 */
  now(): number
  setTimer(fn: () => void, ms: number): void
  /** 加固腳本收集到的錯誤。 */
  earlyErrors(): EarlyError[]
}

export interface HtmlCanvasHooks {
  /** 已經回報過 ready 或 error。 */
  finished(): boolean
  fail(issues: CanvasIssue[]): void
  /** 等版面穩定後回報 ready。 */
  settle(): void
}

/**
 * HTML 畫布的 ready／錯誤判斷（runtime 在畫布內容之前呼叫）：靜態檢查有問題立刻回報；否則等文件 `load`，讀加固腳本收集的
 * 錯誤（後備；畫布 <script> 的錯誤通常已由 runtime 的監聽當下回報），沒有才等版面；時限到還沒 load → `canvas_incomplete`。
 */
export function watchHtmlCanvas(source: string, lineOffset: number, env: HtmlCanvasEnv, hooks: HtmlCanvasHooks): void {
  const lint = lintCanvas('html', source)
  if (lint.length > 0) {
    hooks.fail(lint)
    return
  }
  let loaded = false
  const onLoaded = () => {
    if (loaded) return
    loaded = true
    if (hooks.finished()) return
    const issues = env.earlyErrors().map((error) => htmlRuntimeIssue(error.message, error.line, lineOffset, error.file ?? ''))
    if (issues.length > 0) hooks.fail(issues)
    else hooks.settle()
  }
  env.setTimer(
    () => {
      if (loaded || hooks.finished()) return
      hooks.fail([htmlIncompleteIssue(env.readyState())])
    },
    Math.max(0, HTML_LOAD_DEADLINE_MS - env.now()),
  )
  if (env.readyState() === 'complete') onLoaded()
  else env.onLoad(onLoaded)
}
