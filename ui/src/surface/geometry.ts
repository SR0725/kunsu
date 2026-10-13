// 決策面板「瀏海張開」的形狀幾何（DESIGN「Decision surface」、BDD decision-surface AC-DS-01）。
// 純函數：先沿瀏海所在邊延伸（pA），再往螢幕內展開成面板（pB）。每個進度都是連續、合法的形狀。

export type Edge = 'left' | 'right' | 'top' | 'bottom'
export type Corner = 'tl' | 'tr' | 'bl' | 'br'
export type Anchor = Edge | Corner

export interface Pt {
  x: number
  y: number
}

/** 瀏海靜止時的幾何：直帶以沿邊座標 [u0, u1] 表示；角落 L 形以兩臂自由端到角落的距離表示。 */
export interface NotchShape {
  anchor: Anchor
  /** 直帶：沿邊起點與終點（螢幕座標軸上的位置）。 */
  u0?: number
  u1?: number
  /** L 形：水平臂與垂直臂從角落量起的長度。 */
  armH?: number
  armV?: number
}

export interface MorphFrame {
  /** 每個元素是一個封閉多邊形（SVG path d）。全部填黑後的聯集就是形狀。 */
  paths: string[]
  /** 面板最終內容區（螢幕座標）。 */
  content: { x: number; y: number; w: number; h: number }
}

export const T = 32
export const PAD = 14
const M = 40
const R_CAP = 12
const R_SH = 7
const R_IN = 14
const R_PANEL = 28

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v))
const lerp = (a: number, b: number, t: number) => a + (b - a) * t

function arcPts(c: Pt, p0: Pt, p1: Pt, n = 8): Pt[] {
  const r = Math.hypot(p0.x - c.x, p0.y - c.y) || 0.0001
  const v0 = { x: (p0.x - c.x) / r, y: (p0.y - c.y) / r }
  const v1 = { x: (p1.x - c.x) / r, y: (p1.y - c.y) / r }
  const out: Pt[] = []
  for (let i = 0; i <= n; i++) {
    const t = i / n
    const x = v0.x * (1 - t) + v1.x * t
    const y = v0.y * (1 - t) + v1.y * t
    const l = Math.hypot(x, y) || 1
    out.push({ x: c.x + (x / l) * r, y: c.y + (y / l) * r })
  }
  return out
}

/** 矩形：左上 (x0,y0) 通常在螢幕外；只圓 TR／BR／BL 三個內側角。 */
function rect(x0: number, y0: number, x1: number, y1: number, rTR: number, rBR: number, rBL = 0): Pt[] {
  const pts: Pt[] = [{ x: x0, y: y0 }]
  if (rTR > 0.3) pts.push(...arcPts({ x: x1 - rTR, y: y0 + rTR }, { x: x1 - rTR, y: y0 }, { x: x1, y: y0 + rTR }))
  else pts.push({ x: x1, y: y0 })
  if (rBR > 0.3) pts.push(...arcPts({ x: x1 - rBR, y: y1 - rBR }, { x: x1, y: y1 - rBR }, { x: x1 - rBR, y: y1 }))
  else pts.push({ x: x1, y: y1 })
  if (rBL > 0.3) pts.push(...arcPts({ x: x0 + rBL, y: y1 - rBL }, { x: x0 + rBL, y: y1 }, { x: x0, y: y1 - rBL }))
  else pts.push({ x: x0, y: y1 })
  return pts
}

/** 反向圓角補料：在 q 往對角方向 d 補「正方形減四分之一圓」。 */
function fillet(q: Pt, d: Pt, r: number): Pt[] | null {
  if (r < 0.3) return null
  const c = { x: q.x + d.x * r, y: q.y + d.y * r }
  return [
    { x: q.x - d.x, y: q.y - d.y },
    { x: q.x + d.x * r, y: q.y - d.y },
    ...arcPts(c, { x: q.x + d.x * r, y: q.y }, { x: q.x, y: q.y + d.y * r }),
    { x: q.x - d.x, y: q.y + d.y * r },
  ]
}

