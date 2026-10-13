// 畫布靜態檢查（AC-EC-02）：import 白名單與禁用 API，逐行回報。
import { describe, expect, it } from 'vitest'
import { lintCanvas } from './lint'

const codes = (src: string, kind: 'tsx' | 'jsx' | 'html' = 'tsx') => lintCanvas(kind, src).map((i) => `${i.code}@${i.line}:${i.column}`)

describe('lintCanvas', () => {
  it('只允許 react、recharts、lucide-react、kunsu', () => {
    const src = "import { useState } from 'react'\nimport { LineChart } from 'recharts'\nimport { TrendingUp } from 'lucide-react'\nimport { Stat } from 'kunsu'\nimport axios from 'axios'\n"
    expect(codes(src)).toEqual(['import_not_allowed@5:1'])
    const issue = lintCanvas('tsx', src)[0]
    expect(issue.file).toBe('canvas.tsx')
    expect(issue.hint).toContain('react')
  })

  it('禁用網路、儲存、上層視窗與動態 import', () => {
    const src = [
      'export default function C() {',
      "  fetch('/x')",
      '  const s = localStorage.getItem("k")',
      '  window.parent.postMessage("hi", "*")',
      "  import('react')",
      '  new WebSocket("wss://x")',
      '  return null',
      '}',
    ].join('\n')
    expect(codes(src)).toEqual(['network_api@2:3', 'storage_api@3:13', 'parent_access@4:3', 'dynamic_import@5:3', 'network_api@6:7'])
  })

  it('export 行裡的字串不是模組名；只有 import／export … from 才檢查', () => {
    expect(codes('export default () => <p className="text-k-text">hi</p>\n')).toEqual([])
    expect(codes("export const label = 'kunsu'\n")).toEqual([])
    expect(codes("export { x } from 'lodash'\n")).toEqual(['import_not_allowed@1:1'])
    expect(codes("import 'side-effect'\n")).toEqual(['import_not_allowed@1:1'])
  })

  it('註解與字串裡的字不算', () => {
    const src = "// fetch('/x') 只是註解\nconst label = 'fetch( localStorage'\n/* window.parent */\nexport default () => <p>{label}</p>\n"
    expect(codes(src)).toEqual([])
  })

  it('jsx 檔名正確；HTML 畫布擋外部腳本與網址', () => {
    expect(lintCanvas('jsx', "import x from 'lodash'")[0].file).toBe('canvas.jsx')
    expect(codes('<p>ok</p>\n<script src="https://cdn.x/y.js"></script>\n', 'html')).toEqual(['external_url@2:14'])
    expect(codes('<svg xmlns="http://www.w3.org/2000/svg"></svg>', 'html')).toEqual([])
  })
})

// QA 第 2 輪（AC-EC-02、06）
describe('lintCanvas 多行與 WebRTC', () => {
  it('多行 import 也回報 import_not_allowed（指向 import 那一行）', () => {
    expect(codes("import {\n  get,\n} from 'lodash'\nexport default () => null\n")).toContain('import_not_allowed@1:1')
    expect(codes("import axios from\n  'axios'\nexport default () => null\n")).toContain('import_not_allowed@1:1')
    expect(codes("import {\n  useState,\n} from 'react'\nexport default () => null\n")).toEqual([])
  })

  it('export 行不會吃到下一行的 import', () => {
    expect(codes("export const a = 1\nimport x from 'lodash'\n")).toEqual(['import_not_allowed@2:1'])
  })

  it('WebRTC 算網路', () => {
    expect(codes('const pc = new RTCPeerConnection()\n')).toEqual(['network_api@1:16'])
  })
})

