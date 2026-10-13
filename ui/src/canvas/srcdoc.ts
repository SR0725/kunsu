// 證據畫布的沙盒文件（PRD §24；INV inv_canvas_sandboxed）。
// iframe 只有 allow-scripts（無同源 → 拿不到 Kunsu 的 DOM、IPC 或資料）；CSP 擋掉一切網路、表單、子框架與導覽。

import { CANVAS_GUARD_SCRIPT } from './hardening'
import { scanHtml, type HtmlTag } from './htmlTokens'

export const CANVAS_SANDBOX = 'allow-scripts'

export const CANVAS_CSP = [
  "default-src 'none'",
  // blob:＝沙盒自己產生的本地腳本（畫布程式碼，讓錯誤帶行號）；data:＝HTML 畫布的 inline script 組裝成的 data: 腳本
  // （錯誤才帶訊息與行號）。兩者都不是網路來源。
  "script-src 'unsafe-inline' 'unsafe-eval' blob: data:",
  "style-src 'unsafe-inline'",
  'img-src data: blob:',
  'font-src data:',
  'media-src data:',
  "connect-src 'none'",
  "form-action 'none'",
  "frame-src 'none'",
  "child-src 'none'",
  "worker-src 'none'",
  "object-src 'none'",
  "base-uri 'none'",
  "manifest-src 'none'",
].join('; ')

export type CanvasKind = 'tsx' | 'jsx' | 'html'

export interface CanvasDoc {
  /** 沙盒 runtime（React、Recharts、lucide、sucrase、Tailwind、kunsu 模組）。 */
  runtime: string
  kind: CanvasKind
  source: string
  /** `./assets/x.png` → data URI。 */
  assets: Record<string, string>
  /** 這次渲染的 token；訊息不帶它就忽略。 */
  token: string
}

export function newToken(): string {
  const bytes = new Uint8Array(16)
  crypto.getRandomValues(bytes)
  return Array.from(bytes, (b) => b.toString(36).padStart(2, '0')).join('')
}

/** JSON 放進 <script> 內：`<` 一律跳脫，原始碼再怎麼寫都跳不出資料區塊。 */
function scriptJson(value: unknown): string {
  return JSON.stringify(value).replace(/</g, '\\u003c').replace(/\u2028/g, '\\u2028').replace(/\u2029/g, '\\u2029')
}

/**
 * runtime 與加固腳本放進 <script>：`</script` 改寫成等價的 `<\/script`；`<!--` 改寫成等價的 `\x3C!--`（在字串、樣板字串與
 * 正規表示式裡都是同一個字元）——`<!--` 之後再出現 `<script` 會讓 script data 進入 double escaped，Kunsu 自己的
 * `</script>` 就結束不了腳本、一路吃進畫布。目前的 bundle 沒有 `<!--`，這是保險。
 */
function scriptText(code: string): string {
  return code.replace(/<\/(script)/gi, '<\\/$1').replace(/<!--/g, '\\x3C!--')
}

/** 換行數：LF、CR、CRLF 各算一行（和 HTML parser 的前處理一樣）。 */
function lineBreaks(text: string): number {
  return (text.match(/\r\n|\r|\n/g) ?? []).length
}

/**
 * `<link>` 的最後一道（CONTRACT §12；不依賴 tokenizer 切對）：每一個字面的 `<link` 後面接空白、`/`、`>` 或結尾（ASCII 不分
 * 大小寫）一律改成 `&lt;link`。parser 只會從字面的 `<link` 開出 link 元素，所以畫布 HTML 不論瀏覽器怎麼切、拿掉的標籤前後
 * 怎麼拼接（`<<link>link …>`），都產生不了 `<link>`。屬性值、title／textarea 裡解碼後的文字不變；data: 腳本的內容是 URL
 * 編碼，沒有 `<`；`<link-card>` 這類自訂元素不受影響。
 */
function neutralizeLinks(html: string): string {
  return html.replace(/<(link)(?=[\t\n\f\r />]|$)/gi, '&lt;$1')
}

function htmlAttr(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/"/g, '&quot;')
}

/** 拿掉標籤但保留它跨的換行：畫布 HTML 的行號不變（錯誤與檢查指得到 canvas.html 的同一行）。 */
function keepLines(text: string): string {
  return '\n'.repeat(lineBreaks(text))
}

