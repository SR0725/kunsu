// 沙盒加固腳本（BDD evidence-canvas AC-EC-02、06、09；INV inv_canvas_sandboxed 的縱深防禦）。
// vitest 跑在 node，沒有 DOM：用最小的假 DOM（原生 setter 直接寫屬性表，不經 JS 的 setAttribute）驗證加固邏輯；
// 真 WebKit 的行為由原生實驗驗證。
import { describe, expect, it } from 'vitest'
import { CANVAS_GUARD_SCRIPT, WEBRTC_GLOBALS, installCanvasGuard } from './hardening'
import { parseCanvasMessage } from './messages'
import { failureText, frameReducer, initialFrame } from './frameState'

type Listener = (event: Record<string, unknown>) => void

function fakeDom() {
  // 原生 Node.nodeValue／textContent 對 Attr 節點直接改值（不經 JS 的 Attr.value setter）。
  class FakeNode {
    _text = ''
    _setText(v: string) {
      this._text = String(v)
    }
    get nodeValue(): string {
      return this._text
    }
    set nodeValue(v: string) {
      this._setText(v)
    }
    get textContent(): string {
      return this._text
    }
    set textContent(v: string) {
      this._setText(v)
    }
  }
  class FakeElement extends FakeNode {
    _attrs = new Map<string, string>()
    _html = ''
    setAttribute(name: string, value: string) {
      this._attrs.set(String(name).toLowerCase(), String(value))
    }
    setAttributeNS(_ns: string | null, name: string, value: string) {
      this._attrs.set(String(name).toLowerCase(), String(value))
    }
    getAttribute(name: string): string | null {
      return this._attrs.get(name.toLowerCase()) ?? null
    }
    setAttributeNode(attr: FakeAttr) {
      attr.ownerElement = this
      this._attrs.set(attr.name.toLowerCase(), attr.value)
      return null
    }
    get innerHTML(): string {
      return this._html
    }
    set innerHTML(v: string) {
      this._html = String(v)
    }
    get outerHTML(): string {
      return this._html
    }
    set outerHTML(v: string) {
      this._html = String(v)
    }
    insertAdjacentHTML(_where: string, html: string) {
      this._html += String(html)
    }
  }
  class FakeAttr extends FakeNode {
    name: string
    ownerElement: FakeElement | null = null
    _value = ''
    constructor(name: string) {
      super()
      this.name = name
    }
    _setText(v: string) {
      this._value = String(v)
      this.ownerElement?._attrs.set(this.name.toLowerCase(), this._value)
    }
    get value(): string {
      return this._value
    }
    set value(v: string) {
      this._setText(v)
    }
  }
  class FakeTokenList {
    el: FakeElement
    attr: string
    constructor(el: FakeElement, attr: string) {
      this.el = el
      this.attr = attr
    }
    private tokens(): string[] {
      return (this.el._attrs.get(this.attr) ?? '').split(/\s+/).filter(Boolean)
    }
    get value(): string {
      return this.el._attrs.get(this.attr) ?? ''
    }
    set value(v: string) {
      this.el._attrs.set(this.attr, String(v))
    }
    add(...tokens: string[]) {
      this.el._attrs.set(this.attr, [...this.tokens(), ...tokens].join(' '))
    }
    toggle(token: string): boolean {
      const has = this.tokens().includes(token)
      this.el._attrs.set(this.attr, (has ? this.tokens().filter((t) => t !== token) : [...this.tokens(), token]).join(' '))
      return !has
    }
    replace(from: string, to: string): boolean {
      if (!this.tokens().includes(from)) return false
      this.el._attrs.set(this.attr, this.tokens().map((t) => (t === from ? to : t)).join(' '))
      return true
    }
  }
  class FakeLink extends FakeElement {
    _relList?: FakeTokenList
    get rel(): string {
      return this._attrs.get('rel') ?? ''
    }
    set rel(v: string) {
      this._attrs.set('rel', String(v))
    }
    get relList(): FakeTokenList {
      this._relList ??= new FakeTokenList(this, 'rel')
      return this._relList
    }
    set relList(v: unknown) {
      this._attrs.set('rel', String(v))
    }
  }
  class FakeDiv extends FakeElement {
    _classList?: FakeTokenList
    get classList(): FakeTokenList {
      this._classList ??= new FakeTokenList(this, 'class')
      return this._classList
    }
  }
  class FakeRange {
    createContextualFragment(html: string) {
      return String(html)
    }
  }
  class FakeDocument {
    written = ''
    write(...parts: string[]) {
      this.written += parts.join('')
    }
    writeln(...parts: string[]) {
      this.written += parts.join('') + '\n'
    }
  }
  class FakeDOMParser {
    parseFromString(str: string) {
      return String(str)
    }
  }
  const listeners: Record<string, Listener[]> = {}
  const win: Record<string, unknown> = {
    Node: FakeNode,
    Element: FakeElement,
    HTMLLinkElement: FakeLink,
    DOMTokenList: FakeTokenList,
    Attr: FakeAttr,
    Range: FakeRange,
    Document: FakeDocument,
    DOMParser: FakeDOMParser,
    addEventListener(type: string, fn: Listener) {
      ;(listeners[type] ??= []).push(fn)
    },
  }
  for (const name of WEBRTC_GLOBALS) win[name] = class {}
  return { win, listeners, FakeLink, FakeDiv, FakeAttr, FakeRange, FakeDocument, FakeDOMParser }
}

