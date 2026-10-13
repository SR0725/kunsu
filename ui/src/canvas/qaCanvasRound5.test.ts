// QA 第 5 輪（BDD evidence-canvas AC-EC-02、AC-EC-06；INV inv_canvas_sandboxed 的縱深防禦）：重現測試。
// vitest 跑在 node，沒有 DOM：組裝結果用字串檢查；加固腳本用最小的假 DOM（原生解析器直接產生節點，不經 JS setter）。
// 真 WebKit 的行為由 QA 的原生實驗驗證（見 QA 回報）。
import { describe, expect, it } from 'vitest'
import { buildSrcdoc } from './srcdoc'
import { WEBRTC_GLOBALS, installCanvasGuard } from './hardening'

const DATA_BLOCK = '<script type="application/json" id="kunsu-canvas">'

function assemble(source: string): string {
  return buildSrcdoc({ runtime: 'window.__RUNTIME__ = 1', kind: 'html', source, assets: {}, token: 'tok-5' })
}

/** 畫布 HTML 組裝後的區段（`<div id="root">` 之後；QA 第 6 輪起資料區塊在畫布之前，區段到文件結尾）。 */
function canvasRegion(html: string): string {
  const start = html.indexOf('<div id="root">')
  const data = html.indexOf(DATA_BLOCK, start)
  return html.slice(start, data < 0 ? html.length : data)
}

