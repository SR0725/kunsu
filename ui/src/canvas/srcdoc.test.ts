// 證據畫布沙盒（BDD evidence-canvas AC-EC-06；INV inv_canvas_sandboxed）。
import { describe, expect, it } from 'vitest'
import { CANVAS_CSP, CANVAS_SANDBOX, buildSrcdoc, newToken } from './srcdoc'
import { MAX_CANVAS_HEIGHT, frameMessage, parseCanvasMessage } from './messages'

const doc = (source: string, kind: 'tsx' | 'jsx' | 'html' = 'tsx') =>
  buildSrcdoc({ runtime: 'window.__RUNTIME__ = 1', kind, source, assets: { './assets/a.png': 'data:image/png;base64,AAAA' }, token: 'tok-1' })

describe('inv_canvas_sandboxed', () => {
  it('inv_canvas_sandboxed', () => {
    // 只給執行腳本；沒有同源、表單、彈窗、上層導覽、下載等權限。
    expect(CANVAS_SANDBOX).toBe('allow-scripts')
    for (const rule of ["default-src 'none'", "connect-src 'none'", "form-action 'none'", "frame-src 'none'", "base-uri 'none'", "object-src 'none'", "worker-src 'none'"]) {
      expect(CANVAS_CSP).toContain(rule)
    }
    expect(CANVAS_CSP).not.toMatch(/https?:/)
    const html = doc("export default () => <p>hi</p>")
    // head 最前面：<base href="about:srcdoc">（相對網址與 #id 以畫布自己為基準，不會解析成 Kunsu 的頁面），緊接著 CSP——
    // 任何腳本執行前就生效。
    expect(html).toMatch(/^<!doctype html><html[^>]*><head><base href="about:srcdoc"><meta http-equiv="Content-Security-Policy" content="[^"]+">/i)
    expect(html.match(/<base\b/gi)).toHaveLength(1)
    expect(html).not.toMatch(/<script[^>]+src=/i)
    // CSP 管不到的資源提示：HTML 畫布的 <link> 在組裝時拿掉（引擎設定另外關閉，見殼層 inv_canvas_sandboxed）。
    const hinted = doc('<link rel="preconnect" href="http://x.test/"><link rel=dns-prefetch href=//y.test><p>ok</p>', 'html')
    expect(hinted.slice(hinted.indexOf('<div id="root">'))).not.toMatch(/<link\b/i)
    // 只接受同一次渲染的 resize／ready／error；其餘（包括看起來像作答的）一律忽略。
    for (const bad of [
      { kunsu: 'tok-1', type: 'answer', choice_id: 'a' },
      { kunsu: 'tok-1', type: 'submit' },
      { kunsu: 'tok-1', type: 'navigate', url: 'https://x' },
      { kunsu: 'other', type: 'ready', height: 10 },
      { type: 'ready', height: 10 },
      'ready',
      null,
    ]) {
      expect(parseCanvasMessage(bad, 'tok-1')).toBeNull()
    }
  })

  it('畫布原始碼不能跳出資料區塊', () => {
    const evil = 'export default () => null\n// </script><script>alert(1)</script>'
    const html = doc(evil)
    expect(html.match(/<\/script>/gi)?.length).toBe(3) // 加固腳本、資料區塊、runtime 各一
    expect(html).not.toContain('<script>alert(1)')
  })

  it('HTML 畫布的資產換成 data URI，並注入同一份 CSP', () => {
    const html = doc('<img src="./assets/a.png"><p class="text-sm">hi</p>', 'html')
    expect(html).toContain('src="data:image/png;base64,AAAA"')
    expect(html).toContain('Content-Security-Policy')
  })

  it('token 每次都不同', () => {
    expect(newToken()).not.toBe(newToken())
    expect(newToken()).toMatch(/^[a-z0-9]{16,}$/)
  })
})

