// 畫布靜態檢查（AC-EC-02）：import 白名單與禁用 API，逐行回報。註解與字串內容不算。
import type { CanvasIssue } from './messages'
import type { CanvasKind } from './srcdoc'

export const ALLOWED_IMPORTS = ['react', 'recharts', 'lucide-react', 'kunsu'] as const

/** 把註解與字串內容換成空白（保留換行與位置），之後的比對只看程式碼本體。`strings: false` 只遮註解、保留字串。 */
export function maskCode(src: string, { strings = true }: { strings?: boolean } = {}): string {
  const out = src.split('')
  let i = 0
  const blank = (from: number, to: number) => {
    for (let k = from; k < to; k++) if (out[k] !== '\n') out[k] = ' '
  }
  while (i < src.length) {
    const c = src[i]
    const next = src[i + 1]
    if (c === '/' && next === '/') {
      const end = src.indexOf('\n', i)
      const stop = end === -1 ? src.length : end
      blank(i, stop)
      i = stop
    } else if (c === '/' && next === '*') {
      const end = src.indexOf('*/', i + 2)
      const stop = end === -1 ? src.length : end + 2
      blank(i, stop)
      i = stop
    } else if (c === '"' || c === "'" || c === '`') {
      let k = i + 1
      while (k < src.length && src[k] !== c) {
        if (src[k] === '\\') k++
        else if (c !== '`' && src[k] === '\n') break
        k++
      }
      if (strings) blank(i + 1, k)
      i = k + 1
    } else {
      i++
    }
  }
  return out.join('')
}

interface Rule {
  code: string
  pattern: RegExp
  message: (match: string) => string
  hint: string
}

