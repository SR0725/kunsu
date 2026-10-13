// QA 第 6 輪（BDD evidence-canvas AC-EC-02、AC-EC-06；CONTRACT §12「組裝 HTML 畫布」）：重現測試。
// vitest 跑在 node，沒有 DOM：組裝結果用字串檢查。每一案都先在真 WKWebView（產品的 buildSrcdoc＋建置後的 canvas-runtime.js，
// 沙盒 iframe allow-scripts）確認過：瀏覽器的樹建構和 htmlTokens.ts 的近似切法不同時，Kunsu 放在畫布 HTML 之後的資料區塊與
// runtime 會被吞掉（runtime 沒執行、面板收不到 ready／error → 2 秒逾時；檢查回 canvas_timeout，提示查無窮迴圈），或真正的
// <link> 留在文件裡。
//
// 斷言只要求「瀏覽器那邊吞不掉」：資料區塊與 runtime 已經放在畫布 HTML 之前（那就怎麼寫都吞不掉），或組裝補上了瀏覽器
// 真的需要的結尾／中和。
import { describe, expect, it } from 'vitest'
import { buildSrcdoc } from './srcdoc'
import { lintCanvas } from './lint'

const DATA_BLOCK = '<script type="application/json" id="kunsu-canvas">'
const RUNTIME = 'window.__RUNTIME__ = 1'

function assemble(source: string): string {
  return buildSrcdoc({ runtime: RUNTIME, kind: 'html', source, assets: {}, token: 'tok-6' })
}

/** 資料區塊與 runtime 都在畫布 HTML（`<div id="root">`）之前：畫布 HTML 怎麼寫都吞不掉它們。 */
function kunsuBeforeCanvas(html: string): boolean {
  const root = html.indexOf('<div id="root">')
  const data = html.indexOf(DATA_BLOCK)
  const runtime = html.indexOf(RUNTIME)
  return root >= 0 && data >= 0 && runtime >= 0 && data < root && runtime < root
}

/** 畫布區段：`<div id="root">` 之後到資料區塊之前（資料區塊移到前面時到文件結尾）。 */
function region(html: string): string {
  const start = html.indexOf('<div id="root">') + '<div id="root">'.length
  const data = html.indexOf(DATA_BLOCK, start)
  return html.slice(start, data < 0 ? html.length : data)
}

describe('QA 第 6 輪：畫布 HTML 吞掉 Kunsu 的資料區塊與 runtime（AC-EC-02「怎麼寫都吞不掉」）', () => {
  // WKWebView：tokenizer 把結尾的 `</` 當文字，組裝在後面補 `</template>`——`</</template>` 是 bogus comment，template 沒關，
  // 資料區塊與 runtime 變成 template 內容（不執行）。面板 2 秒逾時。
  it('沒關的 <template> 後面接沒寫完的 `</`：補上的 </template> 不能被吃成 bogus comment', () => {
    const html = assemble('<p>t</p>\n<template></')
    if (kunsuBeforeCanvas(html)) return
    const part = region(html)
    expect(part).not.toContain('</</template>')
    expect(part.trimEnd().endsWith('</template></div>') || part.trimEnd().endsWith('</template>')).toBe(true)
  })

  // WKWebView：`<svg>`／`<math>` 裡對不上的 HTML 結束標籤（`</span>`、`</b>`）在瀏覽器只是被忽略，外來元素仍開著；
  // tokenizer 卻當成「跳回 HTML」、不補結尾。根 div 已被畫布自己的 `</div>` 關掉、或中間隔著 <object>／<table> 這類 scope
  // 邊界時，Kunsu 的 `</div>` 也關不掉它——資料區塊與 runtime 變成 SVG／MathML 的 script（不是 HTML script，runtime 不執行）。
  it.each([
    ['</div><math></span>', 'math'],
    ['<object><svg></b>', 'svg'],
    ['<table><svg></span>', 'svg'],
  ])('%s：瀏覽器仍開著的 <%s> 要由組裝關掉', (source, element) => {
    const html = assemble(`<p>x</p>\n${source}`)
    if (kunsuBeforeCanvas(html)) return
    const part = region(html)
    expect(part.lastIndexOf(`</${element}>`)).toBeGreaterThan(part.lastIndexOf(`<${element}>`))
  })

  // WKWebView：integration point（desc、foreignObject）裡沒關的 HTML 元素讓 `</desc>`／`</foreignObject>` 被忽略（瀏覽器仍在
  // HTML 內容裡）；math 的 annotation-xml 裡的 <svg> 是 SVG 命名空間（foreignObject 是 integration point）。tokenizer 以為回到
  // 外來內容，把 <plaintext> 當成外來元素原樣留著——瀏覽器進入 PLAINTEXT，後面一切變成文字。
  it.each([
    '<svg><desc><b></desc><plaintext>',
    '<svg><foreignObject><div></foreignObject><plaintext>',
    '<math><annotation-xml><svg><foreignObject><plaintext>',
  ])('%s：瀏覽器當成 HTML 的 <plaintext> 要中和', (source) => {
    const html = assemble(`<p>x</p>\n${source}`)
    if (kunsuBeforeCanvas(html)) return
    expect(region(html)).not.toMatch(/<plaintext\b/i)
  })

  // WKWebView（舊版 in select 規則）：<select> 裡的 <STYLE/> 被忽略、不進 RAWTEXT，後面的 <script> 是真的腳本、一路到 EOF；
  // tokenizer 以為還在 style 裡，只補 </style>。資料區塊被吃進畫布的腳本，runtime 讀到空的 payload，送出沒有 token 的錯誤（面板忽略）。
  it('<select><STYLE/><script>：瀏覽器眼中的 <script> 要在資料區塊之前結束', () => {
    const html = assemble('<p>x</p>\n<select><STYLE/><script>')
    if (kunsuBeforeCanvas(html)) return
    const part = region(html)
    expect(part.lastIndexOf('</script')).toBeGreaterThan(part.lastIndexOf('<script>'))
  })
})

describe('QA 第 6 輪：HTML 畫布的 <link> 在組裝時拿掉（AC-EC-06 縱深防禦）', () => {
  // WKWebView：`</b>` 讓 tokenizer 以為離開了 svg，把 <title> 當 HTML 的 RCDATA；瀏覽器仍在 svg，<title> 是 integration point，
  // <b> 之後的 <link rel=preconnect> 是真的 HTMLLinkElement（關掉引擎設定時對本機 listener 開出 TCP 連線；引擎設定打開後沒有）。
  it('外來內容判斷錯誤時，真正的 <link> 不能留在組裝結果裡', () => {
    const html = assemble('<p>x</p>\n<svg></b><title><b><link rel="preconnect" href="//127.0.0.1:9/">\n')
    expect(region(html)).not.toMatch(/<link\b/i)
  })

  // 靜態檢查逐行比對原文：屬性值的字元參照（`&#112;reconnect` 解碼後是 preconnect）比對不到。
  it('rel 用字元參照寫成 preconnect 時，HTML 畫布的靜態檢查一樣回報 network_api', () => {
    const issues = lintCanvas('html', '<p>x</p>\n<link rel="&#112;reconnect" href="//127.0.0.1:9/">\n')
    expect(issues.map((i) => i.code)).toContain('network_api')
  })
})
