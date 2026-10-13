// HTML 畫布的 ready／錯誤判斷（BDD evidence-canvas 場景 2、9；AC-EC-02、09；CONTRACT §12「HTML 畫布的檢查與錯誤」）。
// QA 第 6 輪起 runtime 在畫布 HTML 之前執行：一開始先跑靜態檢查，文件 load 時才讀加固腳本收集的錯誤、等版面；沙盒文件
// 時間到 HTML_LOAD_DEADLINE_MS 還沒 load、也還沒回報時回 canvas_incomplete（能讓 agent 修的原因，不是「無窮迴圈」）。
// 時間用可注入的時鐘，不真的等。
import { describe, expect, it } from 'vitest'
import { HTML_LOAD_DEADLINE_MS, htmlIncompleteIssue, watchHtmlCanvas, type HtmlCanvasEnv } from './htmlCanvas'
import { READY_TIMEOUT_MS } from './frameState'
import type { CanvasIssue } from './messages'

interface Early {
  message: string
  line: number
  file?: string
}

function harness({ start = 0, readyState = 'loading', early = [] as Early[] } = {}) {
  let now = start
  let state = readyState
  let errors = early
  const timers: { at: number; fn: () => void }[] = []
  const loads: (() => void)[] = []
  const failed: CanvasIssue[][] = []
  let settled = 0
  const env: HtmlCanvasEnv = {
    readyState: () => state,
    onLoad: (fn) => loads.push(fn),
    now: () => now,
    setTimer: (fn, ms) => timers.push({ at: now + ms, fn }),
    earlyErrors: () => errors,
  }
  const hooks = {
    finished: () => failed.length > 0 || settled > 0,
    fail: (issues: CanvasIssue[]) => failed.push(issues),
    settle: () => settled++,
  }
  return {
    env,
    hooks,
    failed,
    settled: () => settled,
    setEarly: (list: Early[]) => (errors = list),
    setState: (value: string) => (state = value),
    /** 時間往前走：到期的計時器依序執行。 */
    advance(ms: number) {
      now += ms
      for (const t of timers.splice(0).sort((a, b) => a.at - b.at)) {
        if (t.at <= now) t.fn()
        else timers.push(t)
      }
    },
    load() {
      state = 'complete'
      for (const fn of loads.splice(0)) fn()
    },
  }
}

describe('watchHtmlCanvas', () => {
  it('靜態檢查有問題：畫布內容執行前就回報，之後 load 或時限到都不再動作', () => {
    const h = harness()
    watchHtmlCanvas('<p>x</p>\n<link rel="preconnect" href="//x.test">\n', 3, h.env, h.hooks)
    expect(h.failed).toHaveLength(1)
    expect(h.failed[0].map((i) => `${i.code}@${i.line}`)).toEqual(['network_api@2'])
    h.load()
    h.advance(HTML_LOAD_DEADLINE_MS)
    expect(h.failed).toHaveLength(1)
    expect(h.settled()).toBe(0)
  })

  it('沒有問題：load 之前不判斷 ready；load 時等版面一次；之後時限到不動作', () => {
    const h = harness()
    watchHtmlCanvas('<p>ok</p>', 3, h.env, h.hooks)
    h.advance(HTML_LOAD_DEADLINE_MS - 1)
    expect(h.settled()).toBe(0)
    expect(h.failed).toEqual([])
    h.load()
    expect(h.settled()).toBe(1)
    h.advance(10_000)
    expect(h.failed).toEqual([])
    expect(h.settled()).toBe(1)
  })

  it('load 時讀加固腳本收集的錯誤（後備）：有就回 canvas_runtime（canvas.html 行號），不等版面', () => {
    const h = harness()
    watchHtmlCanvas('<div id="c"></div>\n<script>\nboom()\n</script>', 10, h.env, h.hooks)
    h.setEarly([{ message: "ReferenceError: Can't find variable: boom", line: 1, file: 'data:text/javascript;kunsu-line=3;charset=utf-8,boom()' }])
    h.load()
    expect(h.settled()).toBe(0)
    expect(h.failed).toHaveLength(1)
    expect(h.failed[0][0]).toMatchObject({ file: 'canvas.html', line: 3, code: 'canvas_runtime' })
  })

  it('已經回報過（runtime 的錯誤監聽當下送出）：load 時不再等版面', () => {
    const h = harness()
    watchHtmlCanvas('<p>ok</p>', 3, h.env, h.hooks)
    h.hooks.fail([{ file: 'canvas.html', code: 'canvas_runtime', message: 'x', hint: 'h' }])
    h.load()
    h.advance(HTML_LOAD_DEADLINE_MS)
    expect(h.settled()).toBe(0)
    expect(h.failed).toHaveLength(1)
  })

  it('文件已經載入完（readyState complete）：直接等版面', () => {
    const h = harness({ readyState: 'complete' })
    watchHtmlCanvas('<p>ok</p>', 3, h.env, h.hooks)
    expect(h.settled()).toBe(1)
  })

  it('沙盒文件時間到時限還沒 load、也還沒回報：回 canvas_incomplete（帶文件狀態），不是逾時', () => {
    const h = harness({ start: 300 })
    watchHtmlCanvas('<p>ok</p>\n<script>window.stop()</script>', 3, h.env, h.hooks)
    h.setState('interactive')
    h.advance(HTML_LOAD_DEADLINE_MS - 300 - 1)
    expect(h.failed).toEqual([])
    h.advance(1)
    expect(h.failed).toHaveLength(1)
    expect(h.failed[0]).toEqual([htmlIncompleteIssue('interactive')])
    // 之後才 load 也不會再回 ready。
    h.load()
    expect(h.settled()).toBe(0)
  })

  it('runtime 開始得晚、已經超過時限：計時器立刻到期', () => {
    const h = harness({ start: HTML_LOAD_DEADLINE_MS + 50 })
    watchHtmlCanvas('<p>ok</p>', 3, h.env, h.hooks)
    h.advance(0)
    expect(h.failed.map((list) => list[0].code)).toEqual(['canvas_incomplete'])
  })
})

describe('htmlIncompleteIssue', () => {
  it('指出 canvas.html、文件狀態與能修的原因（window.stop、document.open），不提無窮迴圈', () => {
    const issue = htmlIncompleteIssue('loading')
    expect(issue).toMatchObject({ file: 'canvas.html', code: 'canvas_incomplete' })
    expect(issue.line).toBeUndefined()
    expect(issue.message).toContain('沒有載入完成')
    expect(issue.message).toContain('loading')
    expect(issue.hint).toContain('window.stop()')
    expect(issue.hint).toContain('document.open()')
    expect(issue.hint).not.toContain('無窮迴圈')
  })

  it('時限早於面板的 2 秒逾時（留時間把錯誤送到面板）', () => {
    expect(HTML_LOAD_DEADLINE_MS).toBeLessThanOrEqual(READY_TIMEOUT_MS - 250)
    expect(HTML_LOAD_DEADLINE_MS).toBeGreaterThanOrEqual(1500)
  })
})