const RULES: Rule[] = [
  {
    code: 'dynamic_import',
    pattern: /\bimport\s*\(/,
    message: () => '畫布不能動態 import',
    hint: '只用檔案開頭的 import，模組限 react、recharts、lucide-react、kunsu',
  },
  {
    code: 'network_api',
    pattern: /\b(fetch\s*\(|XMLHttpRequest|WebSocket|EventSource|sendBeacon|(?:webkit)?RTCPeerConnection|RTCDataChannel)/,
    message: (m) => `畫布不能連網路（${m.replace(/\s*\($/, '')}）`,
    hint: '沙盒沒有網路：數據直接寫進畫布，圖片放 assets/',
  },
  {
    code: 'storage_api',
    pattern: /\b(localStorage|sessionStorage|indexedDB|document\.cookie)/,
    message: (m) => `畫布不能用 ${m}`,
    hint: '畫布每次都是全新沙盒，狀態用 React useState 就好',
  },
  {
    code: 'parent_access',
    pattern: /\b(window\.(parent|top|opener|open)\b|parent\.postMessage|top\.location)/,
    message: (m) => `畫布不能碰上層視窗（${m}）`,
    hint: '畫布只負責顯示資料；選項與送出由 Kunsu 原生介面處理',
  },
]

/**
 * 資源提示（`<link rel=preconnect|dns-prefetch|preload|prefetch|modulepreload|prerender>`）：CSP 管不到 preconnect，
 * 能對外 TCP 連線、經 DNS 帶資料。值在字串裡，所以比對「只遮註解」的行；`rel`／`relList` 後面不遠處出現這些值就算。
 * 縱深防禦：真正的保證是殼層的引擎設定（INV-20）。
 */
const RESOURCE_HINT: Rule = {
  code: 'network_api',
  pattern: /\brel(?:List)?\b[^\n]{0,24}?\b(dns-prefetch|preconnect|modulepreload|prefetch|preload|prerender)\b/i,
  // 比對結果以這個值結尾（例如 `rel="preconnect`）。
  message: (m) => `畫布不能預先連線或預取（rel=${m.split(/[^\w-]+/).pop()}）`,
  // 提示不能誘導 agent 把被擋的標籤改成註解留著：HTML 畫布的檢查逐行看原文，註解裡寫到一樣會被擋（QA 第 5 輪）。
  hint: '沙盒沒有網路：直接刪掉這個資源提示（rel 是 preconnect、dns-prefetch、preload、prefetch 的那個標籤或設定），不要改成註解或留說明——HTML 畫布的檢查連註解也看；資料直接寫進畫布，圖片放 assets/',
}

/** `at`：比對文字每個字元在原文的位置（解碼過字元參照時，欄位要指回原文）。 */
function ruleIssue(file: string, rule: Rule, line: string, lineNo: number, at?: number[]): CanvasIssue | null {
  const m = line.match(rule.pattern)
  if (!m || m.index === undefined) return null
  return { file, line: lineNo, column: (at ? at[m.index] : m.index) + 1, code: rule.code, message: rule.message(m[0]), hint: rule.hint }
}

/**
 * HTML 規格具名字元參照表裡解碼成 ASCII 的全部項目（其他都解碼成非 ASCII，不影響檢查用的比對）。
 * 由 character-entities（與瀏覽器同一份 WHATWG 表）篩出。
 */
const ASCII_NAMED_REFS: Record<string, string> = {
  AMP: '&', DiacriticalGrave: '`', GT: '>', Hat: '^', LT: '<', NewLine: '\n', QUOT: '"', Tab: '\t', UnderBar: '_',
  VerticalLine: '|', amp: '&', apos: "'", ast: '*', bsol: '\\', colon: ':', comma: ',', commat: '@', dollar: '$',
  equals: '=', excl: '!', fjlig: 'fj', grave: '`', gt: '>', lbrace: '{', lbrack: '[', lcub: '{', lowbar: '_', lpar: '(',
  lsqb: '[', lt: '<', midast: '*', num: '#', percnt: '%', period: '.', plus: '+', quest: '?', quot: '"', rbrace: '}',
  rbrack: ']', rcub: '}', rpar: ')', rsqb: ']', semi: ';', sol: '/', verbar: '|', vert: '|',
}

const CHAR_REF = /&#[xX]([0-9a-fA-F]+);?|&#([0-9]+);?|&([A-Za-z][A-Za-z0-9]*);/g

/**
 * 一行裡的字元參照照瀏覽器解碼（QA 第 6 輪：`rel="&#112;reconnect"` 就是 preconnect）：數字參照（十進位、十六進位，有沒有
 * `;` 都算；0、超出範圍與 surrogate 換成 U+FFFD）與上表的具名參照（要有 `;`）。解碼出的空白一律當一個空格——不新增換行，
 * 行號不變。`at[i]`＝解碼後第 i 個字元在原文的位置（欄位指回原文）。
 */
export function decodeCharRefs(line: string): { text: string; at: number[] } {
  let text = ''
  const at: number[] = []
  let last = 0
  const push = (value: string, from: number) => {
    for (const ch of value) {
      text += /^[\t\n\f\r ]$/.test(ch) ? ' ' : ch
      for (let k = 0; k < ch.length; k++) at.push(from)
    }
  }
  for (const m of line.matchAll(CHAR_REF)) {
    const from = m.index ?? 0
    for (let k = last; k < from; k++) {
      text += line[k]
      at.push(k)
    }
    let value: string | undefined
    if (m[3] !== undefined) {
      value = ASCII_NAMED_REFS[m[3]]
    } else {
      const code = m[1] !== undefined ? parseInt(m[1], 16) : parseInt(m[2], 10)
      const valid = Number.isFinite(code) && code > 0 && code <= 0x10ffff && !(code >= 0xd800 && code <= 0xdfff)
      value = String.fromCodePoint(valid ? code : 0xfffd)
    }
    if (value === undefined) {
      // 不認得的具名參照（解碼成非 ASCII 或根本不是參照）：原樣保留。
      for (let k = from; k < from + m[0].length; k++) {
        text += line[k]
        at.push(k)
      }
    } else {
      push(value, from)
    }
    last = from + m[0].length
  }
  for (let k = last; k < line.length; k++) {
    text += line[k]
    at.push(k)
  }
  return { text, at }
}

const STATEMENT_START = /^\s*(import|export|const|let|var|function|class|type|interface|async|default)\b/

/** 從第 index 行開始的一個 import／export 敘述（最多 30 行，到下一個敘述開頭或分號為止）。 */
function statementText(rawLines: string[], maskedLines: string[], index: number): string {
  const parts = [rawLines[index]]
  if (/;\s*$/.test(maskedLines[index]) || /['"]\s*;?\s*$/.test(rawLines[index])) return parts[0]
  for (let k = index + 1; k < Math.min(rawLines.length, index + 30); k++) {
    if (STATEMENT_START.test(maskedLines[k])) break
    parts.push(rawLines[k])
    if (/['"]\s*;?\s*$/.test(rawLines[k])) break
  }
  return parts.join('\n')
}

function scriptIssues(kind: CanvasKind, src: string): CanvasIssue[] {
  const file = `canvas.${kind}`
  const masked = maskCode(src)
  const commentless = maskCode(src, { strings: false }).split('\n')
  const rawLines = src.split('\n')
  const issues: CanvasIssue[] = []
  masked.split('\n').forEach((line, index) => {
    const lineNo = index + 1
    // import 白名單（模組名在字串裡，從原文取）。
    if (/^\s*(import|export)\b/.test(line)) {
      // 只有 `import 'x'`、`import … from 'x'`、`export {…}／* from 'x'` 帶模組名；其他 export 行的字串不算。
      // 敘述可能跨行（`import {\n a,\n} from 'x'`）：取到下一個敘述開頭之前為止。
      const raw = statementText(rawLines, masked.split('\n'), index)
      const m =
        raw.match(/^\s*import\s*['"]([^'"]+)['"]/) ??
        raw.match(/^\s*import\b[^'"]*?\bfrom\s*['"]([^'"]+)['"]/) ??
        raw.match(/^\s*export\s*(?:\{[^}]*\}|\*(?:\s+as\s+\w+)?)\s*from\s*['"]([^'"]+)['"]/)
      if (m && !(ALLOWED_IMPORTS as readonly string[]).includes(m[1])) {
        issues.push({
          file,
          line: lineNo,
          column: line.search(/\S/) + 1,
          code: 'import_not_allowed',
          message: `不能 import '${m[1]}'`,
          hint: `只能 import ${ALLOWED_IMPORTS.join('、')}；其他套件的功能請自己寫或改用 Recharts`,
        })
      }
    }
    for (const rule of RULES) {
      const issue = ruleIssue(file, rule, line, lineNo)
      if (issue) issues.push(issue)
    }
    const hinted = ruleIssue(file, RESOURCE_HINT, commentless[index] ?? '', lineNo)
    if (hinted) issues.push(hinted)
  })
  return issues.sort((a, b) => (a.line ?? 0) - (b.line ?? 0) || (a.column ?? 0) - (b.column ?? 0))
}

function htmlIssues(src: string): CanvasIssue[] {
  const issues: CanvasIssue[] = []
  src.split('\n').forEach((raw, index) => {
    // 瀏覽器會先解碼屬性值與文字裡的字元參照：先解碼再比對（欄位指回原文）。
    const { text: line, at } = decodeCharRefs(raw)
    const lower = line.toLowerCase()
    const hits = ['https://', 'http://']
      .map((p) => lower.indexOf(p))
      .filter((pos) => pos >= 0 && !lower.startsWith('http://www.w3.org/', pos))
    if (hits.length > 0) {
      issues.push({
        file: 'canvas.html',
        line: index + 1,
        column: at[Math.min(...hits)] + 1,
        code: 'external_url',
        message: '畫布不能連外部網址（沙盒沒有網路）',
        hint: "圖片放進 assets/ 用 './assets/<檔名>'；腳本直接寫在 <script> 裡",
      })
    }
    // HTML 裡的 <script> 一樣不能連網路（含 WebRTC，AC-EC-06）；<link> 資源提示也算。
    for (const rule of [RULES.find((r) => r.code === 'network_api')!, RESOURCE_HINT]) {
      const issue = ruleIssue('canvas.html', rule, line, index + 1, at)
      if (issue) issues.push(issue)
    }
  })
  return issues
}

export function lintCanvas(kind: CanvasKind, src: string): CanvasIssue[] {
  return kind === 'html' ? htmlIssues(src) : scriptIssues(kind, src)
}
