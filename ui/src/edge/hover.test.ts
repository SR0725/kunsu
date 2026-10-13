// 邊緣瀏海 hover 的隱私守門、逾時收回與舞台識別（BDD edge-notch AC-EN-11、12、21；QA 第 1 輪）。
import { describe, expect, it } from 'vitest'
import {
  HOVER_STALE,
  contentAllowed,
  hoverBegan,
  hoverStale,
  initialPrivacy,
  privacyFadeIn,
  privacyPushed,
  privacyRead,
  stageIdentity,
} from './hover'

describe('AC-EN-12 每次 hover 先當作隱私中', () => {
  it('hover 開始到拿到這次的最新隱私狀態之前，不顯示題目內容', () => {
    let g = hoverBegan(initialPrivacy)
    expect(contentAllowed(g)).toBe(false)
    g = privacyRead(g, g.epoch, false)
    expect(contentAllowed(g)).toBe(true)
  })

  it('之前確認過未命中也不沿用：下一次 hover 重新確認', () => {
    let g = privacyPushed(initialPrivacy, false)
    g = hoverBegan(g)
    expect(contentAllowed(g)).toBe(false)
    g = privacyRead(g, g.epoch, false)
    g = hoverBegan(g)
    expect(contentAllowed(g)).toBe(false)
  })

  it('hover 中 shell 推送命中，立即改回狀態文字；推送解除後恢復', () => {
    let g = hoverBegan(initialPrivacy)
    g = privacyRead(g, g.epoch, false)
    expect(contentAllowed(g)).toBe(true)
    g = privacyPushed(g, true)
    expect(contentAllowed(g)).toBe(false)
    g = privacyPushed(g, false)
    expect(contentAllowed(g)).toBe(true)
  })

  it('上一次 hover 的讀取回應不算數', () => {
    let g = hoverBegan(initialPrivacy)
    const first = g.epoch
    g = hoverBegan(g)
    g = privacyRead(g, first, false)
    expect(contentAllowed(g)).toBe(false)
  })

  it('讀取回應說命中就維持隱私中', () => {
    let g = hoverBegan(initialPrivacy)
    g = privacyRead(g, g.epoch, true)
    expect(contentAllowed(g)).toBe(false)
  })

  it('這次 hover 已收到推送命中時，較慢回來的讀取回應不會重新放行', () => {
    let g = hoverBegan(initialPrivacy)
    g = privacyPushed(g, true)
    g = privacyRead(g, g.epoch, false)
    expect(contentAllowed(g)).toBe(false)
  })
})

describe('AC-EN-11 hover 不會卡住', () => {
  it('3 秒沒有任何「在瀏海上」訊號就收回', () => {
    expect(HOVER_STALE).toBe(3000)
    expect(hoverStale(true, 1000, 1000 + HOVER_STALE - 1)).toBe(false)
    expect(hoverStale(true, 1000, 1000 + HOVER_STALE + 1)).toBe(true)
    expect(hoverStale(false, 0, 1e9)).toBe(false)
  })
})

describe('AC-EN-21 非幾何狀態改變不移動瀏海', () => {
  it('舞台識別只看螢幕大小與螢幕本身，不看隱私、選單列高度或保存值', () => {
    const base = { width: 1440, height: 900, menubar: 37, display: 'Built-in', saved: null }
    const id = stageIdentity(base)
    expect(stageIdentity({ ...base, saved: { corner: 'tr' } })).toBe(id)
    expect(stageIdentity({ ...base, menubar: 24 })).toBe(id)
    expect(stageIdentity({ ...base, privacy: true } as typeof base)).toBe(id)
    expect(stageIdentity({ ...base, width: 1512 })).not.toBe(id)
    expect(stageIdentity({ ...base, height: 982 })).not.toBe(id)
    expect(stageIdentity({ ...base, display: 'LG' })).not.toBe(id)
  })

  it('同名螢幕以螢幕識別區分（QA 第 2 輪）', () => {
    const dell = { width: 1920, height: 1080, menubar: 25, display: 'DELL U2723QE', display_id: 3, saved: null }
    expect(stageIdentity({ ...dell, display_id: 4 })).not.toBe(stageIdentity(dell))
    expect(stageIdentity({ ...dell, saved: { corner: 'tl' }, menubar: 24 })).toBe(stageIdentity(dell))
  })
})

describe('AC-EN-21 隱私解除不讓靜止文字閃（QA 第 4 輪）', () => {
  it('沒有 hover 時隱私解除不重新淡入靜止文字；hover 中才淡入（QA 第 4 輪）', () => {
    // 上一次 hover 已確認未命中，之後麥克風打開（推送命中）又關掉（推送解除）——這時沒有 hover，畫的是靜止文字。
    let g = hoverBegan(initialPrivacy)
    g = privacyRead(g, g.epoch, false)
    g = privacyPushed(g, true)
    const before = contentAllowed(g)
    g = privacyPushed(g, false)
    const after = contentAllowed(g)
    expect([before, after]).toEqual([false, true])
    expect(privacyFadeIn(before, after, false), '沒有 hover：靜止文字不重新淡入').toBe(false)
    // hover 中（跑馬燈從狀態文字換成題目）：淡入
    expect(privacyFadeIn(before, after, true)).toBe(true)
    // 內容沒有從「不可顯示」變成「可顯示」：不淡入
    expect(privacyFadeIn(true, true, true)).toBe(false)
    expect(privacyFadeIn(true, false, true)).toBe(false)
    expect(privacyFadeIn(false, false, true)).toBe(false)
  })
})
