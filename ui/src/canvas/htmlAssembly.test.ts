// HTML 畫布組裝（BDD evidence-canvas 場景 2、6；AC-EC-02、06）：組裝走 HTML tokenizer，只處理真正的標籤。
// `<script>`／`<style>` 等 raw text、註解、屬性值與 CDATA 裡的字樣不是標籤；資料區塊與 runtime 在畫布之前（QA 第 6 輪），
// 畫布 HTML 是文件的最後一段；字面的 `<link` 一律改成 `&lt;link`（不依賴 tokenizer 切對）。
import { describe, expect, it } from 'vitest'
import { buildSrcdoc } from './srcdoc'

function assemble(source: string, assets: Record<string, string> = {}): string {
  return buildSrcdoc({ runtime: 'window.__RUNTIME__ = 1', kind: 'html', source, assets, token: 'tok-a' })
}

/** 畫布區段：`<div id="root">` 之後到文件結尾（畫布 HTML 是最後一段，之後沒有 Kunsu 的東西）。 */
function region(html: string): string {
  const start = html.indexOf('<div id="root">')
  if (start < 0) throw new Error('沒有 <div id="root">')
  if (html.indexOf('<script type="application/json" id="kunsu-canvas">') > start) throw new Error('資料區塊在畫布之後')
  return html.slice(start + '<div id="root">'.length)
}

/** 字面 `<link` 中和後的文字還原（屬性值、title、textarea 裡解碼後是同一段文字）。 */
const decodedLinks = (text: string) => text.replace(/&lt;(link)/gi, '<$1')
/** 開得出 link 元素的字面寫法：`<link` 後面接空白、`/`、`>` 或結尾。 */
const LIVE_LINK = /<link(?=[\t\n\f\r />]|$)/i