// QA 第 4 輪（AC-EC-02、06）：CSP 管不到的資源提示也算連網路（縱深防禦；引擎設定才是保證）。
describe('lintCanvas 資源提示', () => {
  const networkLines = (src: string, kind: 'tsx' | 'html' = 'tsx') =>
    lintCanvas(kind, src)
      .filter((i) => i.code === 'network_api')
      .map((i) => i.line)

  it('TSX：DOM API、setAttribute、relList、JSX 都回報 network_api', () => {
    const src = [
      'export default function C() {',
      '  const l = document.createElement("link")',
      "  l.rel = 'preconnect'",
      "  l.setAttribute('rel', 'dns-prefetch')",
      "  l.relList.add('preload')",
      '  return <link rel="prefetch" href="/x" />',
      '}',
    ].join('\n')
    expect(networkLines(src)).toEqual([3, 4, 5, 6])
    const issue = lintCanvas('tsx', src).find((i) => i.line === 3)!
    expect(issue.message).toContain('preconnect')
    expect(issue.hint).toContain('沙盒沒有網路')
  })

  it('HTML：<link rel=preconnect|dns-prefetch|modulepreload> 回報 network_api', () => {
    const src = '<link rel=preconnect href="//localhost:18933">\n<p>x</p>\n<link rel="dns-prefetch" href="//x.test">\n<link rel="modulepreload" href="/m.js">\n'
    expect(networkLines(src, 'html')).toEqual([1, 3, 4])
  })

  it('不誤判：video preload、變數名、註解', () => {
    expect(codes('<video preload="none"></video>\n', 'html')).toEqual([])
    expect(codes('const preload = 1\nconst related = "preconnect"\nexport default () => <p>{preload}{related}</p>\n')).toEqual([])
    expect(codes("// l.rel = 'preconnect'\nexport default () => null\n")).toEqual([])
  })

  // QA 第 6 輪（AC-EC-06 縱深防禦）：屬性值的字元參照由瀏覽器解碼（`&#112;reconnect` 就是 preconnect），HTML 畫布的檢查
  // 先解碼再比對；欄位指回原文的位置，解碼出的換行不改變行號。
  it('HTML：rel 的字元參照（十進位、十六進位、沒有分號、大寫）先解碼再判斷', () => {
    for (const rel of ['&#112;reconnect', '&#x70;reconnect', '&#X50;RECONNECT', '&#112reconnect', 'pre&#x63;onnect', 'dns&#45;prefetch', '&#0000112;reload']) {
      expect(networkLines(`<p>x</p>\n<link rel="${rel}" href="//127.0.0.1:9/">\n`, 'html'), rel).toEqual([2])
    }
  })

  it('HTML：欄位指回原文（解碼前）的位置；解碼出的換行不改變行號', () => {
    const src = '<p>a&#10;b&NewLine;c</p>\n  <link rel="&#112;reconnect" href="//x.test">\n'
    expect(codes(src, 'html')).toEqual(['network_api@2:9'])
  })

  it('HTML：具名參照拼出的外部網址（http&colon;//、&sol;&sol;）也回報 external_url', () => {
    expect(codes('<img src="http&colon;//x.test/a.png">\n<img src="https:&sol;&sol;y.test/b.png">\n', 'html')).toEqual([
      'external_url@1:11',
      'external_url@2:11',
    ])
  })

  it('HTML：一般的字元參照不誤判', () => {
    expect(codes('<p>&amp; &lt;b&gt; &quot;preconnect&quot; &#169;</p>\n<link rel="stylesheet" href="data:text/css,p{}">\n', 'html')).toEqual([])
  })

  // QA 第 5 輪：提示文字不能誘導 agent 把被擋的寫法改成註解留著（HTML 畫布的檢查逐行看原文，註解裡寫到一樣會被擋，
  // agent 會在「改成註解 → 又被擋」之間打轉）。提示要叫它直接刪掉，而且提示本身不帶可以照抄的 <link 標籤字樣。
  it('資源提示的修正提示：直接刪掉、不要留成註解，提示本身不含 <link 字樣', () => {
    for (const kind of ['tsx', 'html'] as const) {
      const src = kind === 'html' ? '<link rel="preconnect" href="//x">\n' : "export default () => <link rel='preconnect' href='//x' />\n"
      const issue = lintCanvas(kind, src).find((i) => i.code === 'network_api')
      expect(issue?.hint, kind).toMatch(/刪掉/)
      expect(issue?.hint, kind).toMatch(/註解/)
      expect(issue?.hint, kind).not.toMatch(/<link/i)
    }
  })
})