const JS_TYPES = /^(?:|text\/javascript|application\/javascript|text\/ecmascript|application\/ecmascript)$/i

/** 傳統 JavaScript inline script：沒有 `src`，`type`（沒有時看舊的 `language`）是 JavaScript。 */
function classicInline(tag: HtmlTag): boolean {
  const attr = (name: string) => tag.attrs.find((a) => a.name === name)
  if (attr('src')) return false
  const type = attr('type')
  if (type) return JS_TYPES.test(type.value.trim())
  const language = attr('language')?.value.trim()
  return !language || JS_TYPES.test(`text/${language}`)
}

/**
 * WebKit 會把無同源文件裡 inline script 的錯誤遮成 `Script error.`（沒有訊息、行號）；data: 腳本的錯誤不會被遮。
 * 傳統 inline script 改成 data: 腳本：仍是 parser 依序執行的阻塞腳本（document.write、currentScript 照常）；
 * `kunsu-line`＝內容第一行在 canvas.html 的行號，原本的換行放進 `data-kunsu-lines`，行號不變。`async`／`defer` 對 inline
 * script 本來無效，留著會讓 data: 腳本延後執行，所以拿掉（保留換行）。
 */
function dataScript(src: string, start: HtmlTag, contentFrom: number, contentTo: number, end: HtmlTag | null): string {
  const body = src.slice(contentFrom, contentTo)
  const line = lineBreaks(src.slice(0, contentFrom)) + 1
  // 開始標籤名稱之後到 `>` 之前（含換行）；拿掉 async／defer。
  let attrs = ''
  let at = start.nameEnd
  for (const a of start.attrs) {
    if (a.name !== 'async' && a.name !== 'defer') continue
    attrs += src.slice(at, a.from) + keepLines(src.slice(a.from, a.to))
    at = a.to
  }
  attrs += src.slice(at, start.to - 1)
  const url = `data:text/javascript;kunsu-line=${line};charset=utf-8,${encodeURIComponent(body)}`
  return `<script${attrs} src="${url}" data-kunsu-lines="${keepLines(body)}"></script>` + (end ? keepLines(src.slice(end.from, end.to)) : '')
}

const escapeText = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;')

/**
 * HTML 畫布（CONTRACT §12「組裝 HTML 畫布」）：`./assets/x.png`（或 `assets/x.png`）換成快照的 data URI，再用 tokenizer
 * 走過一遍，只動它認為是真正標籤的地方——拿掉 `<link>`（preconnect 等資源提示，CSP 管不到；引擎設定已關閉，這是縱深防禦）與
 * 帶 `http-equiv` 的 `<meta>`（refresh 會換頁；server 的靜態檢查已擋），inline script 改成 data: 腳本（錯誤帶訊息與行號），
 * 沒寫完的標籤拿掉。`<script>`、`<style>` 等 raw text、註解、屬性值裡的字樣不當標籤。tokenizer 是近似的：切錯吞不掉 Kunsu 的
 * 東西（都在畫布之前，畫布 HTML 是文件最後一段，不補結尾），也開不出 `<link>`（最後一道 `neutralizeLinks`）。行號不變。
 */
