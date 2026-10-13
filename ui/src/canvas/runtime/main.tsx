// 證據畫布沙盒 runtime（PRD §24，Claude Artifact 模式）。在 `sandbox="allow-scripts"` iframe 內執行：
// 讀資料區塊 → 靜態檢查 → sucrase 編譯 → 只允許 react／recharts／lucide-react／kunsu → 渲染 → 量高度回報。
// runtime 在畫布內容之前執行（CONTRACT §12「沙盒文件的順序」）：先記下上層視窗、讀完資料區塊、裝好錯誤監聽。
// HTML 畫布接在 runtime 後面解析：先跑靜態檢查，畫布 <script> 的錯誤當下回報，文件 load 時才判斷 ready（watchHtmlCanvas）。
// TSX 畫布等 root 解析完（DOMContentLoaded）才編譯、掛載。對外只送 resize／ready／error 三種訊息（帶這次渲染的 token）。
import * as React from 'react'
import * as ReactDOMClient from 'react-dom/client'
import * as JsxRuntime from 'react/jsx-runtime'
import * as Recharts from 'recharts'
import * as Lucide from 'lucide-react'
import { transform } from 'sucrase'
import '@tailwindcss/browser'
import geist400 from '@fontsource/geist-sans/files/geist-sans-latin-400-normal.woff2?inline'
import geist500 from '@fontsource/geist-sans/files/geist-sans-latin-500-normal.woff2?inline'
import geist600 from '@fontsource/geist-sans/files/geist-sans-latin-600-normal.woff2?inline'
import * as Kunsu from './kunsu'
import { lintCanvas } from '../lint'
import { earlyErrors, installCanvasGuard } from '../hardening'
import { htmlRuntimeIssue, watchHtmlCanvas } from '../htmlCanvas'
import type { CanvasIssue } from '../messages'
import type { CanvasKind } from '../srcdoc'

// CSP 管不到 WebRTC 與資源提示：畫布程式碼執行前先加固（srcdoc 的加固腳本已裝過就略過，AC-EC-06）。
installCanvasGuard(window)

interface Payload {
  token: string
  kind: CanvasKind
  source: string
  assets: Record<string, string>
  /** HTML 畫布：畫布第一行之前的文件行數（文件行號 − line_offset＝canvas.html 行號）。 */
  line_offset?: number
}

// 資料區塊與上層視窗在畫布內容之前就讀好：畫布之後刪掉資料區塊、改掉 window.parent 都不影響回報。
const payload: Payload = JSON.parse(document.getElementById('kunsu-canvas')?.textContent ?? '{}')
const host = window.parent
const file = `canvas.${payload.kind}`
let finished = false
let errored = false

function post(message: Record<string, unknown>) {
  host.postMessage({ kunsu: payload.token, ...message }, '*')
}

/** 回報錯誤（畫完之後才發生的錯誤也回報一次，面板會隱藏畫布並顯示原因）。 */
function fail(issues: CanvasIssue[]) {
  if (errored) return
  errored = true
  finished = true
  post({ type: 'error', issues })
}

/** 量高度的元素：root（畫布 HTML 把它拿掉時退回 body）。 */
function measured(): HTMLElement | null {
  return document.getElementById('root') ?? document.body
}

function contentHeight(): number {
  const root = measured()
  return Math.ceil(Math.max(root?.scrollHeight ?? 0, root?.getBoundingClientRect().height ?? 0)) + 4
}

// ---- 字型與 Tailwind 主題（色票與決策面板一致） ----
const fontCss = [400, 500, 600]
  .map((w, i) => `@font-face{font-family:"Geist";font-style:normal;font-weight:${w};font-display:block;src:url(${[geist400, geist500, geist600][i]}) format("woff2");}`)
  .join('')
const fontStyle = document.createElement('style')
fontStyle.textContent = fontCss
document.head.appendChild(fontStyle)
const theme = document.createElement('style')
theme.setAttribute('type', 'text/tailwindcss')
theme.textContent = `@theme {
  --font-sans: "Geist", -apple-system, BlinkMacSystemFont, "PingFang TC", "Noto Sans TC", sans-serif;
  --color-k-card: #0b0c0e; --color-k-card-2: #111317; --color-k-line: #1d2026; --color-k-line-2: #2a2e35;
  --color-k-text: #f2f3f5; --color-k-text-2: #c3c7ce; --color-k-muted: #8b919b; --color-k-faint: #5f6570;
  --color-k-accent: #3987e5; --color-k-good: #83c494; --color-k-bad: #ef8d86; --color-k-warn: #e0b35a;
  --color-k-s1: #3987e5; --color-k-s2: #d95926; --color-k-s3: #199e70; --color-k-s4: #c98500;
}`
document.head.appendChild(theme)

