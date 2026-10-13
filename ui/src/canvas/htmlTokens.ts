// HTML 畫布組裝用的 tokenizer（BDD evidence-canvas 場景 2、6；AC-EC-02、06；CONTRACT §12「組裝 HTML 畫布」）。
//
// 組裝沙盒文件時要拿掉 `<link>`／`<meta http-equiv>`、把傳統 inline script 轉成 data: 腳本——只想動「真正的標籤」。
// 單一正規表示式分不出 `<script>` 內容、註解、屬性值裡的 `<link` 字樣（QA 第 5 輪：腳本註解寫到 `<link` 就從那裡一路
// 刪到 `</script>` 的 `>`），所以這裡照 HTML 規格的 tokenizer 狀態切 token，回傳每一段的位置：data／RCDATA／RAWTEXT／
// script data（含 escaped 與 double escaped）／PLAINTEXT、註解、`<!…>`／`<?…>`、屬性值引號。
//
// **這不是完整的 HTML parser。** 哪些標籤會切換 tokenizer 狀態，取決於樹建構（插入模式與開著的元素）；這裡只近似追蹤
// `<svg>`／`<math>` 外來內容（integration point、CDATA、跳回 HTML 的標籤）與 `<template>`，不追蹤一般 HTML 元素的堆疊與
// `<select>`／`<table>`／`<frameset>` 等插入模式。少見的寫法會和瀏覽器切得不一樣（QA 第 6 輪的 parse5 差分測試 40 萬筆找出
// 258 筆：`<svg>` 裡對不上的 `</b>`、integration point 裡沒關的 HTML 元素、`<select>` 裡的 `<style/>`、`annotation-xml` 裡的
// `<svg>`……）。所以組裝只把這裡的切法用在切錯也無害的地方：
// - Kunsu 的資料區塊、加固腳本與 runtime 在畫布 HTML 之前，畫布 HTML 是文件最後一段——切錯吞不掉它們；
// - 字面的 `<link` 由組裝的最後一道一律中和（srcdoc.ts `neutralizeLinks`）——切錯也開不出 `<link>`；
// - `<meta http-equiv>` 由 server 的靜態檢查保證——這裡拿掉只是縱深防禦；
// - 切錯的最壞結果是某個 `<script>` 沒轉成 data: 腳本（錯誤只剩 `Script error.`、沒有行號）或某段被誤轉。
// 引擎設定才是網路的保證（INV-20）。

export interface HtmlAttr {
  /** 小寫屬性名稱。 */
  name: string
  value: string
  /** 屬性在原始碼的範圍（名稱開頭到值結尾，不含前面的空白）。 */
  from: number
  to: number
}

export interface HtmlTag {
  end: boolean
  /** 小寫標籤名稱。 */
  name: string
  /** 整個標籤的範圍（`<` 到 `>` 之後）；沒寫完的標籤到原始碼結尾。 */
  from: number
  to: number
  /** 標籤名稱結束的位置（屬性從這裡開始）。 */
  nameEnd: number
  attrs: HtmlAttr[]
  selfClosing: boolean
  /** false＝到原始碼結尾都沒有 `>`（瀏覽器會整個丟掉這個標籤）。 */
  complete: boolean
  /** 這個標籤在 HTML 內容裡（不是 `<svg>`／`<math>` 的外來內容）。 */
  html: boolean
}

export type HtmlSegment =
  | { kind: 'text'; from: number; to: number }
  /** 註解、`<!…>`、`<?…>`、`</ …>` 等 bogus comment、外來內容的 CDATA（原始碼結束時還沒關的，到結尾為止）。 */
  | { kind: 'comment'; from: number; to: number }
  | { kind: 'tag'; tag: HtmlTag }
  /** raw text 元素（script、style、textarea…）：開始標籤、內容範圍、結束標籤（沒有＝到結尾都沒關）。 */
  | { kind: 'raw'; start: HtmlTag; contentFrom: number; contentTo: number; end: HtmlTag | null }
  /** `<plaintext>`：之後的一切都是文字。 */
  | { kind: 'plaintext'; start: HtmlTag; contentFrom: number }