function assembleHtml(html: string, assets: Record<string, string>): string {
  // 換行先統一成 LF：HTML parser 的前處理本來就把 CRLF、單獨的 CR 換成 LF（解析結果不變），之後拿掉標籤補的換行才不會和
  // 前面的 CR 併成一個 CRLF、少算一行（差分測試找到）；data: 腳本的內容也和原本 inline 時 parser 看到的一樣。
  let src = html.replace(/\r\n?/g, '\n')
  for (const [key, uri] of Object.entries(assets)) {
    const bare = key.replace(/^\.\//, '')
    src = src.split(key).join(uri).split(`"${bare}"`).join(`"${uri}"`).split(`'${bare}'`).join(`'${uri}'`)
  }
  let out = ''
  for (const seg of scanHtml(src)) {
    switch (seg.kind) {
      case 'text':
      case 'comment':
        out += src.slice(seg.from, seg.to)
        break
      case 'tag': {
        const t = seg.tag
        const raw = src.slice(t.from, t.to)
        const dropped =
          !t.complete || (!t.end && (t.name === 'link' || (t.name === 'meta' && t.attrs.some((a) => a.name === 'http-equiv'))))
        out += dropped ? keepLines(raw) : raw
        break
      }
      case 'raw':
        if (seg.start.name === 'script' && classicInline(seg.start)) {
          out += dataScript(src, seg.start, seg.contentFrom, seg.contentTo, seg.end)
        } else {
          out += src.slice(seg.start.from, seg.contentTo) + (seg.end ? src.slice(seg.end.from, seg.end.to) : '')
        }
        break
      case 'plaintext':
        out += `<pre${src.slice(seg.start.nameEnd, seg.start.to - 1)}>${escapeText(src.slice(seg.contentFrom))}</pre>`
        break
    }
  }
  return neutralizeLinks(out)
}

export const CANVAS_BASE_CSS = `
:root { color-scheme: dark; --k-bg: transparent; --k-text: #f2f3f5; --k-text-2: #c3c7ce; --k-muted: #8b919b;
  --k-faint: #5f6570; --k-line: #1d2026; --k-panel: #0b0c0e; --k-accent: #3987e5; --k-good: #83c494; --k-bad: #ef8d86;
  --k-warn: #e0b35a; }
html, body { margin: 0; padding: 0; background: transparent; color: var(--k-text);
  font: 14px/1.5 "Geist", -apple-system, BlinkMacSystemFont, "PingFang TC", "Noto Sans TC", sans-serif;
  -webkit-font-smoothing: antialiased; overflow-x: hidden; }
#root { padding: 2px; }
`

export function buildSrcdoc(doc: CanvasDoc): string {
  // <base href="about:srcdoc"> 在最前面（CSP 之前；CSP 的 base-uri 'none' 照樣擋畫布自己加的 <base>）：srcdoc 文件預設以上層
  // 頁面網址為基準，畫布的 `#id`、`index.html` 會解析成 Kunsu 自己的頁面。改以 about:srcdoc 為基準後，`#id` 在畫布內捲動，
  // 相對網址解析失敗、不會導覽（2026-10-09 原生實驗；殼層另有子框架導覽關卡，INV-20）。
  const head =
    '<base href="about:srcdoc">' +
    `<meta http-equiv="Content-Security-Policy" content="${htmlAttr(CANVAS_CSP)}">` +
    '<meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">' +
    `<style>${CANVAS_BASE_CSS}</style>`
  const open = `<!doctype html><html lang="zh-Hant"><head>${head}</head><body>`
  // 順序（CONTRACT §12「沙盒文件的順序」，QA 第 6 輪）：資料區塊 → 加固腳本 → runtime → <div id="root">＋畫布。Kunsu 的東西
  // 在 parser 讀到畫布之前都已經解析、執行完；HTML 畫布是文件的最後一段，怎麼寫都吞不掉它們（不靠 tokenizer 切對）。
  // 加固腳本在資料區塊之後：安裝時就記下 token（pagehide 回報用）。runtime 再呼叫一次加固（裝過就略過）。
  const guard = `<script>${scriptText(CANVAS_GUARD_SCRIPT)}</script>`
  const runtime = `<script>${scriptText(doc.runtime)}</script>`
  const root = '<div id="root">'
  const html = doc.kind === 'html'
  // 不放檢查／顯示模式：畫布拿不到，兩者行為一致（AC-EC-06）。HTML 畫布也帶原始碼，讓 runtime 跑同一套靜態檢查；
  // line_offset＝畫布第一行之前的換行數，runtime 據此把文件行號換成 canvas.html 的行號。資料區塊的 JSON 沒有換行
  // （JSON 跳脫了 \n、\r），不影響行數。
  const payload = {
    token: doc.token,
    kind: doc.kind,
    source: doc.source,
    assets: html ? {} : doc.assets,
    line_offset: html ? lineBreaks(open + guard + runtime + root) : 0,
  }
  const data = `<script type="application/json" id="kunsu-canvas">${scriptJson(payload)}</script>`
  const prefix = open + data + guard + runtime + root
  return html ? prefix + assembleHtml(doc.source, doc.assets) : `${prefix}</div></body></html>`
}