// ---- 錯誤位置：畫布程式碼放在沙盒自產的 blob 腳本裡執行，堆疊會帶 blob:…:行:欄（sucrase 保留行號） ----
function stackLine(stack: string | undefined): { line?: number } {
  const m = stack?.match(/blob:[^\s)]*?:(\d+):\d+/)
  return m ? { line: Number(m[1]) } : {}
}

function runtimeIssue(error: unknown, code = 'canvas_runtime'): CanvasIssue {
  const err = error instanceof Error ? error : new Error(String(error))
  // 行號準（sucrase 保留行號）；欄位是編譯後的位置，不回報以免誤導。
  const { line } = stackLine(err.stack)
  return {
    file,
    ...(line ? { line } : {}),
    code,
    message: err.message.slice(0, 400),
    hint: '在畫布裡修正這個錯誤；常見原因：資料欄位名稱打錯、陣列是空的、元件名稱大小寫不對',
  }
}

// 跟 Claude Artifact 一樣：不 import 也能直接用 React.useState 之類。
;(window as unknown as { React: typeof React }).React = React

const MODULES: Record<string, unknown> = {
  react: React,
  'react/jsx-runtime': JsxRuntime,
  recharts: Recharts,
  'lucide-react': Lucide,
  kunsu: Kunsu,
}

function requireModule(name: string): unknown {
  if (name in MODULES) return MODULES[name]
  throw new Error(`不能 import '${name}'：只能用 react、recharts、lucide-react、kunsu`)
}

// ---- 版面量測與回報 ----
function whenSettled(done: () => void) {
  // 等 DOM 安靜（Recharts 動畫、Tailwind 產生樣式）再量；最多等 1.6 秒。檢查與正式顯示同一個標準，
  // 檢查過了，使用者那邊也畫得出來（時限都是 2 秒）。
  const quietMs = 220
  const maxMs = 1600
  const start = performance.now()
  let timer = window.setTimeout(finish, quietMs)
  const observer = new MutationObserver(() => {
    window.clearTimeout(timer)
    if (performance.now() - start < maxMs) timer = window.setTimeout(finish, quietMs)
    else finish()
  })
  // HTML 畫布可能用 <frameset> 換掉 body，或把 body 拿掉：退回整份文件。
  observer.observe(document.body ?? document.documentElement, { subtree: true, childList: true, attributes: true, characterData: true })
  let called = false
  function finish() {
    if (called) return
    called = true
    observer.disconnect()
    void document.fonts.ready.then(done)
  }
}

/** 畫完：一律回報高度與橫向溢出，由外層判斷（檢查頁轉成版面問題；面板只用高度）。畫布看不出自己被檢查。 */
function reportReady() {
  if (finished) return
  const height = contentHeight()
  const overflowX = Math.max(0, document.documentElement.scrollWidth - document.documentElement.clientWidth)
  finished = true
  post({ type: 'ready', height, overflowX })
  const root = measured()
  if (root) new ResizeObserver(() => post({ type: 'resize', height: contentHeight() })).observe(root)
}

const html = payload.kind === 'html'
const lineOffset = Number(payload.line_offset) || 0
// capture：在畫布內容之前裝好，比畫布自己的監聽先跑；只收 target 是 window 的（capture 也看得到圖片等資源載入失敗）。
window.addEventListener(
  'error',
  (event) => {
    if (event.target && event.target !== window) return
    fail([html ? htmlRuntimeIssue(event.message, event.lineno, lineOffset, event.filename) : runtimeIssue(event.error ?? event.message)])
  },
  { capture: true },
)
window.addEventListener(
  'unhandledrejection',
  (event) =>
    fail([html ? htmlRuntimeIssue(event.reason instanceof Error ? event.reason.message : event.reason, 0, lineOffset) : runtimeIssue(event.reason)]),
  { capture: true },
)

class Boundary extends React.Component<{ children: React.ReactNode }, { failed: boolean }> {
  state = { failed: false }
  static getDerivedStateFromError() {
    return { failed: true }
  }
  componentDidCatch(error: unknown) {
    fail([runtimeIssue(error)])
  }
  render() {
    return this.state.failed ? null : this.props.children
  }
}

