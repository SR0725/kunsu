// 沙盒 runtime 的縱深防禦與診斷去重（BDD evidence-canvas AC-EC-02、06、09）。

/** CSP 管不到 WebRTC：在執行畫布程式碼前把建構子拿掉，並鎖住不讓畫布設回。 */
export const WEBRTC_GLOBALS = [
  'RTCPeerConnection',
  'webkitRTCPeerConnection',
  'RTCDataChannel',
  'RTCSessionDescription',
  'RTCIceCandidate',
  'RTCRtpSender',
  'RTCRtpReceiver',
  'RTCRtpTransceiver',
  'RTCDtlsTransport',
  'RTCIceTransport',
  'RTCSctpTransport',
  'RTCCertificate',
] as const

export function disableNetworkGlobals(target: object): void {
  for (const name of WEBRTC_GLOBALS) {
    try {
      Object.defineProperty(target, name, { value: undefined, writable: false, configurable: false })
    } catch {
      // 已經不可設定（瀏覽器內建或先前鎖過）：盡量刪掉。
      try {
        delete (target as Record<string, unknown>)[name]
      } catch {
        // 刪不掉就算了；CSP 與導覽白名單仍在。
      }
    }
  }
}

/* eslint-disable @typescript-eslint/no-explicit-any -- 對瀏覽器內建原型動態加固，型別只能是 any。 */
/**
 * 沙盒文件的加固腳本：在任何畫布程式碼之前執行（INV-20 的縱深防禦；真正的保證是殼層的引擎設定，CONTRACT §12 列出
 * 已知繞過）。
 * 1. 移除 WebRTC 全域與 `XSLTProcessor`（轉換結果是程式產生的，字串過濾擋不住；畫布用不到）。
 * 2. 擋資源提示：`<link rel=preconnect|dns-prefetch|preload|prefetch|modulepreload|prerender>` 不能經 DOM API（`rel`
 *    setter、`setAttribute(NS)`、`relList`、Attr 節點）設定；字串解析（`innerHTML`、`DOMParser` 的 HTML 與 XML、
 *    `document.write`、`execCommand('insertHTML')` 等）的輸入裡的 `<link>`／`<前綴:link>` 改成無作用的元素名稱，`<!ENTITY`
 *    改成解析錯誤（XML 內部實體能拼出標籤）。`document.write` 跨呼叫拆開的標籤名稱先留著，下一次 write 合併後再過濾；
 *    這段腳本結束（microtask）或 `document.close()` 時還沒寫完就當文字輸出。畫布自己開的子框架不經這裡，只靠引擎設定。
 * 3. 收集錯誤（runtime 的錯誤監聽之外的後備：HTML 畫布在文件 load 時再讀一次），由 runtime 以 `canvas_runtime` 回報。
 * 4. 畫布文件被換掉（導到 about:blank、blob: 或重新載入）時，在 `pagehide`（capture，比畫布自己的監聽先跑）送出這次
 *    token 的 `error`（`canvas_navigation`），面板顯示「圖表載入失敗：畫布換頁了」並記 `canvas_failed`，不留無聲的白框。
 *    token 與上層視窗在安裝時就記下（資料區塊在加固腳本之前）。
 *
 * srcdoc 在資料區塊之後、runtime 與任何畫布內容之前放字串版（`CANVAS_GUARD_SCRIPT`）；runtime 載入時再呼叫一次（裝過就
 * 略過）。必須自給自足——不引用這個模組的其他識別字——`toString()` 才能直接放進 srcdoc。
 */