describe('parseCanvasMessage', () => {
  it('接受 resize／ready／error 並限制高度', () => {
    expect(parseCanvasMessage({ kunsu: 't', type: 'ready', height: 320.4 }, 't')).toEqual({ type: 'ready', height: 321, overflowX: 0, tall: false })
    expect(parseCanvasMessage({ kunsu: 't', type: 'resize', height: 99999 }, 't')).toEqual({ type: 'resize', height: MAX_CANVAS_HEIGHT })
    expect(parseCanvasMessage({ kunsu: 't', type: 'resize', height: Number.NaN }, 't')).toBeNull()
    const error = parseCanvasMessage(
      { kunsu: 't', type: 'error', issues: [{ file: 'canvas.tsx', line: 3, column: 1, code: 'canvas_runtime', message: 'x is not defined', hint: '檢查變數', extra: '<b>' }] },
      't',
    )
    expect(error).toEqual({ type: 'error', issues: [{ file: 'canvas.tsx', line: 3, column: 1, code: 'canvas_runtime', message: 'x is not defined', hint: '檢查變數' }] })
  })

  it('error 訊息裁掉過長內容與過多條目', () => {
    const issues = Array.from({ length: 50 }, () => ({ file: 'canvas.tsx', code: 'c', message: 'm'.repeat(5000), hint: 'h' }))
    const parsed = parseCanvasMessage({ kunsu: 't', type: 'error', issues }, 't')
    expect(parsed?.type).toBe('error')
    if (parsed?.type !== 'error') throw new Error('unreachable')
    expect(parsed.issues.length).toBeLessThanOrEqual(20)
    expect(parsed.issues[0].message.length).toBeLessThanOrEqual(500)
  })
})

describe('QA 證據畫布第一輪', () => {
  it('HTML 畫布濾掉 meta http-equiv（不能自己換頁）', () => {
    const html = doc('<p>hi</p><META http-equiv="refresh" content="0;url=https://x.test/">', 'html')
    expect(html.toLowerCase()).not.toContain('http-equiv="refresh"')
    expect(html).toContain('<p>hi</p>')
  })
})

// QA 第 2 輪（AC-EC-06）：畫布拿不到檢查／顯示模式；ready 一律帶版面數據，由外層判斷。
describe('畫布看不到模式', () => {
  it('srcdoc 的資料區塊沒有 mode', () => {
    const html = doc('export default () => null')
    const data = html.match(/<script type="application\/json" id="kunsu-canvas">([^<]*)<\/script>/)?.[1] ?? ''
    expect(JSON.parse(data)).not.toHaveProperty('mode')
  })

  it('ready 帶寬度溢出與是否過高；檢查頁據此產生版面問題', async () => {
    const { layoutIssues } = await import('./messages')
    const ready = parseCanvasMessage({ kunsu: 'tok-1', type: 'ready', height: 3000, overflowX: 40 }, 'tok-1')
    expect(ready).toEqual({ type: 'ready', height: MAX_CANVAS_HEIGHT, overflowX: 40, tall: true })
    expect(layoutIssues(ready!, 'canvas.tsx').map((i) => i.code)).toEqual(['canvas_overflow_x', 'canvas_too_tall'])
    const fine = parseCanvasMessage({ kunsu: 'tok-1', type: 'ready', height: 300 }, 'tok-1')
    expect(fine).toEqual({ type: 'ready', height: 300, overflowX: 0, tall: false })
    expect(layoutIssues(fine!, 'canvas.tsx')).toEqual([])
  })

  it('屬性值含 > 的 meta http-equiv 也會被濾掉', () => {
    const html = buildSrcdoc({ runtime: '', kind: 'html', source: '<p>hi</p><meta content="0;url=https://x.test/?a>b" http-equiv="refresh">', assets: {}, token: 't' })
    expect(html.toLowerCase()).not.toContain('http-equiv="refresh"')
  })
})