/** 區段裡每個 data: 腳本解碼後的程式碼。 */
function dataScripts(part: string): string[] {
  return [...part.matchAll(/<script\b[^>]*\bsrc="data:text\/javascript;[^,"]*,([^"]*)"/g)].map((m) => decodeURIComponent(m[1]))
}

function lineOffset(html: string): number {
  const m = html.match(/<script type="application\/json" id="kunsu-canvas">([^<]*)<\/script>/)
  if (!m) throw new Error('沒有資料區塊')
  return JSON.parse(m[1]).line_offset
}

describe('HTML 畫布組裝：raw text、註解、屬性值裡的字樣不是標籤', () => {
  it('<style> 內容、HTML 註解、屬性值、<textarea> 裡的 <link …> 不是標籤：不拿掉，只把 <link 改成 &lt;link', () => {
    const cases = [
      '<style>/* <link rel="preconnect" href="//x"> */ p { color: red }</style>',
      '<!-- <link rel="preconnect" href="//x"> -->',
      '<div title="<link rel=preconnect href=//x>">t</div>',
      '<textarea><link rel="preconnect" href="//x"></textarea>',
    ]
    for (const source of cases) {
      const part = region(assemble(source))
      expect(part, source).not.toMatch(LIVE_LINK)
      expect(decodedLinks(part), source).toBe(source)
    }
    // 屬性值、textarea 解碼後的文字和原本一樣。
    expect(region(assemble('<div title="<link rel=preconnect>">t</div>'))).toBe('<div title="&lt;link rel=preconnect>">t</div>')
    const title = '<title><meta http-equiv="refresh" content="0"></title>'
    expect(region(assemble(title))).toBe(title)
  })

  it('真正的 <link> 與 <meta http-equiv> 照樣拿掉、保留換行；其他 meta 不動', () => {
    const source = '<p>a</p>\n<link\n rel="preconnect" href="//x">\n<meta charset="utf-8"><META HTTP-EQUIV="refresh" content="0">\n<p>b</p>'
    const html = assemble(source)
    expect(region(html)).toBe('<p>a</p>\n\n\n<meta charset="utf-8">\n<p>b</p>')
    expect(html.split('\n')[lineOffset(html) + 4]).toContain('<p>b</p>')
  })

  it('script data 的 escaped／double escaped：<!--<script> 裡的 </script> 不結束腳本，整段照瀏覽器的切法轉成一個 data: 腳本', () => {
    const body = '\nvar a = "<!--<script>"\nvar b = "</script>"\nvar c = "-->"\nreport(a, b, c)\n'
    const html = assemble(`<script>${body}</script>\n<p id="after">x</p>`)
    const part = region(html)
    expect(dataScripts(part)).toEqual([body])
    expect(part).toContain('<p id="after">x</p>')
  })

  it('腳本結束標籤可帶空白、換行與屬性（屬性值可含 >），換行保留、行號不變', () => {
    const html = assemble('<script>\nboom()\n</script\n foo=">">\n<p id="four">4</p>')
    const part = region(html)
    expect(dataScripts(part)).toEqual(['\nboom()\n'])
    expect(html.split('\n')[lineOffset(html) + 4]).toContain('<p id="four">4</p>')
  })

  // 差分測試（400k 筆）找到：parser 把單獨的 CR 當換行，組裝只數 \n——拿掉的標籤、data: 腳本的行數與 kunsu-line 會少算。
  it('單獨的 CR 也算一行：拿掉的標籤與轉成 data: 的腳本保留同樣的行數，kunsu-line 照 parser 的行號算', () => {
    const src = '<p>1</p>\r<link\rrel=preconnect>\r<script>\rboom()\r</script>\r<p id="seven">7</p>'
    const part = region(assemble(src))
    const lines = (text: string) => text.split(/\r\n|\r|\n/).length
    expect(lines(part)).toBe(lines(src))
    expect(part).toMatch(/kunsu-line=4;/)
    const html = assemble(src)
    expect(html.split(/\r\n|\r|\n/)[lineOffset(html) + 6]).toContain('<p id="seven">7</p>')
  })

  it('async／defer 對 inline script 無效：轉成 data: 腳本時拿掉，維持原本的執行順序；跨行的開始標籤行號照算', () => {
    const html = assemble('<p>1</p>\n<script\n defer async>\nfirst()\n</script>\n<p id="six">6</p>')
    const part = region(html)
    const tag = part.match(/<script\b[^>]*>/)?.[0] ?? ''
    expect(tag).not.toMatch(/\b(?:defer|async)\b/)
    expect(tag).toContain('kunsu-line=3;')
    expect(dataScripts(part)).toEqual(['\nfirst()\n'])
    expect(html.split('\n')[lineOffset(html) + 5]).toContain('<p id="six">6</p>')
  })

  it('有 src、type=module、非 JavaScript 型別、language=vbscript 的 script 不轉換（內容裡的字面 <link 一樣中和）', () => {
    for (const source of [
      '<script src="data:text/javascript,1"></script>',
      '<script type="module">m()</script>',
      '<script language="vbscript">x</script>',
    ]) {
      expect(region(assemble(source)), source).toBe(source)
    }
    expect(region(assemble('<script type="text/plain"><link rel=preconnect></script>'))).toBe(
      '<script type="text/plain">&lt;link rel=preconnect></script>',
    )
  })

  it('<svg> 裡的 script 是 SVG 元素：不轉 data:；<foreignObject> 裡回到 HTML，script 照轉', () => {
    const svg = '<svg><script>draw()</script><rect/></svg>'
    expect(region(assemble(svg))).toBe(svg)
    const fo = region(assemble('<svg><foreignObject><script>html()</script></foreignObject></svg>'))
    expect(dataScripts(fo)).toEqual(['html()'])
    // SVG 的 <style> 不是 raw text：裡面的標籤是真的標籤（<link> 一樣拿掉）。CDATA 是文字。
    expect(region(assemble('<svg><style><link rel="preconnect" href="//x"></style></svg>'))).toBe('<svg><style></style></svg>')
    const cdata = '<svg><![CDATA[<link rel="preconnect" href="//x">]]></svg>'
    expect(region(assemble(cdata))).toBe('<svg><![CDATA[&lt;link rel="preconnect" href="//x">]]></svg>')
  })
})

describe('HTML 畫布組裝：畫布 HTML 是文件最後一段，沒關的結構交給 parser 在結尾收（組裝不補結尾）', () => {
  it('沒關的註解、module script、<style>、<textarea>、<template>、<svg> 原樣留著', () => {
    for (const source of [
      '<p>a</p>\n<!-- 待辦',
      '<script type="module">m()',
      '<script type="module"><!--<script>',
      '<style>p{}',
      '<textarea>t',
      '<template><p>t</p>',
      '<svg><g><rect/>',
      '<p>t</p>\n<template></',
      '<p>x</p>\n<select><STYLE/><script>',
    ]) {
      expect(region(assemble(source)), source).toBe(source)
    }
  })

  it('沒關的傳統 <script> 照樣轉成 data: 腳本並自己結束', () => {
    const part = region(assemble('<p>a</p>\n<script>\nlast()\n'))
    expect(dataScripts(part)).toEqual(['\nlast()\n'])
    expect((part.match(/<script\b/g) ?? []).length).toBe((part.match(/<\/script>/g) ?? []).length)
  })

  it('沒寫完的標籤拿掉（屬性值沒關引號也一樣），保留換行', () => {
    expect(region(assemble('<p>a</p>\n<div title="x\ny'))).toBe('<p>a</p>\n\n')
  })

  it('<plaintext> 改成 <pre>＋跳脫後的內容（plaintext 之後的一切都會變成文字）', () => {
    expect(region(assemble('<plaintext>a < b & </div>'))).toBe('<pre>a &lt; b &amp; &lt;/div></pre>')
  })
})

describe('HTML 畫布組裝：<link> 的最後一道（不依賴 tokenizer）', () => {
  it('tokenizer 切錯（<svg></b><title> 被當成 HTML 的 RCDATA）時，字面的 <link 一樣中和', () => {
    const part = region(assemble('<p>x</p>\n<svg></b><title><b><link rel="preconnect" href="//127.0.0.1:9/">\n'))
    expect(part).not.toMatch(LIVE_LINK)
    expect(part).toContain('&lt;link rel="preconnect"')
  })

  it('拿掉的標籤前後拼接不出 <link（<<link>link …>、<<meta http-equiv>link …>）', () => {
    for (const source of ['<<link>link rel=preconnect href=//x.test>', '<<meta http-equiv=x>link rel=preconnect href=//x.test>']) {
      expect(region(assemble(source)), source).not.toMatch(LIVE_LINK)
    }
  })

  it('每一種分隔字元與結尾都算；<link-card>、<linkx> 這類別的元素不動', () => {
    for (const source of ['<LINK\trel=preconnect>', '<link/rel=preconnect>', '<link>', '<Link\frel=x>', '<link\rrel=x>', '<p>x</p><link']) {
      expect(region(assemble(`<svg></b><title><b>${source}`)), source).not.toMatch(LIVE_LINK)
    }
    expect(region(assemble('<link-card>a</link-card><linkx>b</linkx>'))).toBe('<link-card>a</link-card><linkx>b</linkx>')
  })

  it('傳統 JavaScript 腳本的內容不受影響（已轉成 data: 腳本，URL 編碼）', () => {
    const part = region(assemble('<script>\nconst t = "<link rel=preconnect>"\n</script>'))
    expect(dataScripts(part)).toEqual(['\nconst t = "<link rel=preconnect>"\n'])
  })
})

describe('HTML 畫布組裝：資產', () => {
  it('./assets/ 的引用照樣換成快照 data URI（含腳本裡的字串）', () => {
    const part = region(assemble('<img src="./assets/a.png">\n<script>\nconst u = "./assets/a.png"\n</script>', { './assets/a.png': 'data:image/png;base64,AAAA' }))
    expect(part).toContain('<img src="data:image/png;base64,AAAA">')
    expect(dataScripts(part)).toEqual(['\nconst u = "data:image/png;base64,AAAA"\n'])
  })
})
