import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import { RichContent } from './RichContent'

describe('RichContent', () => {
  it('renders markdown and safe html', () => {
    const html = renderToStaticMarkup(
      <RichContent>{'## 脈絡\n\n<mark>重要</mark>\n\n- 一\n- 二'}</RichContent>,
    )
    expect(html).toContain('<h2>脈絡</h2>')
    expect(html).toContain('<mark>重要</mark>')
    expect(html).toContain('<li>一</li>')
  })

  it('renders compact Markdown without a paragraph wrapper for inline decision copy', () => {
    const html = renderToStaticMarkup(<RichContent inline>**直接送出**</RichContent>)
    expect(html).toContain('<strong>直接送出</strong>')
    expect(html).not.toContain('<p>')
  })

  it('removes executable and layout-breaking html', () => {
    const html = renderToStaticMarkup(
      <RichContent>
        {'<script>alert(1)</script><form><input value="steal" /></form><p style="position:fixed" onclick="bad()">保留文字</p>'}
      </RichContent>,
    )
    expect(html).not.toContain('script')
    expect(html).not.toContain('form')
    expect(html).not.toContain('input')
    expect(html).not.toContain('position:fixed')
    expect(html).not.toContain('onclick')
    expect(html).toContain('保留文字')
  })

  it('草稿題（有資產快照）只顯示快照圖片，外部圖片一律不載入', () => {
    const html = renderToStaticMarkup(
      <RichContent assets={{ './assets/a.png': 'data:image/png;base64,AAAA' }}>
        {'![快照](./assets/a.png)\n\n![追蹤](https://tracker.test/p.png)\n\n<img src="https://tracker.test/q.png" alt="q">'}
      </RichContent>,
    )
    expect(html).toContain('src="data:image/png;base64,AAAA"')
    expect(html).not.toContain('tracker.test')
  })

  it('連結按第一次只提示，第二次（4 秒內）才用瀏覽器開', async () => {
    const { linkClick } = await import('./RichContent')
    expect(linkClick(null, 'https://a.test', 0)).toEqual({ action: 'confirm', pending: { href: 'https://a.test', at: 0 } })
    expect(linkClick({ href: 'https://a.test', at: 0 }, 'https://a.test', 3000).action).toBe('open')
    expect(linkClick({ href: 'https://a.test', at: 0 }, 'https://a.test', 5000).action).toBe('confirm')
    expect(linkClick({ href: 'https://a.test', at: 0 }, 'https://b.test', 1000).action).toBe('confirm')
    expect(linkClick(null, 'javascript:alert(1)', 0).action).toBe('ignore')
  })
})

// QA 第 2 輪（AC-EC-08）：草稿題的任何 Markdown 都只顯示快照；picture／source／srcset 一律拿掉。
describe('只顯示快照', () => {
  const snap = { './assets/b.png': 'data:image/png;base64,AAAA' }

  it('picture／source／srcset 不被允許', () => {
    const html = renderToStaticMarkup(
      <RichContent>{'<picture><source srcset="https://tracker.test/s.png"><img src="./x.png" srcset="https://tracker.test/2x.png 2x" alt="x"></picture>'}</RichContent>,
    )
    expect(html).not.toContain('tracker.test')
    expect(html).not.toMatch(/<picture|<source/)
  })

  it('DraftAssetsContext 讓 inline 內容也只顯示快照', async () => {
    const { DraftAssetsContext } = await import('./RichContent')
    const html = renderToStaticMarkup(
      <DraftAssetsContext.Provider value={snap}>
        <RichContent inline>{'證據 ![t](https://tracker.test/e.png) ![b](./assets/b.png)'}</RichContent>
      </DraftAssetsContext.Provider>,
    )
    expect(html).not.toContain('tracker.test')
    expect(html).toContain('data:image/png;base64,AAAA')
  })

  it('快照還沒載入（空物件）時也不放行外部圖片', async () => {
    const { DraftAssetsContext } = await import('./RichContent')
    const html = renderToStaticMarkup(
      <DraftAssetsContext.Provider value={{}}>
        <RichContent inline>{'![追蹤](https://tracker.test/e.png)'}</RichContent>
      </DraftAssetsContext.Provider>,
    )
    expect(html).not.toContain('tracker.test')
    expect(html).toContain('［圖片：追蹤］')
  })
})

// 2026-10-08 作者回報：中文全形標點旁的粗體沒生效（CommonMark 的 flanking 規則不認 CJK），清單也要有編號。
describe('中日韓文字的強調', () => {
  it('全形標點旁的粗體照常生效', () => {
    const html = renderToStaticMarkup(<RichContent>{'**要你決定：**用一句話寫下今天最想推進的一件事。'}</RichContent>)
    expect(html).toContain('<strong>要你決定：</strong>用一句話')
    expect(html).not.toContain('**')
  })

  it('引號與括號旁的粗體也生效', () => {
    const html = renderToStaticMarkup(<RichContent inline>{'先看**「督導」**頁，再按**（打開原對話）**'}</RichContent>)
    expect(html).toContain('<strong>「督導」</strong>')
    expect(html).toContain('<strong>（打開原對話）</strong>')
  })
})

// QA 第 4 輪（AC-EC-11）：sanitizer 拿掉 href 的連結（javascript: 等）不能常駐「再按一次用瀏覽器開啟」。
describe('沒有 href 的連結', () => {
  it('不顯示確認提示', () => {
    const html = renderToStaticMarkup(
      <RichContent>{'[j](javascript:alert(1)) 與 <a href="javascript:alert(2)">k</a> 與 <a>沒有網址</a>'}</RichContent>,
    )
    expect(html).not.toContain('再按一次')
    expect(html).not.toContain('data-confirm')
    expect(html).not.toContain('javascript:')
  })

  it('有網址的連結第一次渲染也不顯示提示（點了才提示）', () => {
    const html = renderToStaticMarkup(<RichContent>{'[官網](https://a.test)'}</RichContent>)
    expect(html).toContain('href="https://a.test"')
    expect(html).not.toContain('再按一次')
  })
})