// AC-EC-06：沙盒 runtime 在執行畫布前移除 WebRTC（CSP 管不到）。
describe('disableNetworkGlobals', () => {
  it('移除 WebRTC 建構子且無法再設回', async () => {
    const { disableNetworkGlobals } = await import('./hardening')
    const win: Record<string, unknown> = { RTCPeerConnection: class {}, webkitRTCPeerConnection: class {}, RTCDataChannel: class {}, fetch: () => 1 }
    disableNetworkGlobals(win)
    expect(win.RTCPeerConnection).toBeUndefined()
    expect(win.webkitRTCPeerConnection).toBeUndefined()
    expect(win.RTCDataChannel).toBeUndefined()
    expect(() => {
      win.RTCPeerConnection = class {}
    }).toThrow()
    expect(win.RTCPeerConnection).toBeUndefined()
  })
})

// AC-EC-09：canvas_failed 每個題目／版本／顯示只記一次。
describe('failureOnce', () => {
  it('同一個 key 只回報一次', async () => {
    const { failureOnce } = await import('./hardening')
    const seen: string[] = []
    const report = failureOnce((reason) => seen.push(reason))
    report('q@1@tok', '圖表載入失敗：A')
    report('q@1@tok', '圖表載入失敗：A')
    report('q@1@tok', '圖表載入失敗：B')
    report('q@1@tok2', '圖表載入失敗：C')
    expect(seen).toEqual(['圖表載入失敗：A', '圖表載入失敗：C'])
  })
})

// QA 第 3 輪（AC-EC-06）：HTML 畫布在 runtime 之前就會執行，加固腳本要放在畫布 HTML 前面；HTML 檢查也看 WebRTC。
describe('HTML 畫布加固', () => {
  it('加固腳本在畫布 HTML 之前', () => {
    const html = buildSrcdoc({ runtime: '', kind: 'html', source: '<p id="canvas-html">hi</p>', assets: {}, token: 't' })
    const guard = html.indexOf('RTCPeerConnection')
    expect(guard).toBeGreaterThan(-1)
    // 資料區塊（帶原始碼）在加固腳本之前；要比的是畫布 HTML 本身（<div id="root"> 之後）。
    expect(guard).toBeLessThan(html.indexOf('canvas-html', html.indexOf('<div id="root">')))
  })

  it('HTML 畫布用 WebRTC 會被檢查出來', async () => {
    const { lintCanvas } = await import('./lint')
    const codes = lintCanvas('html', '<div></div>\n<script>const pc = new RTCPeerConnection()</script>\n').map((i) => `${i.code}@${i.line}`)
    expect(codes).toContain('network_api@2')
  })
})

// QA 第 4 輪（AC-EC-02、06）：HTML 畫布的檢查真的會跑、早期錯誤帶 canvas.html 行號；資源提示在組裝時就拿掉。
function payloadOf(html: string): { kind: string; source: string; line_offset: number } {
  const m = html.match(/<script type="application\/json" id="kunsu-canvas">([^<]*)<\/script>/)
  if (!m) throw new Error('沒有資料區塊')
  return JSON.parse(m[1])
}

/** 畫布內容所在的區段（`<div id="root">` 之後；資料區塊在畫布之前，所以到文件結尾）。 */
function canvasPart(html: string): string {
  const start = html.indexOf('<div id="root">')
  const data = html.indexOf('<script type="application/json"', start)
  return html.slice(start, data < 0 ? html.length : data)
}