describe('installCanvasGuard', () => {
  it('移除 WebRTC 全域且無法設回', () => {
    const { win } = fakeDom()
    installCanvasGuard(win)
    for (const name of WEBRTC_GLOBALS) expect(win[name], name).toBeUndefined()
    expect(() => {
      win.RTCPeerConnection = class {}
    }).toThrow()
  })

  it('資源提示：rel setter、setAttribute(NS)、relList、Attr 節點都設不進 preconnect／dns-prefetch／preload／prefetch', () => {
    const { win, FakeLink, FakeAttr } = fakeDom()
    installCanvasGuard(win)
    const link = new FakeLink()
    link.rel = 'preconnect stylesheet'
    expect(link.getAttribute('rel')).toBe('stylesheet')
    link.setAttribute('REL', 'dns-prefetch')
    expect(link.getAttribute('rel')).toBe('')
    link.setAttributeNS(null, 'rel', 'preload icon')
    expect(link.getAttribute('rel')).toBe('icon')
    link.relList.add('prefetch', 'modulepreload', 'alternate')
    expect(link.getAttribute('rel')).toBe('icon alternate')
    expect(link.relList.toggle('preconnect')).toBe(false)
    expect(link.relList.replace('icon', 'preconnect')).toBe(false)
    link.relList.value = 'Preconnect'
    expect(link.getAttribute('rel')).toBe('')
    link.relList = 'dns-prefetch' as unknown as typeof link.relList
    expect(link.getAttribute('rel')).toBe('')
    const attr = new FakeAttr('rel')
    attr.ownerElement = link
    attr.value = 'preconnect'
    expect(link.getAttribute('rel')).toBe('')
    attr.nodeValue = 'dns-prefetch'
    expect(attr.value).toBe('')
    const detached = new FakeAttr('rel')
    detached.value = 'preload'
    link.setAttributeNode(detached)
    expect(link.getAttribute('rel')).toBe('')
  })

  it('其他屬性與 class 不受影響', () => {
    const { win, FakeLink, FakeDiv } = fakeDom()
    installCanvasGuard(win)
    const link = new FakeLink()
    link.setAttribute('href', 'preconnect')
    expect(link.getAttribute('href')).toBe('preconnect')
    const div = new FakeDiv()
    div.setAttribute('class', 'preload')
    div.classList.add('preconnect')
    expect(div.getAttribute('class')).toBe('preload preconnect')
  })

  it('字串解析（innerHTML、outerHTML、insertAdjacentHTML、createContextualFragment、document.write、DOMParser）產生不了 <link>', () => {
    const { win, FakeDiv, FakeRange, FakeDocument, FakeDOMParser } = fakeDom()
    installCanvasGuard(win)
    const div = new FakeDiv()
    div.innerHTML = '<p>x</p><LINK rel=preconnect href=//x.test>'
    expect(div.innerHTML).not.toMatch(/<link\b/i)
    expect(div.innerHTML).toContain('<p>x</p>')
    div.outerHTML = '<link rel="dns-prefetch" href="//y.test">'
    expect(div.outerHTML).not.toMatch(/<link\b/i)
    div.insertAdjacentHTML('beforeend', '<link\nrel=preload href=/z>')
    expect(div.innerHTML).not.toMatch(/<link\b/i)
    expect(new FakeRange().createContextualFragment('<link rel=preconnect href=//a>')).not.toMatch(/<link\b/i)
    const d = new FakeDocument()
    d.write('<link rel=preconnect ', 'href=//b>')
    d.writeln('<link rel=preconnect href=//c>')
    expect(d.written).not.toMatch(/<link\b/i)
    expect(new FakeDOMParser().parseFromString('<link rel=preconnect href=//d>')).not.toMatch(/<link\b/i)
  })

  it('收集 runtime 之前的錯誤（含未處理的 Promise 拒絕），畫布改不掉收集器', () => {
    const { win, listeners } = fakeDom()
    installCanvasGuard(win)
    const dataScript = 'data:text/javascript;kunsu-line=4;charset=utf-8,%0A%20%20boom()'
    for (const fn of listeners.error ?? []) fn({ message: "ReferenceError: Can't find variable: data", lineno: 7, filename: dataScript })
    for (const fn of listeners.unhandledrejection ?? []) fn({ reason: new Error('nope') })
    const read = win.__kunsuEarlyErrors as () => { message: string; line: number; file: string }[]
    // 檔名保留到看得到 kunsu-line（runtime 據此換算 canvas.html 行號）。
    expect(read()).toEqual([
      { message: "ReferenceError: Can't find variable: data", line: 7, file: dataScript },
      { message: 'nope', line: 0, file: '' },
    ])
    expect(() => {
      win.__kunsuEarlyErrors = () => []
    }).toThrow()
    read().length = 0
    expect(read()).toHaveLength(2)
  })

  it('裝兩次不出錯、不重複收集', () => {
    const { win, listeners } = fakeDom()
    installCanvasGuard(win)
    expect(() => installCanvasGuard(win)).not.toThrow()
    expect(listeners.error).toHaveLength(1)
  })

  it('CANVAS_GUARD_SCRIPT 自給自足：單獨當腳本執行就有同樣效果（HTML 畫布在 runtime 之前用它）', () => {
    const { win, FakeLink } = fakeDom()
    new Function('window', CANVAS_GUARD_SCRIPT)(win)
    expect(win.RTCPeerConnection).toBeUndefined()
    const link = new FakeLink()
    link.rel = 'preconnect'
    expect(link.getAttribute('rel')).toBe('')
    expect(typeof win.__kunsuEarlyErrors).toBe('function')
  })
})