function Mounted({ children }: { children: React.ReactNode }) {
  React.useEffect(() => whenSettled(reportReady), [])
  return <>{children}</>
}

function runScript() {
  const lint = lintCanvas(payload.kind, payload.source)
  if (lint.length > 0) return fail(lint)
  let code: string
  try {
    code = transform(payload.source, {
      transforms: payload.kind === 'tsx' ? ['typescript', 'jsx', 'imports'] : ['jsx', 'imports'],
      jsxRuntime: 'automatic',
      production: true,
      filePath: file,
    }).code
  } catch (e) {
    const err = e as Error & { loc?: { line: number; column: number } }
    const m = err.message.match(/\((\d+):(\d+)\)/)
    const line = err.loc?.line ?? (m ? Number(m[1]) : undefined)
    const column = err.loc ? err.loc.column + 1 : m ? Number(m[2]) + 1 : undefined
    return fail([
      {
        file,
        ...(line ? { line } : {}),
        ...(column ? { column } : {}),
        code: 'canvas_compile',
        message: err.message
          .replace(/\s*\(\d+:\d+\)\s*$/, '')
          .replace(/^Error transforming [^:]+:\s*/, '')
          .slice(0, 400),
        hint: '語法錯誤：檢查括號、JSX 標籤是否成對、TypeScript 型別寫法',
      },
    ])
  }
  // 草稿資產：原始碼裡的 './assets/x.png' 換成快照 data URI。
  for (const [key, uri] of Object.entries(payload.assets)) {
    const bare = key.replace(/^\.\//, '')
    for (const q of ['"', "'", '`']) {
      code = code.split(`${q}${key}${q}`).join(JSON.stringify(uri)).split(`${q}${bare}${q}`).join(JSON.stringify(uri))
    }
  }
  // 包裝放在第一行同一行，不改變行號。
  const blob = new Blob([`window.__kunsuDefine(function (require, module, exports) {${code}\n})`], { type: 'text/javascript' })
  const url = URL.createObjectURL(blob)
  const w = window as unknown as { __kunsuDefine?: (factory: ModuleFactory) => void }
  w.__kunsuDefine = (factory) => {
    delete w.__kunsuDefine
    const module = { exports: {} as Record<string, unknown> }
    try {
      factory(requireModule, module, module.exports)
    } catch (e) {
      fail([runtimeIssue(e)])
      return
    }
    mount(module.exports)
  }
  const script = document.createElement('script')
  script.src = url
  script.onerror = () => fail([{ file, code: 'canvas_load', message: '畫布程式碼載入失敗', hint: '重新註冊一次；持續失敗請告訴使用者' }])
  document.head.appendChild(script)
}

type ModuleFactory = (require: (name: string) => unknown, module: { exports: Record<string, unknown> }, exports: Record<string, unknown>) => void

function mount(exports: Record<string, unknown>) {
  const Component = (exports.default ?? exports) as React.ComponentType | undefined
  if (typeof Component !== 'function') {
    return fail([
      {
        file,
        code: 'canvas_no_default_export',
        message: '畫布沒有 default export 一個 React 元件',
        hint: '寫成 export default function Canvas() { return (...) }',
      },
    ])
  }
  ReactDOMClient.createRoot(document.getElementById('root')!).render(
    <Boundary>
      <Mounted>
        <Component />
      </Mounted>
    </Boundary>,
  )
}

if (html) {
  // 註冊檢查與正式顯示走同一條路：靜態檢查（network_api、external_url）→ 畫布 HTML 解析（錯誤當下回報）→ 文件 load 時
  // 讀加固腳本收集的錯誤 → 版面穩定才 ready；時限到還沒 load → canvas_incomplete（CONTRACT §12）。
  watchHtmlCanvas(
    payload.source,
    lineOffset,
    {
      readyState: () => document.readyState,
      // capture 也會看到圖片、子框架各自的 load（不冒泡，但經過 window）：只收文件本身的（target 是 document）。
      onLoad: (fn) =>
        window.addEventListener('load', (event) => (event.target === document || event.target === window) && fn(), { capture: true }),
      now: () => performance.now(),
      setTimer: (fn, ms) => void window.setTimeout(fn, ms),
      earlyErrors: () => earlyErrors(window),
    },
    { finished: () => finished, fail, settle: () => whenSettled(reportReady) },
  )
} else if (document.readyState === 'loading') {
  // runtime 在 <div id="root"> 之前執行：root 解析完才編譯、掛載。
  document.addEventListener('DOMContentLoaded', runScript, { once: true })
} else {
  runScript()
}
