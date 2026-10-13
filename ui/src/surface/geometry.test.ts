// AC-DS-01：瀏海張開成面板的形狀幾何。
import { describe, expect, it } from 'vitest'
import { CLOSE_FALLBACK_MS, closeProgress, critical, morphFrame, notchFromOrigin, OPEN_FALLBACK_MS, openProgress, PAD, panelRadius, T } from './geometry'
import type { NotchShape } from './geometry'

const screen = { w: 1440, h: 900 }

function numbers(d: string): number[] {
  return (d.match(/-?\d+(\.\d+)?/g) ?? []).map(Number)
}

function bbox(paths: string[]) {
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (const d of paths) {
    const n = numbers(d)
    for (let i = 0; i + 1 < n.length; i += 2) {
      const [x, y] = [n[i], n[i + 1]]
      // 螢幕外延伸部分不算可見範圍
      const cx = Math.max(0, Math.min(screen.w, x))
      const cy = Math.max(0, Math.min(screen.h, y))
      minX = Math.min(minX, cx)
      minY = Math.min(minY, cy)
      maxX = Math.max(maxX, cx)
      maxY = Math.max(maxY, cy)
    }
  }
  return { minX, minY, maxX, maxY }
}

const anchors: NotchShape[] = [
  { anchor: 'left', u0: 45, u1: 205 },
  { anchor: 'right', u0: 300, u1: 460 },
  { anchor: 'top', u0: 200, u1: 360 },
  { anchor: 'bottom', u0: 600, u1: 760 },
  { anchor: 'tl', armH: 96, armV: 96 },
  { anchor: 'tr', armH: 96, armV: 96 },
  { anchor: 'bl', armH: 96, armV: 96 },
  { anchor: 'br', armH: 96, armV: 96 },
]

describe('morphFrame', () => {
  it.each(anchors)('$anchor：靜止時就是瀏海本身（厚度 T、貼邊）', (notch) => {
    const f = morphFrame(notch, 0, 0, screen)
    expect(f.paths.length).toBeGreaterThan(0)
    const b = bbox(f.paths)
    const width = b.maxX - b.minX
    const height = b.maxY - b.minY
    if (notch.anchor === 'left' || notch.anchor === 'right') {
      expect(width).toBeCloseTo(T, 0)
      expect(height).toBeGreaterThanOrEqual((notch.u1 ?? 0) - (notch.u0 ?? 0) - 1)
    } else if (notch.anchor === 'top' || notch.anchor === 'bottom') {
      expect(height).toBeCloseTo(T, 0)
    } else {
      expect(width).toBeLessThan(140)
      expect(height).toBeLessThan(140)
    }
    // 貼齊實體邊
    if (notch.anchor === 'left' || notch.anchor === 'tl' || notch.anchor === 'bl') expect(b.minX).toBe(0)
    if (notch.anchor === 'right' || notch.anchor === 'tr' || notch.anchor === 'br') expect(b.maxX).toBe(screen.w)
    if (notch.anchor === 'top' || notch.anchor === 'tl' || notch.anchor === 'tr') expect(b.minY).toBe(0)
    if (notch.anchor === 'bottom' || notch.anchor === 'bl' || notch.anchor === 'br') expect(b.maxY).toBe(screen.h)
  })

  it.each(anchors)('$anchor：完全展開時覆蓋整個面板，非貼邊側內縮 PAD', (notch) => {
    const f = morphFrame(notch, 1, 1, screen)
    const b = bbox(f.paths)
    expect(b.maxX - b.minX).toBeGreaterThan(screen.w - 2 * PAD - 1)
    expect(b.maxY - b.minY).toBeGreaterThan(screen.h - 2 * PAD - 1)
    const c = f.content
    expect(c.w).toBeGreaterThan(screen.w - 2 * PAD - 1)
    expect(c.h).toBeGreaterThan(screen.h - 2 * PAD - 1)
    expect(c.x).toBeGreaterThanOrEqual(0)
    expect(c.y).toBeGreaterThanOrEqual(0)
    expect(c.x + c.w).toBeLessThanOrEqual(screen.w)
    expect(c.y + c.h).toBeLessThanOrEqual(screen.h)
  })

  it.each(anchors)('$anchor：任何進度都是有限數值且不跳形（面積單調增加）', (notch) => {
    let last = 0
    for (let i = 0; i <= 20; i++) {
      const p = i / 20
      const f = morphFrame(notch, Math.min(1, p * 1.6), p, screen)
      for (const d of f.paths) {
        expect(d.startsWith('M')).toBe(true)
        for (const n of numbers(d)) expect(Number.isFinite(n)).toBe(true)
      }
      const b = bbox(f.paths)
      const area = (b.maxX - b.minX) * (b.maxY - b.minY)
      expect(area).toBeGreaterThanOrEqual(last - 1)
      last = area
    }
  })
})