// QA 第 5 輪修正（AC-EC-06、06b）：分段 document.write、XML 內部實體、execCommand、XSLT，以及畫布框被換掉時回報。
describe('installCanvasGuard：第 5 輪', () => {
  function writerDom() {
    class Doc {
      written = ''
      closed = 0
      lastCommand: [string, unknown] | null = null
      write(...parts: string[]) {
        this.written += parts.join('')
      }
      writeln(...parts: string[]) {
        this.written += parts.join('') + '\n'
      }
      close() {
        this.closed++
      }
      execCommand(command: string, _ui: boolean, value: unknown) {
        this.lastCommand = [command, value]
        return true
      }
    }
    class Parser {
      parseFromString(text: string) {
        return String(text)
      }
    }
    const listeners: Record<string, { fn: Listener; options: unknown }[]> = {}
    const posted: unknown[] = []
    const win: Record<string, unknown> = {
      Document: Doc,
      DOMParser: Parser,
      parent: { postMessage: (message: unknown) => posted.push(message) },
      addEventListener(type: string, fn: Listener, options?: unknown) {
        ;(listeners[type] ??= []).push({ fn, options })
      },
    }
    return { win, Doc, Parser, listeners, posted }
  }

  it('沒寫完的標籤名稱先留著，下一次 write 合併後再過濾；writeln 照常', () => {
    const { win, Doc } = writerDom()
    installCanvasGuard(win)
    const d = new Doc()
    d.write('<p>a</p><li')
    expect(d.written).toBe('<p>a</p>')
    d.write('nk rel="preconnect" href="//x">')
    expect(d.written).not.toMatch(/<link\b/i)
    expect(d.written).toContain('<p>a</p><kunsu-blocked-link rel="preconnect"')
    d.write('<')
    d.write('/p><b>ok</b>')
    expect(d.written.endsWith('</p><b>ok</b>')).toBe(true)
    d.writeln('<i>x</i>')
    expect(d.written.endsWith('<i>x</i>\n')).toBe(true)
    // 標籤名稱已經完整（只剩屬性沒寫完）就不必留：名稱完整的 <link 在那一次 write 就被改名。
    d.write('<div title="')
    expect(d.written.endsWith('<div title="')).toBe(true)
  })

  it('這段腳本結束（microtask）或 document.close() 時還沒寫完的標籤當文字輸出，不會和後面的原始碼拼成 <link>', async () => {
    const { win, Doc } = writerDom()
    installCanvasGuard(win)
    const d = new Doc()
    d.write('<li')
    expect(d.written).toBe('')
    await Promise.resolve()
    await Promise.resolve()
    expect(d.written).toBe('&lt;li')
    const e = new Doc()
    e.write('x <l')
    e.close()
    expect(e.written).toBe('x &lt;l')
    expect(e.closed).toBe(1)
  })

  it('DOMParser 的 <!ENTITY 宣告改成解析錯誤（XML 內部實體能拼出 <link>）；結尾標籤 </h:link> 一起改名', () => {
    const { win, Parser } = writerDom()
    installCanvasGuard(win)
    const out = new Parser().parseFromString(
      '<!DOCTYPE r [<!ENTITY e "&#60;link rel=\'preconnect\'/>">]><r xmlns:h="http://www.w3.org/1999/xhtml"><h:link></h:link>&e;</r>',
    )
    expect(out).not.toMatch(/<!ENTITY/i)
    expect(out).not.toMatch(/<\/?h:link\b/)
  })

  it("execCommand('insertHTML') 的 HTML 一樣不能帶 <link>；其他指令不動", () => {
    const { win, Doc } = writerDom()
    installCanvasGuard(win)
    const d = new Doc()
    d.execCommand('insertHTML', false, '<link rel="preconnect" href="//x"><b>b</b>')
    expect(String(d.lastCommand?.[1])).not.toMatch(/<link\b/i)
    d.execCommand('bold', false, '<link>')
    expect(d.lastCommand).toEqual(['bold', '<link>'])
  })

  it('移除 XSLTProcessor（畫布用不到；轉換結果是程式產生的，字串過濾擋不住）', () => {
    const { win } = writerDom()
    win.XSLTProcessor = class {}
    installCanvasGuard(win)
    expect(win.XSLTProcessor).toBeUndefined()
    expect(() => {
      win.XSLTProcessor = class {}
    }).toThrow()
  })

  it('畫布文件被換掉（pagehide，capture）時送出這次 token 的 error：canvas_navigation「畫布換頁了」', () => {
    const { win, listeners, posted } = writerDom()
    win.document = {
      getElementById: (id: string) => (id === 'kunsu-canvas' ? { textContent: JSON.stringify({ token: 'tok-9', kind: 'html' }) } : null),
    }
    installCanvasGuard(win)
    const hide = listeners.pagehide ?? []
    expect(hide).toHaveLength(1)
    expect(hide[0].options).toMatchObject({ capture: true })
    hide[0].fn({})
    expect(posted).toHaveLength(1)
    expect(posted[0]).toMatchObject({
      kunsu: 'tok-9',
      type: 'error',
      issues: [{ file: 'canvas.html', code: 'canvas_navigation', message: '畫布換頁了' }],
    })
    const parsed = parseCanvasMessage(posted[0], 'tok-9')
    expect(parsed?.type).toBe('error')
    expect(failureText(frameReducer(initialFrame, { type: 'message', message: parsed! }))).toBe('圖表載入失敗：畫布換頁了')
  })

  it('讀不到資料區塊（畫布把它刪了）就不送：沒有 token 的訊息面板本來就不收', () => {
    const { win, listeners, posted } = writerDom()
    win.document = { getElementById: () => null }
    installCanvasGuard(win)
    for (const { fn } of listeners.pagehide ?? []) fn({})
    expect(posted).toEqual([])
  })

  // QA 第 6 輪（CONTRACT §12「沙盒文件的順序」）：資料區塊在加固腳本之前，加固腳本安裝時就記下 token 與上層視窗——畫布之後
  // 刪掉資料區塊、用 <frameset> 換掉 body 或改掉 window.parent，pagehide 一樣送到 Kunsu。
  it('安裝時就記下 token 與上層視窗：畫布之後刪掉資料區塊、改掉 window.parent 也照送', () => {
    const { win, listeners, posted } = writerDom()
    let block: { textContent: string } | null = { textContent: JSON.stringify({ token: 'tok-7', kind: 'html' }) }
    win.document = { getElementById: (id: string) => (id === 'kunsu-canvas' ? block : null) }
    installCanvasGuard(win)
    block = null
    const swallowed: unknown[] = []
    win.parent = { postMessage: (message: unknown) => swallowed.push(message) }
    for (const { fn } of listeners.pagehide ?? []) fn({})
    expect(swallowed).toEqual([])
    expect(posted).toHaveLength(1)
    expect(posted[0]).toMatchObject({ kunsu: 'tok-7', type: 'error', issues: [{ file: 'canvas.html', code: 'canvas_navigation' }] })
  })
})