describe('QA 第 4 輪：HTML 畫布', () => {
  const source = '<div>圖</div>\n<script>\n  const pc = new RTCPeerConnection()\n</script>\n'

  it('資料區塊帶 HTML 原始碼，runtime 的檢查看得到 network_api（canvas.html 第 3 行）', async () => {
    const { htmlCanvasIssues } = await import('./htmlCanvas')
    const payload = payloadOf(buildSrcdoc({ runtime: '', kind: 'html', source, assets: {}, token: 't' }))
    expect(payload.kind).toBe('html')
    expect(payload.source).toBe(source)
    const issues = htmlCanvasIssues(payload.source, [], payload.line_offset)
    expect(issues.map((i) => `${i.file}:${i.code}@${i.line}`)).toContain('canvas.html:network_api@3')
  })

  it('文件行號 − line_offset ＝ canvas.html 行號（資產換成 data URI、拿掉標籤都不改變行號）', () => {
    const src = '<p>1</p>\n<meta\n http-equiv="refresh" content="0">\n<img src="./assets/a.png">\n<p id="five">5</p>\n<script>boom()</script>'
    const html = buildSrcdoc({ runtime: 'x', kind: 'html', source: src, assets: { './assets/a.png': 'data:image/png;base64,AAAA' }, token: 't' })
    const offset = payloadOf(html).line_offset
    const lines = html.split('\n')
    expect(lines[offset]).toContain('<p>1</p>')
    expect(lines[offset + 3]).toContain('data:image/png;base64,AAAA')
    expect(lines[offset + 4]).toContain('<p id="five">5</p>')
    expect(lines[offset + 5]).toContain('boom()')
  })

  it('runtime 之前的錯誤回 canvas_runtime 與 canvas.html 的行號；有錯誤就不會 ready', async () => {
    const { htmlCanvasIssues } = await import('./htmlCanvas')
    const clean = '<div id="chart">k</div>\n<script>\n  document.getElementById("chart").textContent = data.rows.length\n</script>'
    const payload = payloadOf(buildSrcdoc({ runtime: '', kind: 'html', source: clean, assets: {}, token: 't' }))
    expect(htmlCanvasIssues(payload.source, [], payload.line_offset)).toEqual([])
    const issues = htmlCanvasIssues(
      payload.source,
      [{ message: "ReferenceError: Can't find variable: data", line: payload.line_offset + 3 }],
      payload.line_offset,
    )
    expect(issues).toHaveLength(1)
    expect(issues[0]).toMatchObject({ file: 'canvas.html', line: 3, code: 'canvas_runtime' })
    expect(issues[0].message).toContain('data')
    // 行號換算不出來（0 或落在畫布之前）就不帶行號，不亂指。
    const unknown = htmlCanvasIssues(payload.source, [{ message: 'Script error.', line: 0 }], payload.line_offset)
    expect(unknown[0].line).toBeUndefined()
  })

  it('畫布 HTML 的傳統 inline script 改成 data: 腳本（WebKit 會把 inline 錯誤遮成 Script error.），行號不變；module 與 JSON 不動', () => {
    const src =
      '<div>a</div>\n<script>\n  const x = 1\n  boom(x)\n</script>\n<script type="module">m()</script>\n<script type="application/json">{"a":1}</script>\n<p id="after">after</p>'
    const html = buildSrcdoc({ runtime: '', kind: 'html', source: src, assets: {}, token: 't' })
    expect(CANVAS_CSP).toMatch(/script-src [^;]*\bdata:/)
    const offset = payloadOf(html).line_offset
    const lines = html.split('\n')
    expect(lines[offset + 1]).toMatch(/<script src="data:text\/javascript;kunsu-line=2;charset=utf-8,[^"]*boom/)
    expect(lines[offset + 5]).toContain('<script type="module">m()</script>')
    expect(lines[offset + 6]).toContain('<script type="application/json">{"a":1}</script>')
    expect(lines[offset + 7]).toContain('<p id="after">after</p>')
    const url = html.match(/src="(data:text\/javascript;kunsu-line=2;charset=utf-8,[^"]*)"/)?.[1] ?? ''
    expect(decodeURIComponent(url.slice(url.indexOf(',') + 1))).toBe('\n  const x = 1\n  boom(x)\n')
    // 原始碼（靜態檢查用）不變。
    expect(payloadOf(html).source).toBe(src)
  })

  it('data: 腳本的錯誤行號＝kunsu-line＋錯誤行號－1；文件本身的錯誤＝文件行號－line_offset', async () => {
    const { htmlCanvasIssues } = await import('./htmlCanvas')
    const src = '<div id="chart">k</div>\n<script>\n  document.getElementById("chart").textContent = data.rows.length\n</script>'
    const offset = payloadOf(buildSrcdoc({ runtime: '', kind: 'html', source: src, assets: {}, token: 't' })).line_offset
    const issues = htmlCanvasIssues(
      src,
      [
        { message: "ReferenceError: Can't find variable: data", line: 2, file: 'data:text/javascript;kunsu-line=2;charset=utf-8,%0A%20%20document' },
        { message: 'TypeError: x', line: offset + 4, file: 'about:srcdoc' },
        { message: 'Script error.', line: 0, file: '' },
      ],
      offset,
    )
    expect(issues.map((i) => i.line)).toEqual([3, 4, undefined])
    expect(issues[0].message).toBe("ReferenceError: Can't find variable: data")
  })

  it('HTML 畫布的 <link>（preconnect、dns-prefetch 等資源提示）在組裝時拿掉，換行保留', () => {
    const src = '<link rel="preconnect" href="http://127.0.0.1:18933/">\n<LINK\n rel=dns-prefetch href="//x.test">\n<p>ok</p>'
    const html = buildSrcdoc({ runtime: '', kind: 'html', source: src, assets: {}, token: 't' })
    expect(canvasPart(html)).not.toMatch(/<link\b/i)
    const offset = payloadOf(html).line_offset
    expect(html.split('\n')[offset + 3]).toContain('<p>ok</p>')
  })

  it('加固腳本（WebRTC、資源提示、早期錯誤收集）在任何畫布內容之前，TSX 與 HTML 都有', async () => {
    const { CANVAS_GUARD_SCRIPT } = await import('./hardening')
    for (const kind of ['tsx', 'html'] as const) {
      const html = buildSrcdoc({ runtime: 'RUNTIME', kind, source: kind === 'html' ? '<p id="canvas-html">x</p>' : 'export default () => null', assets: {}, token: 't' })
      const at = html.indexOf(CANVAS_GUARD_SCRIPT)
      expect(at, kind).toBeGreaterThan(-1)
      expect(at, kind).toBeLessThan(html.indexOf('<div id="root">'))
      expect(at, kind).toBeLessThan(html.indexOf('RUNTIME'))
    }
  })
})