describe('notchFromOrigin', () => {
  it('picks the nearest edge and centres the band on the origin', () => {
    expect(notchFromOrigin({ x: 16, y: 300 }, screen)).toEqual({ anchor: 'left', u0: 220, u1: 380 })
    expect(notchFromOrigin({ x: 1430, y: 300 }, screen).anchor).toBe('right')
    expect(notchFromOrigin({ x: 700, y: 10 }, screen).anchor).toBe('top')
    expect(notchFromOrigin({ x: 700, y: 895 }, screen).anchor).toBe('bottom')
    const clamped = notchFromOrigin({ x: 10, y: 20 }, screen)
    expect(clamped.u0).toBeGreaterThanOrEqual(0)
  })
})

describe('timing', () => {
  it('critical damping is monotone, never overshoots and settles', () => {
    let prev = 0
    for (let t = 0; t <= 1; t += 0.02) {
      const v = critical(t, 11)
      expect(v).toBeGreaterThanOrEqual(prev - 1e-9)
      expect(v).toBeLessThanOrEqual(1)
      prev = v
    }
    expect(critical(0, 11)).toBe(0)
    expect(critical(1.2, 11)).toBeGreaterThan(0.99)
  })

  it('extends along the edge first, then spreads; content after 0.2 s', () => {
    const early = openProgress(0.08)
    expect(early.pA).toBeGreaterThan(early.pB)
    expect(early.contentVisible).toBe(false)
    expect(openProgress(0.25).contentVisible).toBe(true)
    const done = openProgress(1.5)
    expect(done.pA).toBeGreaterThan(0.999)
    expect(done.pB).toBeGreaterThan(0.99)
  })
})

// QA 第 1 輪 #5（AC-DS-01）：動畫迴圈沒跑時的計時後備；收起從當下進度倒放。
describe('fallback timing', () => {
  it('張開的後備在自然定案之後才觸發，不截斷正在跑的動畫，且不超過 1.2 秒', () => {
    const p = openProgress(OPEN_FALLBACK_MS / 1000)
    expect(p.pA).toBeGreaterThan(0.999)
    expect(p.pB).toBeGreaterThan(0.999)
    expect(OPEN_FALLBACK_MS).toBeLessThanOrEqual(1200)
  })
  it('收起從當下的形狀開始倒放，後備時間內一定收完，且不超過 0.9 秒', () => {
    expect(closeProgress(0, { pA: 0.6, pB: 0.3 })).toEqual({ pA: 0.6, pB: 0.3 })
    const mid = closeProgress(0.3, { pA: 1, pB: 1 })
    expect(mid.pB).toBeLessThan(1)
    const end = closeProgress(CLOSE_FALLBACK_MS / 1000, { pA: 1, pB: 1 })
    expect(end.pA).toBeLessThan(0.002)
    expect(end.pB).toBeLessThan(0.002)
    expect(CLOSE_FALLBACK_MS).toBeLessThanOrEqual(900)
  })
})

// QA 第 1 輪 #14（AC-DS-01）：沒有形狀時（減少動態、主視窗）只有朝螢幕內的角是圓角。
describe('panelRadius', () => {
  it('貼齊瀏海所在的邊，朝螢幕內的角 28 px', () => {
    expect(panelRadius('left')).toBe('0 28px 28px 0')
    expect(panelRadius('right')).toBe('28px 0 0 28px')
    expect(panelRadius('top')).toBe('0 0 28px 28px')
    expect(panelRadius('bottom')).toBe('28px 28px 0 0')
    expect(panelRadius('tl')).toBe('0 0 28px 0')
    expect(panelRadius('tr')).toBe('0 0 0 28px')
    expect(panelRadius('bl')).toBe('0 28px 0 0')
    expect(panelRadius('br')).toBe('28px 0 0 0')
  })
})
