// HTML 畫布組裝的差分測試（BDD evidence-canvas AC-EC-02、AC-EC-06；CONTRACT §12「沙盒文件的順序」「<link> 的最後一道」）。
// 組裝用的 tokenizer 是近似的；這裡拿符合規格的 parser（parse5，只在測試用）照瀏覽器的方式解析組裝結果，檢查不依賴
// tokenizer 切對的性質：
//  1. Kunsu 的資料區塊、加固腳本、runtime 都是 HTML 命名空間的 <script>，在 parser 開出畫布的 <div id="root"> 之前就已經
//     關閉（瀏覽器在這時執行）——畫布 HTML 在它們之後，怎麼寫都影響不了。
//  2. 整份文件（含 <template> 內容）沒有任何 HTML <link> 元素被建立過。
//  3. 畫布區段的換行數與原始碼相同（行號對照不變）。
// QA 第 6 輪用同樣的方法跑 40 萬筆，舊的組裝（資料區塊與 runtime 在畫布之後）失敗 258 筆；這裡固定跑 QA 找到的寫法加上
// 固定亂數種子的隨機組合。
import { describe, expect, it } from 'vitest'
import { defaultTreeAdapter, parse, type DefaultTreeAdapterMap } from 'parse5'
import { buildSrcdoc } from './srcdoc'

type Element = DefaultTreeAdapterMap['element']
type Node = DefaultTreeAdapterMap['node']

const HTML_NS = 'http://www.w3.org/1999/xhtml'
const RUNTIME = 'window.__RUNTIME__ = 1'
const ROOT = '<div id="root">'

const attr = (el: Element, name: string) => el.attrs.find((a) => a.name === name)?.value
const textOf = (el: Element) =>
  el.childNodes.map((c: Node) => (c.nodeName === '#text' ? (c as DefaultTreeAdapterMap['textNode']).value : '')).join('')
const breaks = (text: string) => (text.match(/\r\n|\r|\n/g) ?? []).length

/** 組裝並解析；回傳違反的性質（空陣列＝通過）。runtime 以 `__RUNTIME__` 字樣辨認。 */
function check(source: string, runtime = RUNTIME): string[] {
  const html = buildSrcdoc({ runtime, kind: 'html', source, assets: {}, token: 'tok-diff' })
  const json = html.match(/<script type="application\/json" id="kunsu-canvas">([^<]*)<\/script>/)?.[1]
  const closed: string[] = []
  const links: string[] = []
  let rootOpened = false
  const treeAdapter = {
    ...defaultTreeAdapter,
    createElement(tagName: string, namespaceURI: Parameters<typeof defaultTreeAdapter.createElement>[1], attrs: Element['attrs']) {
      if (namespaceURI === HTML_NS && tagName === 'link') links.push(JSON.stringify(attrs))
      return defaultTreeAdapter.createElement(tagName, namespaceURI, attrs)
    },
    onItemPush(el: Element) {
      if (!rootOpened && el.namespaceURI === HTML_NS && el.tagName === 'div' && attr(el, 'id') === 'root') rootOpened = true
    },
    onItemPop(el: Element) {
      if (rootOpened || el.namespaceURI !== HTML_NS || el.tagName !== 'script') return
      const text = textOf(el)
      if (attr(el, 'id') === 'kunsu-canvas' && attr(el, 'type') === 'application/json' && text === json) closed.push('data')
      else if (text.includes('__kunsuGuard')) closed.push('guard')
      else if (text.includes('__RUNTIME__')) closed.push('runtime')
    },
  }
  parse(html, { scriptingEnabled: true, treeAdapter })
  const problems: string[] = []
  if (closed.join(',') !== 'data,guard,runtime') problems.push(`Kunsu 的 script 在畫布之前關閉的順序：${closed.join(',') || '（沒有）'}`)
  if (!rootOpened) problems.push('沒有開出 <div id="root">')
  if (links.length > 0) problems.push(`建立了 HTML <link>：${links.join(' ')}`)
  const start = html.indexOf(ROOT)
  if (start < 0) return [...problems, '找不到畫布區段']
  // 畫布區段：<div id="root"> 之後到文件結尾（之後不能有 Kunsu 的東西）。
  const region = html.slice(start + ROOT.length)
  if (breaks(region) !== breaks(source)) problems.push(`換行數 ${breaks(source)} → ${breaks(region)}`)
  return problems
}