// QA 第 6 輪（AC-EC-02；CONTRACT §12「沙盒文件的順序」）：近似的 tokenizer 和瀏覽器切法不同時，放在畫布之後的資料區塊與
// runtime 會被吞掉。結構性修法：Kunsu 的東西全部放在畫布 HTML 之前，畫布 HTML 是文件的最後一段。
describe('QA 第 6 輪：沙盒文件的順序（資料區塊 → 加固腳本 → runtime → 畫布）', () => {
  const DATA = '<script type="application/json" id="kunsu-canvas">'

  it('TSX 與 HTML：資料區塊、加固腳本、runtime 依序在 <div id="root"> 之前', async () => {
    const { CANVAS_GUARD_SCRIPT } = await import('./hardening')
    for (const kind of ['tsx', 'html'] as const) {
      const html = buildSrcdoc({ runtime: 'RUNTIME_CODE', kind, source: kind === 'html' ? '<p>x</p>' : 'export default () => null', assets: {}, token: 't' })
      const data = html.indexOf(DATA)
      const guard = html.indexOf(CANVAS_GUARD_SCRIPT)
      const runtime = html.indexOf('<script>RUNTIME_CODE</script>')
      const root = html.indexOf('<div id="root">')
      expect([data > 0, guard > data, runtime > guard, root > runtime], kind).toEqual([true, true, true, true])
      // 都在 body 裡（head 只有 base、CSP、meta 與基本樣式）。
      expect(data, kind).toBeGreaterThan(html.indexOf('<body>'))
    }
  })

  it('HTML 畫布是文件的最後一段：畫布 HTML 之後沒有任何 Kunsu 的東西', () => {
    const html = buildSrcdoc({ runtime: 'RUNTIME_CODE', kind: 'html', source: '<p>x</p>\n<template></', assets: {}, token: 't' })
    expect(html.endsWith('<div id="root"><p>x</p>\n<template></')).toBe(true)
  })

  it('TSX 畫布的 root 是空的，文件照常結尾', () => {
    const html = buildSrcdoc({ runtime: 'RUNTIME_CODE', kind: 'tsx', source: 'export default () => null', assets: {}, token: 't' })
    expect(html.endsWith('<script>RUNTIME_CODE</script><div id="root"></div></body></html>')).toBe(true)
  })

  it('line_offset 含 runtime 的行數，LF、CR、CRLF 各算一行（和 parser 一樣）', () => {
    const runtime = 'a()\r\nb()\rc()\nd()'
    const html = buildSrcdoc({ runtime, kind: 'html', source: '<p id="one">1</p>\n<p id="two">2</p>', assets: {}, token: 't' })
    const offset = payloadOf(html).line_offset
    const lines = html.split(/\r\n|\r|\n/)
    expect(lines[offset]).toContain('<p id="one">1</p>')
    expect(lines[offset + 1]).toContain('<p id="two">2</p>')
  })

  it('資料區塊的 JSON 沒有任何換行（不影響 line_offset）', () => {
    const html = buildSrcdoc({ runtime: '', kind: 'html', source: 'a\nb\r\nc\rd\u2028e\u2029f', assets: {}, token: 't' })
    const json = html.match(/<script type="application\/json" id="kunsu-canvas">([^<]*)<\/script>/)?.[1] ?? ''
    expect(json).not.toMatch(/[\r\n\u2028\u2029]/)
    expect(payloadOf(html).source).toBe('a\nb\r\nc\rd\u2028e\u2029f')
  })
})