const RAWTEXT = new Set(['style', 'xmp', 'iframe', 'noembed', 'noframes', 'noscript'])
const RCDATA = new Set(['textarea', 'title'])
/** 外來內容裡遇到這些開始標籤會跳回 HTML（規格 13.2.6.5）。 */
const BREAKOUT = new Set(
  'b big blockquote body br center code dd div dl dt em embed h1 h2 h3 h4 h5 h6 head hr i img li listing menu meta nobr ol p pre ruby s small span strong strike sub sup table tt u ul var'.split(
    ' ',
  ),
)
/** 外來內容裡對不上的結束標籤若是這些 HTML 元素，通常是在關外面的 HTML 元素（瀏覽器會一路關掉 svg）。 */
const HTML_END = new Set(
  [
    ...BREAKOUT,
    'a abbr address article aside button caption col colgroup data details dialog dfn fieldset figcaption figure footer form header hgroup html label legend main mark nav option optgroup output picture q rp rt samp search section select summary tbody td tfoot th thead time tr wbr',
  ]
    .join(' ')
    .split(' '),
)

const isWs = (c: string | undefined): boolean => c === ' ' || c === '\n' || c === '\t' || c === '\f' || c === '\r'
const isAlpha = (c: string | undefined): boolean => c !== undefined && ((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'))
/** tag name 結束的字元：空白、`/`、`>`。 */
const isDelim = (c: string | undefined): boolean => isWs(c) || c === '/' || c === '>'

/** `src[at…]` 是否（ASCII 不分大小寫）等於 `word`。不用 toLowerCase 整份轉換：有些 Unicode 字元轉小寫會改變長度。 */
function asciiIs(src: string, at: number, word: string): boolean {
  if (at + word.length > src.length) return false
  for (let k = 0; k < word.length; k++) {
    let c = src.charCodeAt(at + k)
    if (c >= 65 && c <= 90) c += 32
    if (c !== word.charCodeAt(k)) return false
  }
  return true
}

/** 從 `lt`（`<` 或 `</` 的位置）讀一個標籤，含屬性（值可含 `>`）。 */
function readTag(src: string, lt: number, end: boolean, html: boolean): HtmlTag {
  const n = src.length
  let p = lt + (end ? 2 : 1)
  const nameFrom = p
  while (p < n && !isDelim(src[p])) p++
  const tag: HtmlTag = {
    end,
    name: src.slice(nameFrom, p).toLowerCase(),
    from: lt,
    to: n,
    nameEnd: p,
    attrs: [],
    selfClosing: false,
    complete: false,
    html,
  }
  for (;;) {
    while (p < n && isWs(src[p])) p++
    if (p >= n) return tag
    const c = src[p]
    if (c === '>') {
      tag.to = p + 1
      tag.complete = true
      return tag
    }
    if (c === '/') {
      if (src[p + 1] === '>') {
        tag.selfClosing = true
        tag.to = p + 2
        tag.complete = true
        return tag
      }
      p++
      continue
    }
    // 屬性名稱（第一個字可以是 `=`）。
    const from = p
    p++
    while (p < n && !isWs(src[p]) && src[p] !== '/' && src[p] !== '>' && src[p] !== '=') p++
    const name = src.slice(from, p).toLowerCase()
    let q = p
    while (q < n && isWs(src[q])) q++
    let value = ''
    if (src[q] === '=') {
      p = q + 1
      while (p < n && isWs(src[p])) p++
      if (p >= n) return tag
      const quote = src[p]
      if (quote === '"' || quote === "'") {
        const close = src.indexOf(quote, p + 1)
        if (close < 0) return tag
        value = src.slice(p + 1, close)
        p = close + 1
      } else if (quote !== '>') {
        const start = p
        while (p < n && !isWs(src[p]) && src[p] !== '>') p++
        value = src.slice(start, p)
      }
    }
    tag.attrs.push({ name, value, from, to: p })
  }
}

type ScriptState = 'data' | 'escaped' | 'escapedDash' | 'escapedDashDash' | 'double' | 'doubleDash' | 'doubleDashDash'

/**
 * script data（規格 13.2.5.4 起）：找出真正結束 `<script>` 的 `</script`。`<!--` 之後再出現 `<script` 會進入 double
 * escaped，這時的 `</script>` 只回到 escaped、不結束腳本。回傳結束標籤的位置（-1＝到結尾都沒關）。
 */
function scriptEnd(src: string, from: number): number {
  const n = src.length
  let state: ScriptState = 'data'
  let p = from
  /** `<` 之後的 `script` + 分隔字元。 */
  const scriptWord = (at: number) => asciiIs(src, at, 'script') && isDelim(src[at + 6])
  while (p < n) {
    const c = src[p]
    switch (state) {
      case 'data':
        if (c === '<') {
          if (src[p + 1] === '/' && scriptWord(p + 2)) return p
          if (src.startsWith('<!--', p)) {
            state = 'escapedDashDash'
            p += 4
            continue
          }
        }
        break
      case 'escaped':
      case 'escapedDash':
      case 'escapedDashDash':
        if (c === '-') {
          state = state === 'escaped' ? 'escapedDash' : 'escapedDashDash'
        } else if (c === '<') {
          if (src[p + 1] === '/' && scriptWord(p + 2)) return p
          if (scriptWord(p + 1)) {
            state = 'double'
            p += 7
            continue
          }
          state = 'escaped'
        } else if (c === '>' && state === 'escapedDashDash') {
          state = 'data'
        } else {
          state = 'escaped'
        }
        break
      default:
        // double escaped
        if (c === '-') {
          state = state === 'double' ? 'doubleDash' : 'doubleDashDash'
        } else if (c === '<') {
          if (src[p + 1] === '/' && scriptWord(p + 2)) {
            state = 'escaped'
            p += 8
            continue
          }
          state = 'double'
        } else if (c === '>' && state === 'doubleDashDash') {
          state = 'data'
        } else {
          state = 'double'
        }
    }
    p++
  }
  return -1
}

/** RAWTEXT／RCDATA：`</元素名稱` + 分隔字元的位置（-1＝沒有）。 */
function rawEnd(src: string, from: number, element: string): number {
  let p = from
  for (;;) {
    const at = src.indexOf('</', p)
    if (at < 0) return -1
    if (asciiIs(src, at + 2, element) && isDelim(src[at + 2 + element.length])) return at
    p = at + 2
  }
}

interface Context {
  name: string
  ns: 'html' | 'svg' | 'math'
  /** HTML integration point（svg foreignObject／desc／title、MathML mi／mo／mn／ms／mtext、annotation-xml〔HTML 編碼〕）。 */
  integration: boolean
}

function integrationPoint(ns: 'svg' | 'math', name: string, attrs: HtmlAttr[]): boolean {
  if (ns === 'svg') return name === 'foreignobject' || name === 'desc' || name === 'title'
  if (['mi', 'mo', 'mn', 'ms', 'mtext'].includes(name)) return true
  if (name !== 'annotation-xml') return false
  const encoding = attrs.find((a) => a.name === 'encoding')?.value.toLowerCase()
  return encoding === 'text/html' || encoding === 'application/xhtml+xml'
}

/** 走過整份 HTML，回傳每一段（沒關的結構不補結尾：畫布 HTML 是沙盒文件的最後一段，由 parser 在結尾收）。 */
export function scanHtml(src: string): HtmlSegment[] {
  const n = src.length
  const segments: HtmlSegment[] = []
  const stack: Context[] = []
  const top = (): Context | undefined => stack[stack.length - 1]
  const htmlContext = (): boolean => {
    const t = top()
    return !t || t.ns === 'html' || t.integration
  }
  /** 從外來內容跳回 HTML：一路關到 integration point、`<template>` 或最外層。 */
  const breakOut = () => {
    while (stack.length > 0 && !htmlContext()) stack.pop()
  }
  const text = (from: number, to: number) => {
    if (to > from) segments.push({ kind: 'text', from, to })
  }

  let i = 0
  while (i < n) {
    const lt = src.indexOf('<', i)
    if (lt < 0) {
      text(i, n)
      break
    }
    text(i, lt)
    const next = src[lt + 1]

    // 註解：`<!-->`、`<!--->` 直接結束；否則到 `-->` 或 `--!>`。
    if (src.startsWith('<!--', lt)) {
      const body = lt + 4
      let to = -1
      if (src[body] === '>') to = body + 1
      else if (src.startsWith('->', body)) to = body + 2
      else {
        const a = src.indexOf('-->', body)
        const b = src.indexOf('--!>', body)
        if (a >= 0 && (b < 0 || a <= b)) to = a + 3
        else if (b >= 0) to = b + 4
      }
      segments.push({ kind: 'comment', from: lt, to: to < 0 ? n : to })
      i = to < 0 ? n : to
      continue
    }
    // 外來內容的 CDATA；其他 `<!…>`、`<?…>` 是 bogus comment，到 `>` 為止。
    if (next === '!' || next === '?') {
      if (next === '!' && !htmlContext() && src.startsWith('<![CDATA[', lt)) {
        const close = src.indexOf(']]>', lt + 9)
        const to = close < 0 ? n : close + 3
        segments.push({ kind: 'comment', from: lt, to })
        i = to
        continue
      }
      const close = src.indexOf('>', lt + 2)
      const to = close < 0 ? n : close + 1
      segments.push({ kind: 'comment', from: lt, to })
      i = to
      continue
    }

    if (next === '/') {
      const c = src[lt + 2]
      if (c === undefined) {
        // `</` 在結尾：瀏覽器當文字。
        text(lt, n)
        break
      }
      if (c === '>') {
        // `</>`：瀏覽器直接忽略。
        segments.push({ kind: 'comment', from: lt, to: lt + 3 })
        i = lt + 3
        continue
      }
      if (!isAlpha(c)) {
        const close = src.indexOf('>', lt + 2)
        const to = close < 0 ? n : close + 1
        segments.push({ kind: 'comment', from: lt, to })
        i = to
        continue
      }
      const tag = readTag(src, lt, true, htmlContext())
      segments.push({ kind: 'tag', tag })
      i = tag.to
      if (!tag.complete) break
      if (htmlContext()) {
        const t = top()
        if (tag.name === 'template') {
          const at = stack.map((s) => s.ns === 'html' && s.name === 'template').lastIndexOf(true)
          if (at >= 0) stack.length = at
        } else if (t?.integration && t.name === tag.name) {
          stack.pop()
        }
      } else if (tag.name === 'br' || tag.name === 'p') {
        breakOut()
      } else {
        // 對得上的外來元素就關到它；對不上的 HTML 元素名稱＝在關外面的元素，跳回 HTML。
        let at = -1
        for (let k = stack.length - 1; k >= 0 && stack[k].ns !== 'html'; k--) {
          if (stack[k].name === tag.name) {
            at = k
            break
          }
          if (stack[k].integration) break
        }
        if (at >= 0) stack.length = at
        else if (HTML_END.has(tag.name)) breakOut()
      }
      continue
    }

    if (!isAlpha(next)) {
      // `<` 後面不是標籤開頭：文字。
      text(lt, lt + 1)
      i = lt + 1
      continue
    }

    // 開始標籤。外來內容裡遇到 HTML 專用的標籤先跳回 HTML 再處理。
    let tag = readTag(src, lt, false, htmlContext())
    if (!htmlContext()) {
      const font = tag.name === 'font' && tag.attrs.some((a) => a.name === 'color' || a.name === 'face' || a.name === 'size')
      if (BREAKOUT.has(tag.name) || font) {
        breakOut()
        tag = { ...tag, html: true }
      }
    }
    i = tag.to
    if (!tag.complete) {
      segments.push({ kind: 'tag', tag })
      break
    }
    if (!tag.html) {
      const t = top()!
      if (!tag.selfClosing) {
        const ns = t.ns === 'html' ? 'svg' : t.ns
        stack.push({ name: tag.name, ns, integration: integrationPoint(ns, tag.name, tag.attrs) })
      }
      segments.push({ kind: 'tag', tag })
      continue
    }
    if (tag.name === 'svg' || tag.name === 'math') {
      if (!tag.selfClosing) stack.push({ name: tag.name, ns: tag.name, integration: false })
      segments.push({ kind: 'tag', tag })
      continue
    }
    if (tag.name === 'template') {
      stack.push({ name: 'template', ns: 'html', integration: false })
      segments.push({ kind: 'tag', tag })
      continue
    }
    if (tag.name === 'plaintext') {
      segments.push({ kind: 'plaintext', start: tag, contentFrom: tag.to })
      i = n
      break
    }
    if (tag.name === 'script' || RAWTEXT.has(tag.name) || RCDATA.has(tag.name)) {
      const at = tag.name === 'script' ? scriptEnd(src, tag.to) : rawEnd(src, tag.to, tag.name)
      if (at < 0) {
        segments.push({ kind: 'raw', start: tag, contentFrom: tag.to, contentTo: n, end: null })
        i = n
        break
      }
      const end = readTag(src, at, true, true)
      segments.push({ kind: 'raw', start: tag, contentFrom: tag.to, contentTo: at, end: end.complete ? end : null })
      if (!end.complete) {
        // 結束標籤沒寫完：瀏覽器丟掉它，raw text 到結尾為止。
        segments.push({ kind: 'tag', tag: end })
        break
      }
      i = end.to
      continue
    }
    segments.push({ kind: 'tag', tag })
  }
  return segments
}