const fmt = (n: number) => (Math.round(n * 100) / 100).toString()
const polyD = (pts: Pt[]) => 'M' + pts.map((p) => `${fmt(p.x)} ${fmt(p.y)}`).join('L') + 'Z'

interface Canonical {
  polys: Pt[][]
  content: { x: number; y: number; w: number; h: number }
}

/** 標準座標：左邊直帶（寬 cw、高 ch；沿邊座標 = y）。 */
function leftBand(u0n: number, u1n: number, pA: number, pB: number, cw: number, ch: number): Canonical {
  const u0 = lerp(u0n, PAD, pA)
  const u1 = lerp(u1n, ch - PAD, pA)
  const depth = lerp(T, cw - PAD, pB)
  const rCap = Math.max(0, Math.min(lerp(R_CAP, R_PANEL, pB), (u1 - u0) / 2, depth))
  const polys: Pt[][] = [rect(-M, u0, depth, u1, rCap, rCap)]
  const top = fillet({ x: 0, y: u0 }, { x: 1, y: -1 }, Math.min(R_SH, u0))
  const bottom = fillet({ x: 0, y: u1 }, { x: 1, y: 1 }, Math.min(R_SH, ch - u1))
  if (top) polys.push(top)
  if (bottom) polys.push(bottom)
  return { polys, content: { x: 0, y: PAD, w: cw - PAD, h: ch - 2 * PAD } }
}

/** 標準座標：左上角 L（上臂自由端 x1、左臂自由端 y1）＋從角落長大的核心。 */
function topLeftL(armH: number, armV: number, pA: number, pB: number, cw: number, ch: number): Canonical {
  const x1 = lerp(armH, cw - PAD, pA)
  const y1 = lerp(armV, ch - PAD, pA)
  const w = lerp(T, cw - PAD, pB)
  const h = lerp(T, ch - PAD, pB)
  const rCore = Math.min(R_PANEL, Math.max(0, w - T), Math.max(0, h - T))
  const polys: Pt[][] = [
    rect(-M, -M, x1, T, 0, Math.max(0, Math.min(R_CAP, x1 - T))),
    rect(-M, -M, T, y1, 0, Math.max(0, Math.min(R_CAP, y1 - T))),
    rect(-M, -M, w, h, 0, rCore),
  ]
  const extras = [
    fillet({ x: T, y: T }, { x: 1, y: 1 }, Math.max(0, Math.min(R_IN, x1 - T, y1 - T))),
    fillet({ x: w, y: T }, { x: 1, y: 1 }, Math.max(0, Math.min(R_IN, x1 - w - R_CAP, h - T - rCore))),
    fillet({ x: T, y: h }, { x: 1, y: 1 }, Math.max(0, Math.min(R_IN, y1 - h - R_CAP, w - T - rCore))),
    fillet({ x: x1, y: 0 }, { x: 1, y: 1 }, Math.min(R_SH, cw - x1)),
    fillet({ x: 0, y: y1 }, { x: 1, y: 1 }, Math.min(R_SH, ch - y1)),
  ]
  for (const e of extras) if (e) polys.push(e)
  return { polys, content: { x: 0, y: 0, w: cw - PAD, h: ch - PAD } }
}

function mapRect(r: Canonical['content'], map: (p: Pt) => Pt) {
  const a = map({ x: r.x, y: r.y })
  const b = map({ x: r.x + r.w, y: r.y + r.h })
  return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(b.x - a.x), h: Math.abs(b.y - a.y) }
}