/** 區段裡每個 data: 腳本解碼後的程式碼。 */
function dataScripts(region: string): string[] {
  return [...region.matchAll(/<script\b[^>]*\bsrc="data:text\/javascript;[^,"]*,([^"]*)"/g)].map((m) => decodeURIComponent(m[1]))
}

describe('QA 第 5 輪：HTML 畫布組裝時拿掉 <link>／<meta http-equiv> 不能吃掉 <script> 裡的程式碼（AC-EC-02）', () => {
  // 原生實驗（真 WKWebView、產品的 buildSrcdoc 與 runtime）：同一個有 bug 的畫布，沒有這行註解時回報
  // `canvas_runtime` 第 3 行「ReferenceError: Can't find variable: data」；多了這行註解，`<link` 到 `</script>` 的 `>` 被當成
  // 一個標籤拿掉，畫布的 <script> 沒有結束標籤、吞掉資料區塊，runtime 讀到空的 payload，只送出沒有 token 的
  // 「Script error.」（面板忽略）——結果是 2 秒逾時（檢查回 `canvas_timeout`，提示查無窮迴圈），連沒有 bug 的畫布也一樣失敗。
  it('腳本註解提到 "<link" 時，腳本照樣轉成 data: 腳本、帶 canvas.html 行號，而且在資料區塊之前結束', () => {
    const source =
      '<div id="chart"></div>\n<script>\nconst rows = data.map((r) => r * 2)\n// 註：沙盒不能用 <link 載入樣式\n</script>\n<p>ok</p>\n'
    const region = canvasRegion(assemble(source))
    expect(region).toMatch(/<script src="data:text\/javascript;kunsu-line=2;/)
    expect((region.match(/<script\b/gi) ?? []).length).toBe((region.match(/<\/script\s*>/gi) ?? []).length)
    expect(dataScripts(region).join('\n')).toContain('const rows = data.map((r) => r * 2)')
    expect(region).toContain('<p>ok</p>')
  })

  it('腳本註解提到 "<meta http-equiv" 時一樣不能吃掉 </script>', () => {
    const source = '<div id="c"></div>\n<script>\ndocument.getElementById("c").textContent = "ok"\n// 不要用 <meta http-equiv=refresh 換頁\n</script>\n'
    const region = canvasRegion(assemble(source))
    expect((region.match(/<script\b/gi) ?? []).length).toBe((region.match(/<\/script\s*>/gi) ?? []).length)
    expect(dataScripts(region).join('\n')).toContain('document.getElementById("c").textContent = "ok"')
  })

  it('字串裡的 "<link …>" 是程式碼，不是標籤：執行的程式碼要和 canvas.html 一樣', () => {
    const source = '<script>\nconst parts = ["<link ", href, ">"]\nconsole.log(parts.length)\n</script>\n'
    expect(dataScripts(canvasRegion(assemble(source))).join('\n')).toContain('const parts = ["<link ", href, ">"]')
  })
})

/** 最小假 DOM：原生解析器（DOMParser、document.write、XSLTProcessor）產生的節點直接帶屬性，不經 JS setter。 */
function parserDom() {
  class FakeNode {}
  class FakeElement extends FakeNode {
    _attrs = new Map<string, string>()
    localName: string
    namespaceURI: string
    constructor(localName = 'div', namespaceURI = 'http://www.w3.org/1999/xhtml') {
      super()
      this.localName = localName
      this.namespaceURI = namespaceURI
    }
    setAttribute(name: string, value: string) {
      this._attrs.set(String(name).toLowerCase(), String(value))
    }
    getAttribute(name: string): string | null {
      return this._attrs.get(name.toLowerCase()) ?? null
    }
  }
  class FakeLink extends FakeElement {
    constructor() {
      super('link')
    }
    get rel(): string {
      return this._attrs.get('rel') ?? ''
    }
    set rel(v: string) {
      this._attrs.set('rel', String(v))
    }
  }
  /** XML 解析：把 `<prefix:link … rel="…">`（XHTML 命名空間）解析成真的 <link> 元素。 */
  function parseXmlLinks(text: string): FakeLink[] {
    const links: FakeLink[] = []
    for (const m of text.matchAll(/<(?:[\w-]+:)?link\b([^>]*)>/gi)) {
      const link = new FakeLink()
      for (const a of m[1].matchAll(/([\w:-]+)="([^"]*)"/g)) link._attrs.set(a[1].toLowerCase(), a[2])
      links.push(link)
    }
    return links
  }
  class FakeDOMParser {
    parseFromString(text: string) {
      return { links: parseXmlLinks(String(text)) }
    }
  }
  class FakeDocument {
    stream = ''
    write(...parts: string[]) {
      this.stream += parts.join('')
    }
    writeln(...parts: string[]) {
      this.stream += parts.join('') + '\n'
    }
    /** HTML parser 依序讀進來的內容裡出現的 <link>。 */
    parsedLinks(): FakeLink[] {
      return parseXmlLinks(this.stream)
    }
  }
  class FakeXSLTProcessor {
    output = ''
    importStylesheet(xsl: { links?: FakeLink[]; output?: string }) {
      this.output = xsl.output ?? ''
    }
    transformToFragment() {
      return { links: parseXmlLinks(this.output) }
    }
    transformToDocument() {
      return { links: parseXmlLinks(this.output) }
    }
  }
  const win: Record<string, unknown> = {
    Node: FakeNode,
    Element: FakeElement,
    HTMLLinkElement: FakeLink,
    Document: FakeDocument,
    DOMParser: FakeDOMParser,
    XSLTProcessor: FakeXSLTProcessor,
    addEventListener() {},
  }
  for (const name of WEBRTC_GLOBALS) win[name] = class {}
  return { win, FakeDocument, FakeDOMParser, FakeXSLTProcessor }
}

const HINT = /^(?:preconnect|dns-prefetch|preload|prefetch|modulepreload|prerender)$/i
const hinted = (links: { rel: string }[]) => links.filter((l) => l.rel.split(/\s+/).some((t) => HINT.test(t)))

describe('QA 第 5 輪：加固腳本擋得住字串解析產生的資源提示 <link>（CONTRACT §12 縱深防禦；AC-EC-06）', () => {
  // 原生實驗（真 WKWebView、產品的 srcdoc 與加固腳本、關掉引擎設定作對照）：下面三種寫法都在畫布文件裡產生
  // `HTMLLinkElement rel=preconnect`，對本機 listener 開出 TCP 連線；打開產品的引擎設定後沒有連線——目前只剩引擎這一層。
  it('XML 解析（DOMParser application/xml）的命名空間前綴 <h:link> 不能留下 rel=preconnect', () => {
    const { win } = parserDom()
    installCanvasGuard(win)
    const Parser = win.DOMParser as new () => { parseFromString(t: string, type: string): { links: { rel: string }[] } }
    const doc = new Parser().parseFromString(
      '<root xmlns:h="http://www.w3.org/1999/xhtml"><h:link rel="preconnect" href="http://127.0.0.1:9/"/></root>',
      'application/xml',
    )
    expect(hinted(doc.links)).toEqual([])
  })

  it('HTML 畫布分段 document.write（"<li" + "nk rel=preconnect …>"）不能讓 parser 讀到資源提示 <link>', () => {
    const { win, FakeDocument } = parserDom()
    installCanvasGuard(win)
    const doc = new FakeDocument()
    doc.write('<li')
    doc.write('nk rel="preconnect" href="http://127.0.0.1:9/">')
    expect(hinted(doc.parsedLinks())).toEqual([])
  })

  it('XSLTProcessor 轉出來的 <link rel=preconnect> 也要擋（或沙盒裡沒有 XSLTProcessor）', () => {
    const { win } = parserDom()
    installCanvasGuard(win)
    const Xslt = win.XSLTProcessor as (new () => { importStylesheet(x: unknown): void; transformToFragment(): { links: { rel: string }[] } }) | undefined
    if (Xslt === undefined) return
    const proc = new Xslt()
    proc.importStylesheet({ output: '<link rel="preconnect" href="http://127.0.0.1:9/">' })
    expect(hinted(proc.transformToFragment().links)).toEqual([])
  })
})
