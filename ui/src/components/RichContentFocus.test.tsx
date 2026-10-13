// 決策面板 QA 第 3 輪（AC-DS-13、AC-DS-07）：連結要能用鍵盤打開——Tab 到連結、Enter 一次顯示「再按一次用瀏覽器開啟」、
// 再 Enter 一次才開。瀏覽器實測：第一次 Enter 後提示出現，連結元素卻整個重新掛載（`components.a` 每次渲染都是新的函式，
// React 視為不同元件），焦點掉到 body，第二次 Enter 打不開。行內模式的 `p` 也一樣（包著連結的段落被重掛）。
// 這裡在 node 裡檢查交給 react-markdown 的元件在不同渲染之間是同一個（重新渲染只更新、不重掛載）。
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'

const seen = vi.hoisted(() => [] as Record<string, unknown>[])
vi.mock('react-markdown', async (importOriginal) => {
  const mod = await importOriginal<typeof import('react-markdown')>()
  const Original = mod.default
  return {
    ...mod,
    default: (props: Parameters<typeof Original>[0]) => {
      seen.push({ ...(props.components ?? {}) })
      return Original(props)
    },
  }
})

import { RichContent } from './RichContent'

describe('AC-DS-13 連結在重新渲染時不重掛載（鍵盤 Enter 兩次能打開）', () => {
  it('交給 react-markdown 的連結與行內段落元件跨渲染是同一個', () => {
    seen.length = 0
    renderToStaticMarkup(
      <RichContent inline onOpenLink={() => {}}>
        {'見 [試算表](https://sheets.example.com/s)'}
      </RichContent>,
    )
    renderToStaticMarkup(
      <RichContent inline onOpenLink={() => {}}>
        {'見 [試算表](https://sheets.example.com/s)'}
      </RichContent>,
    )
    expect(seen).toHaveLength(2)
    expect(seen[0].a, '連結元件每次渲染都換一個：提示出現時連結被重掛載、焦點掉到 body').toBe(seen[1].a)
    expect(seen[0].p, '行內段落元件每次渲染都換一個：包著的連結跟著重掛載').toBe(seen[1].p)
  })
})