// AC-EC-06b：畫布文件被換掉時，加固腳本在 pagehide 送出 canvas_navigation。WebKit 送到時 event.source 仍是這個 iframe；
// Chromium（開發用瀏覽器）送到時來源文件已不在、source 是 null。只有這一種訊息可以沒有來源，而且仍要同一次渲染的 token。
describe('frameMessage：只收這個 iframe 的訊息', () => {
  const frame = { name: 'canvas-frame' }
  const other = { name: 'other-frame' }
  const nav = { kunsu: 't', type: 'error', issues: [{ file: 'canvas.tsx', code: 'canvas_navigation', message: '畫布換頁了', hint: 'h' }] }
  const ready = { kunsu: 't', type: 'ready', height: 10 }
  const runtimeError = { kunsu: 't', type: 'error', issues: [{ file: 'canvas.tsx', code: 'canvas_runtime', message: 'x', hint: 'h' }] }

  it('同一個 iframe、同一次 token 的 resize／ready／error 照收', () => {
    expect(frameMessage(frame, frame, ready, 't')?.type).toBe('ready')
    expect(frameMessage(frame, frame, nav, 't')?.type).toBe('error')
  })

  it('別的視窗送的一律不收（即使 token 對）', () => {
    expect(frameMessage(other, frame, ready, 't')).toBeNull()
    expect(frameMessage(other, frame, nav, 't')).toBeNull()
    expect(frameMessage(undefined, frame, ready, 't')).toBeNull()
  })

  it('來源已不在（null）只收同一次 token 的 canvas_navigation；ready、其他錯誤、token 不對都不收', () => {
    expect(frameMessage(null, frame, nav, 't')?.type).toBe('error')
    expect(frameMessage(null, frame, ready, 't')).toBeNull()
    expect(frameMessage(null, frame, runtimeError, 't')).toBeNull()
    expect(frameMessage(null, frame, { ...nav, issues: [...nav.issues, runtimeError.issues[0]] }, 't')).toBeNull()
    expect(frameMessage(null, frame, nav, 'other-token')).toBeNull()
    // iframe 已經卸下（frameRef 是 null）時什麼都不收。
    expect(frameMessage(null, null, nav, 't')).toBeNull()
  })
})