/** QA 第 6 輪的 8 種寫法、<link> 的兩種寫法，以及差分測試找到的其他寫法。 */
const QA_CASES = [
  '<p>t</p>\n<template></',
  '<p>x</p>\n</div><math></span>',
  '<p>x</p>\n<object><svg></b>',
  '<p>x</p>\n<table><svg></span>',
  '<p>x</p>\n<svg><desc><b></desc><plaintext>',
  '<p>x</p>\n<svg><foreignObject><div></foreignObject><plaintext>',
  '<p>x</p>\n<math><annotation-xml><svg><foreignObject><plaintext>',
  '<p>x</p>\n<select><STYLE/><script>',
  '<p>x</p>\n<svg></b><title><b><link rel="preconnect" href="//127.0.0.1:9/">\n',
  '<p>x</p>\n<svg></b><title><b><link rel="&#112;reconnect" href="//127.0.0.1:9/">\n',
  '<select><ruby><noscript></annotation-xml><textarea/><p>',
  '<h1><td><select><title/><script><meta http-equiv=refresh content=0>',
  '</object></svg><object><svg></select>',
  '<table></math><malignmark><math><meta http-equiv=refresh content=0><foreignObject>',
  '<listing><select><title><textarea/><li><textarea>',
  '<svg></form><title><script><!--<script></b></body>',
  '<select><foreignObject><b><noembed><head><textarea>',
  '</select><select></xmp></iframe><xmp><textarea/>',
  '<applet><svg></html>',
  '<template></iframe></math></mi></br></',
  '</div><svg></form><math><a href="#x">',
  '<marquee><svg><noscript></mi></title></span>',
  '<a href="#x"><math><annotation-xml><desc></table>',
  '"<template></body></',
  "<math></title>'<select><annotation-xml></b>",
  '"<select><sTyle><script><isindex>-->',
  '</div><listing></mi><math><meta http-equiv=refresh content=0>',
  '<table></script><ruby><isindex><math></div>',
  '<marquee><math></form><textarea>',
  '<br>]]></textarea><template></span></',
  '<select><xmp><textarea/>',
  '<select></object><table><svg><iframe><textarea/>',
  '<tr><frameset><select><noframes></object><textarea>',
  '<marquee><form></template><template><b></',
  '<select><noscript><script/></svg>',
  '</div>><math></select><html>',
  '<mtext><mi>]]><template></',
  '<template><caption></',
  '<font color=red><</title><nobr><template></',
  '<math></select><frameset><STYLE/><meta http-equiv=refresh content=0>',
  // 拿掉標籤後前後拼接、外來內容裡的 module script、template 內容裡的 <link>。
  '<<link>link rel=preconnect href=//x.test>',
  '<<meta http-equiv=x>link rel=preconnect href=//x.test>',
  '<svg></b><script type=module><b><link rel=preconnect href=//x.test>',
  '<template><link rel=preconnect href=//x.test></template>',
  '<svg><style><b><LINK\nrel=preconnect href=//x.test>',
]

const PIECES = [
  '<svg>', '</svg>', '<math>', '</math>', '<mi>', '</mi>', '<mtext>', '<annotation-xml>', '<annotation-xml encoding="text/html">',
  '</annotation-xml>', '<foreignObject>', '</foreignObject>', '<desc>', '</desc>', '<title>', '</title>', '<div>', '</div>', '<b>',
  '</b>', '<span>', '</span>', '<p>', '</p>', '<br>', '</br>', '<table>', '</table>', '<tr>', '<td>', '<select>', '</select>',
  '<option>', '<template>', '</template>', '<style>', '</style>', '<script>', '</script>', '<!--', '-->', '<![CDATA[', ']]>',
  '<textarea>', '</textarea>', '<xmp>', '</xmp>', '<noscript>', '</noscript>', '<iframe>', '</iframe>', '<noembed>', '<noframes>',
  '<plaintext>', '<link rel=preconnect href=//x.test>', '<LINK\nrel=dns-prefetch href=//y.test>', '<link', '<meta http-equiv=refresh content=0>',
  '>', '"', "'", '<a title="', '<font color=red>', '<frameset>', '<head>', '<body>', '<html>', '</body>', '</html>', '<object>',
  '</object>', '<h1>', '<li>', '<!DOCTYPE x>', '<?x', '</', '<', '<!', 'x', '\n', '\r\n', '<svg/>', '<g>', '</g>', '<mglyph>',
  '<malignmark>', '<frame>', '<image>', '<isindex>', '<form>', '</form>', '<button>', '<ruby>', '<rt>', '<colgroup>', '<caption>',
  '<script><!--<script>', '<SCRIPT>', '</SCRIPT >', '<sTyle>', '<STYLE/>', '<script/>', '<textarea/>', '<title/>', '<svg><title>',
  '<a href="#x">', '<input>', '<keygen>', '<hr>', '<marquee>', '<applet>', '<nobr>', '<listing>', '<pre>', '<script type=module>',
  '<script type=text/plain>', 'link rel=preconnect href=//z.test>', '&lt;', '&#60;link', '\r',
]

/** 固定種子的線性同餘亂數（每次跑的組合都一樣，失敗能重現）。 */
function rng(seed: number) {
  let s = seed >>> 0
  return () => {
    s = (Math.imul(s, 1664525) + 1013904223) >>> 0
    return s / 2 ** 32
  }
}

describe('HTML 畫布組裝的差分測試（parse5）：畫布 HTML 怎麼寫都吞不掉 Kunsu 的東西、開不出 <link>', () => {
  it.each(QA_CASES)('%j', (source) => {
    expect(check(source)).toEqual([])
  })

  // runtime 是 bundle：裡面若有 `<!--` 再接 `<script`，script data 會進入 double escaped，Kunsu 自己的 </script> 結束不了
  // 腳本、一路吃進畫布（目前的 bundle 沒有 `<!--`；組裝把它改寫成等價的 `\x3C!--` 當保險）。
  it('runtime 裡有 "<!--" 接 "<script"（double escaped）也在畫布之前結束', () => {
    const runtime = 'window.__RUNTIME__ = 1; var a = "<!--"; var b = "<script>"; var c = "</script>"'
    expect(check('<p id="canvas">x</p>', runtime)).toEqual([])
  })

  it('固定種子的隨機組合 4000 筆', () => {
    const random = rng(6)
    const failures: string[] = []
    for (let k = 0; k < 4000 && failures.length < 5; k++) {
      const count = 1 + Math.floor(random() * 6)
      let source = ''
      for (let j = 0; j < count; j++) source += PIECES[Math.floor(random() * PIECES.length)]
      const problems = check(`<p>x</p>\n${source}`)
      if (problems.length > 0) failures.push(`${JSON.stringify(source)}：${problems.join('；')}`)
    }
    expect(failures).toEqual([])
  })
})