export function morphFrame(
  notch: NotchShape,
  pA: number,
  pB: number,
  screen: { w: number; h: number },
): MorphFrame {
  const W = screen.w
  const H = screen.h
  const a = clamp(pA, 0, 1)
  const b = clamp(pB, 0, 1)
  let canonical: Canonical
  let map: (p: Pt) => Pt
  switch (notch.anchor) {
    case 'left':
      canonical = leftBand(notch.u0 ?? 0, notch.u1 ?? T, a, b, W, H)
      map = (p) => p
      break
    case 'right':
      canonical = leftBand(notch.u0 ?? 0, notch.u1 ?? T, a, b, W, H)
      map = (p) => ({ x: W - p.x, y: p.y })
      break
    case 'top':
      canonical = leftBand(notch.u0 ?? 0, notch.u1 ?? T, a, b, H, W)
      map = (p) => ({ x: p.y, y: p.x })
      break
    case 'bottom':
      canonical = leftBand(notch.u0 ?? 0, notch.u1 ?? T, a, b, H, W)
      map = (p) => ({ x: p.y, y: H - p.x })
      break
    default: {
      canonical = topLeftL(notch.armH ?? 96, notch.armV ?? 96, a, b, W, H)
      const flipX = notch.anchor === 'tr' || notch.anchor === 'br'
      const flipY = notch.anchor === 'bl' || notch.anchor === 'br'
      map = (p) => ({ x: flipX ? W - p.x : p.x, y: flipY ? H - p.y : p.y })
    }
  }
  return {
    paths: canonical.polys.map((poly) => polyD(poly.map(map))),
    content: mapRect(canonical.content, map),
  }
}

/** 由轉場原點（瀏海圖示中心）推得靜止瀏海形狀：最近的邊、以原點為中心、長度 160。 */
export function notchFromOrigin(origin: Pt, screen: { w: number; h: number }, length = 160): NotchShape {
  const d = { left: origin.x, right: screen.w - origin.x, top: origin.y, bottom: screen.h - origin.y }
  const edge = (Object.keys(d) as Edge[]).reduce((best, e) => (d[e] < d[best] ? e : best), 'left' as Edge)
  const along = edge === 'left' || edge === 'right' ? origin.y : origin.x
  const span = edge === 'left' || edge === 'right' ? screen.h : screen.w
  const u0 = clamp(along - length / 2, 0, Math.max(0, span - length))
  return { anchor: edge, u0, u1: u0 + length }
}

/** 臨界阻尼響應 1-(1+ωt)e^{-ωt}：不回彈、收尾柔和。t 秒。 */
export function critical(t: number, omega: number): number {
  if (t <= 0) return 0
  return 1 - (1 + omega * t) * Math.exp(-omega * t)
}

/** 開啟動畫在時間 t（秒）的兩段進度：先沿邊延伸，0.09 秒後往內展開；0.2 秒後內容淡入。 */
export function openProgress(t: number): { pA: number; pB: number; contentVisible: boolean } {
  return { pA: critical(t, 20), pB: critical(t - 0.09, 11), contentVisible: t > 0.2 }
}

/** 收起：從當下的進度倒放——先往內收（pB），再沿邊縮回瀏海（pA）。 */
export function closeProgress(t: number, from: { pA: number; pB: number }): { pA: number; pB: number } {
  return { pA: from.pA * (1 - critical(t - 0.24, 18)), pB: from.pB * (1 - critical(t - 0.12, 14)) }
}

/** 動畫迴圈沒有執行時的計時後備（AC-DS-01）：張開最晚這時候定案、收起最晚這時候完成。 */
export const OPEN_FALLBACK_MS = 1200
export const CLOSE_FALLBACK_MS = 900

/** 沒有形狀時（減少動態、主視窗）面板貼齊瀏海所在的邊，只有朝螢幕內的角是圓角（CSS border-radius 順序：左上 右上 右下 左下）。 */
export function panelRadius(anchor: Anchor): string {
  const r = `${R_PANEL}px`
  switch (anchor) {
    case 'left':
      return `0 ${r} ${r} 0`
    case 'right':
      return `${r} 0 0 ${r}`
    case 'top':
      return `0 0 ${r} ${r}`
    case 'bottom':
      return `${r} ${r} 0 0`
    case 'tl':
      return `0 0 ${r} 0`
    case 'tr':
      return `0 0 0 ${r}`
    case 'bl':
      return `0 ${r} 0 0`
    case 'br':
      return `${r} 0 0 0`
  }
}