export function installCanvasGuard(win: any): void {
  if (!win || win.__kunsuGuard) return
  const O = Object
  const lock = (obj: any, name: string, value: unknown): void => {
    try {
      O.defineProperty(obj, name, { value, writable: false, configurable: false })
    } catch {
      try {
        delete obj[name]
      } catch {
        // 刪不掉就算了；引擎設定、CSP 與導覽白名單仍在。
      }
    }
  }
  lock(win, '__kunsuGuard', true)

  // 1. WebRTC（與 WEBRTC_GLOBALS 相同；測試檢查兩份一致）。
  const rtc = [
    'RTCPeerConnection',
    'webkitRTCPeerConnection',
    'RTCDataChannel',
    'RTCSessionDescription',
    'RTCIceCandidate',
    'RTCRtpSender',
    'RTCRtpReceiver',
    'RTCRtpTransceiver',
    'RTCDtlsTransport',
    'RTCIceTransport',
    'RTCSctpTransport',
    'RTCCertificate',
  ]
  for (const name of rtc) lock(win, name, undefined)
  lock(win, 'XSLTProcessor', undefined)

  // 2. 資源提示。
  const hint = /^(?:preconnect|dns-prefetch|preload|prefetch|modulepreload|prerender)$/i
  const clean = (value: unknown): string =>
    String(value)
      .split(/\s+/)
      .filter((token) => token !== '' && !hint.test(token))
      .join(' ')
  const isRel = (name: unknown): boolean => /^(?:[^:]*:)?rel$/i.test(String(name))
  // `<link`、`</link`、`<h:link`（XML 命名空間前綴）改名；`<!ENTITY` 改成解析錯誤。
  const noLinks = (html: unknown): unknown =>
    typeof html === 'string'
      ? html
          .replace(/(<\/?)((?:[^\s<>/:"'=]+:)?)(link)(?=[\s/>]|$)/gi, '$1$2kunsu-blocked-$3')
          .replace(/<!ENTITY/gi, '<!KUNSU-BLOCKED-ENTITY')
      : html
  const protoOf = (name: string): any => (win[name] && win[name].prototype) || null
  /** 包住 setter（getter 不變）；包好的屬性不能再被改回去。 */
  const wrapSet = (target: any, prop: string, map: (this: any, value: unknown) => unknown): void => {
    const d = target ? O.getOwnPropertyDescriptor(target, prop) : undefined
    if (!d || !d.set || !d.configurable) return
    const set = d.set
    O.defineProperty(target, prop, {
      get: d.get,
      set(this: any, value: unknown) {
        set.call(this, map.call(this, value))
      },
      enumerable: d.enumerable,
      configurable: false,
    })
  }
  const wrapFn = (target: any, name: string, wrap: (orig: (...args: any[]) => any) => (this: any, ...args: any[]) => any): void => {
    const d = target ? O.getOwnPropertyDescriptor(target, name) : undefined
    if (!d || typeof d.value !== 'function' || !d.configurable) return
    O.defineProperty(target, name, { value: wrap(d.value), writable: false, enumerable: d.enumerable, configurable: false })
  }

  // Attr 節點的原生 value setter（先留著，包 setAttributeNode 時用）。
  const attrValue = protoOf('Attr') ? O.getOwnPropertyDescriptor(protoOf('Attr'), 'value') : undefined
  const viaAttrNode = (orig: (...args: any[]) => any) =>
    function (this: any, attr: any) {
      if (attr && isRel(attr.name) && attrValue && attrValue.set) attrValue.set.call(attr, clean(attr.value))
      return orig.call(this, attr)
    }

  const El = protoOf('Element')
  wrapFn(El, 'setAttribute', (orig) =>
    function (this: any, name: unknown, value: unknown) {
      return orig.call(this, name, isRel(name) ? clean(value) : value)
    },
  )
  wrapFn(El, 'setAttributeNS', (orig) =>
    function (this: any, ns: unknown, name: unknown, value: unknown) {
      return orig.call(this, ns, name, isRel(name) ? clean(value) : value)
    },
  )
  wrapFn(El, 'setAttributeNode', viaAttrNode)
  wrapFn(El, 'setAttributeNodeNS', viaAttrNode)
  wrapFn(protoOf('NamedNodeMap'), 'setNamedItem', viaAttrNode)
  wrapFn(protoOf('NamedNodeMap'), 'setNamedItemNS', viaAttrNode)
  wrapSet(protoOf('Attr'), 'value', function (this: any, value: unknown) {
    return isRel(this.name) ? clean(value) : value
  })
  const relAttrNode = function (this: any, value: unknown) {
    return win.Attr && this instanceof win.Attr && isRel(this.name) ? clean(value) : value
  }
  wrapSet(protoOf('Node'), 'nodeValue', relAttrNode)
  wrapSet(protoOf('Node'), 'textContent', relAttrNode)

  const Link = protoOf('HTMLLinkElement')
  wrapSet(Link, 'rel', clean)
  // relList：記下 <link> 的 token list，只對它們過濾（class 等其他 token list 不受影響）。
  const relLists = new WeakSet<object>()
  const relList = Link ? O.getOwnPropertyDescriptor(Link, 'relList') : undefined
  if (relList && relList.get && relList.configurable) {
    const get = relList.get
    const set = relList.set
    O.defineProperty(Link, 'relList', {
      get(this: any) {
        const list = get.call(this)
        if (list && typeof list === 'object') relLists.add(list)
        return list
      },
      set(this: any, value: unknown) {
        if (set) set.call(this, clean(value))
      },
      enumerable: relList.enumerable,
      configurable: false,
    })
  }
  const Tokens = protoOf('DOMTokenList')
  wrapFn(Tokens, 'add', (orig) =>
    function (this: any, ...tokens: unknown[]) {
      return orig.apply(this, relLists.has(this) ? tokens.filter((t) => !hint.test(String(t))) : tokens)
    },
  )
  wrapFn(Tokens, 'toggle', (orig) =>
    function (this: any, ...args: unknown[]) {
      return relLists.has(this) && hint.test(String(args[0])) ? false : orig.apply(this, args)
    },
  )
  wrapFn(Tokens, 'replace', (orig) =>
    function (this: any, ...args: unknown[]) {
      return relLists.has(this) && hint.test(String(args[1])) ? false : orig.apply(this, args)
    },
  )
  wrapSet(Tokens, 'value', function (this: any, value: unknown) {
    return relLists.has(this) ? clean(value) : value
  })

  // 字串解析產生的 <link> 一律換成無作用的元素。
  for (const name of ['Element', 'ShadowRoot']) {
    wrapSet(protoOf(name), 'innerHTML', noLinks)
    wrapFn(protoOf(name), 'setHTMLUnsafe', (orig) =>
      function (this: any, html: unknown, ...rest: unknown[]) {
        return orig.call(this, noLinks(html), ...rest)
      },
    )
  }
  wrapSet(El, 'outerHTML', noLinks)
  wrapFn(El, 'insertAdjacentHTML', (orig) =>
    function (this: any, where: unknown, html: unknown) {
      return orig.call(this, where, noLinks(html))
    },
  )
  wrapFn(protoOf('Range'), 'createContextualFragment', (orig) =>
    function (this: any, html: unknown) {
      return orig.call(this, noLinks(html))
    },
  )
  wrapFn(protoOf('DOMParser'), 'parseFromString', (orig) =>
    function (this: any, html: unknown, ...rest: unknown[]) {
      return orig.call(this, noLinks(html), ...rest)
    },
  )
  // document.write：拆在兩次呼叫之間的標籤名稱（`write('<li')`＋`write('nk …>')`）先留著，合併後再過濾。
  const Doc = protoOf('Document')
  const docWrite = Doc && O.getOwnPropertyDescriptor(Doc, 'write')
  const origWrite = docWrite && typeof docWrite.value === 'function' ? docWrite.value : null
  const held = new WeakMap<object, string>()
  const tail = /<\/?(?:[A-Za-z][^\t\n\f\r />]*)?$/
  const later = (fn: () => void): void => {
    if (typeof win.queueMicrotask === 'function') win.queueMicrotask(fn)
    else void Promise.resolve().then(fn)
  }
  /** 還沒寫完的標籤當文字輸出（不會和後面的文件原始碼拼成標籤）。 */
  const flush = (doc: any): void => {
    const rest = held.get(doc)
    if (rest === undefined) return
    held.delete(doc)
    origWrite.call(doc, '&lt;' + rest.slice(1))
  }
  const guardedWrite = (doc: any, text: string): unknown => {
    let out = (held.get(doc) ?? '') + text
    held.delete(doc)
    const m = tail.exec(out)
    if (m) {
      held.set(doc, out.slice(m.index))
      out = out.slice(0, m.index)
      later(() => flush(doc))
    }
    return out === '' ? undefined : origWrite.call(doc, noLinks(out))
  }
  if (origWrite) {
    wrapFn(Doc, 'write', () =>
      function (this: any, ...parts: unknown[]) {
        return guardedWrite(this, parts.join(''))
      },
    )
    wrapFn(Doc, 'writeln', () =>
      function (this: any, ...parts: unknown[]) {
        return guardedWrite(this, parts.join('') + '\n')
      },
    )
    wrapFn(Doc, 'close', (orig) =>
      function (this: any, ...args: unknown[]) {
        flush(this)
        return orig.apply(this, args)
      },
    )
  }
  wrapFn(Doc, 'execCommand', (orig) =>
    function (this: any, command: unknown, ...rest: unknown[]) {
      if (String(command).toLowerCase() === 'inserthtml' && rest.length > 1) rest[1] = noLinks(rest[1])
      return orig.call(this, command, ...rest)
    },
  )
  wrapFn(win.Document, 'parseHTMLUnsafe', (orig) =>
    function (this: any, html: unknown, ...rest: unknown[]) {
      return orig.call(this, noLinks(html), ...rest)
    },
  )

  // 3. 錯誤收集（runtime 監聽之外的後備；畫布改不掉收集器，回傳的是副本）。來源檔名保留到看得到 data: 腳本的 kunsu-line。
  const errors: { message: string; line: number; file: string }[] = []
  const push = (message: unknown, line: unknown, file: unknown): void => {
    if (errors.length >= 20) return
    const n = Number(line)
    errors.push({ message: String(message || 'Error').slice(0, 400), line: n > 0 ? Math.floor(n) : 0, file: String(file || '').slice(0, 200) })
  }
  if (typeof win.addEventListener === 'function') {
    // capture：比畫布自己後來加的監聽先跑，畫布 stopImmediatePropagation 也攔不掉。capture 也會看到圖片等資源載入失敗
    // （target 是元素、不是腳本錯誤），只收 target 是 window 的。
    win.addEventListener(
      'error',
      (event: any) => {
        if (event && event.target && event.target !== win) return
        push(event && (event.message || (event.error && event.error.message)), event && event.lineno, event && event.filename)
      },
      { capture: true },
    )
    win.addEventListener(
      'unhandledrejection',
      (event: any) => {
        const reason = event && event.reason
        push(reason && reason.message ? reason.message : reason, 0, '')
      },
      { capture: true },
    )
  }
  lock(win, '__kunsuEarlyErrors', () => errors.slice())

  // 4. 畫布文件被換掉：送這次 token 的 error（token 從資料區塊讀；讀不到就不送，面板本來就不收沒有 token 的訊息）。
  //    資料區塊在加固腳本之前（QA 第 6 輪的文件順序）：安裝時就讀好 token、記下上層視窗——畫布之後刪掉資料區塊、用
  //    <frameset> 換掉 body 或改掉 window.parent 都不影響；安裝時讀不到才在 pagehide 再讀一次。
  const readPayload = (): any => {
    try {
      const block = win.document && win.document.getElementById('kunsu-canvas')
      const payload = JSON.parse((block && block.textContent) || '{}')
      return payload && typeof payload.token === 'string' && payload.token !== '' ? payload : null
    } catch {
      return null
    }
  }
  const installed = readPayload()
  const host = win.parent
  if (typeof win.addEventListener === 'function') {
    win.addEventListener(
      'pagehide',
      () => {
        try {
          const payload = installed || readPayload()
          if (!payload) return
          host.postMessage(
            {
              kunsu: payload.token,
              type: 'error',
              issues: [
                {
                  file: 'canvas.' + (payload.kind === 'html' || payload.kind === 'jsx' ? payload.kind : 'tsx'),
                  code: 'canvas_navigation',
                  message: '畫布換頁了',
                  hint: '畫布不能換頁、重新載入或開連結；同頁跳轉用 <a href="#元素id">（會在畫布裡捲動），資料直接畫在畫布上',
                },
              ],
            },
            '*',
          )
        } catch {
          // 送不出去就算了：殼層的子框架關卡仍擋住導到外面或 Kunsu 自己的頁面。
        }
      },
      { capture: true },
    )
  }
}
/* eslint-enable @typescript-eslint/no-explicit-any */

/** srcdoc 在資料區塊之後、runtime 與任何畫布內容之前放這段（與 runtime 用的是同一份程式碼）。 */
export const CANVAS_GUARD_SCRIPT = `(${installCanvasGuard.toString()})(window)`

/** 加固腳本收集到的錯誤（沒有加固腳本就是空的）。 */
export function earlyErrors(win: unknown): { message: string; line: number; file?: string }[] {
  const read = (win as { __kunsuEarlyErrors?: unknown } | null)?.__kunsuEarlyErrors
  if (typeof read !== 'function') return []
  try {
    const list = read()
    return Array.isArray(list) ? list.slice(0, 20) : []
  } catch {
    return []
  }
}

/** canvas_failed 診斷事件：同一個 key（題目＋版本＋這次顯示）只回報第一次。 */
export function failureOnce(report: (reason: string) => void): (key: string, reason: string) => void {
  const seen = new Set<string>()
  return (key, reason) => {
    if (seen.has(key)) return
    seen.add(key)
    report(reason)
  }
}
