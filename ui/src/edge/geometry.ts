// 邊緣瀏海幾何（PRD §19、DESIGN「Edge notch」、BDD edge-notch）。
// 移植自 docs/prototypes/edge-notch.html：瀏海是沿螢幕內縮中心線的一段弧長 [s, s+L]，以方形筆刷（厚度 T）
// 掃出黑色帶狀區；跨角時自然成 L 形。所有半徑 = min(名目值, 可用間隙)，保證幾何連續。純函數，可測試。

export const T = 32 // 厚度
export const R_CAP = 12 // 自由端內側凸角
export const R_SH = 7 // 自由端與螢幕邊交界的反向圓角（肩）
export const R_IN = 14 // L 形內側凹角
export const RC_TEXT = 10 // 文字中心線轉角小圓弧
export const M = 40 // 螢幕外延伸（避免邊緣反鋸齒露底）
export const FONT = 12
export const TRACK = 1.5
export const ICON = 18
export const HOVER_EXTRA = 88
export const HOVER_DELAY = 120
export const HOVER_LEAVE = 260
export const PREDICT_S = 0.12
export const HYSTERESIS = 40
export const DRAG_THRESHOLD = 4
export const CORNER_SNAP_RATIO = 0.3
export const EDGE_GAP = 24
export const MARQUEE_SPEED = 38
export const MARQUEE_PAUSE = 800
export const MARQUEE_GAP = 40
export const OMEGA_DRAG = 30
export const OMEGA_SETTLE = 20
/**
 * 長度與中心走同一個彈簧：hover 伸縮與文字長度改變時，中心移長度差的一半，每一幀頭端都不動（場景 10b／11）。拖曳中兩者都用
 * OMEGA_DRAG（`stepMotion`）。
 */
export const OMEGA_LENGTH = OMEGA_SETTLE
/** 文字換頭端後淡入的時間（場景 9）。 */
export const TEXT_FADE_MS = 180

export type Edge = 'top' | 'right' | 'bottom' | 'left'
export type Corner = 'tl' | 'tr' | 'br' | 'bl'
export type HeadEnd = 'ccw' | 'cw'
export interface Pt {
  x: number
  y: number
}
/** 回報給原生層的命中多邊形：扁平 [x0, y0, x1, y1, …]（舞台本地 CSS px，0.01 px 精度），與填色同一組點。 */
export type HitShape = number[]

export interface Stage {
  W: number
  H: number
  /** 上下邊中心線長度。 */
  A: number
  /** 左右邊中心線長度。 */
  B: number
  /** 周長。 */
  P: number
  corners: Record<Corner, number>
}

export function makeStage(W: number, H: number): Stage {
  const A = W - T
  const B = H - T
  return { W, H, A, B, P: 2 * A + 2 * B, corners: { tl: 0, tr: A, br: A + B, bl: 2 * A + B } }
}

export const mod = (v: number, m: number) => ((v % m) + m) % m
export const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v))

export function wrapDelta(st: Stage, d: number): number {
  const x = mod(d, st.P)
  return x > st.P / 2 ? x - st.P : x
}

export function edgeOfS(st: Stage, s: number): Edge {
  const x = mod(s, st.P)
  if (x < st.A) return 'top'
  if (x < st.A + st.B) return 'right'
  if (x < 2 * st.A + st.B) return 'bottom'
  return 'left'
}

export function pointOnCenterline(st: Stage, s: number): Pt {
  const x = mod(s, st.P)
  const { A, B, W, H } = st
  if (x < A) return { x: T / 2 + x, y: T / 2 }
  if (x < A + B) return { x: W - T / 2, y: T / 2 + (x - A) }
  if (x < 2 * A + B) return { x: W - T / 2 - (x - A - B), y: H - T / 2 }
  return { x: T / 2, y: H - T / 2 - (x - 2 * A - B) }
}

/** 點投影到某一邊的周長座標。 */
export function sOnEdge(st: Stage, edge: Edge, p: Pt): number {
  const { A, B, W, H } = st
  if (edge === 'top') return clamp(p.x - T / 2, 0, A)
  if (edge === 'right') return A + clamp(p.y - T / 2, 0, B)
  if (edge === 'bottom') return A + B + clamp(W - T / 2 - p.x, 0, A)
  return 2 * A + B + clamp(H - T / 2 - p.y, 0, B)
}

export interface Piece {
  edge: Edge
  s0: number
  s1: number
}

/** 沿周長從 s0 走 len，於角落切開。 */
export function pieces(st: Stage, s0: number, len: number): Piece[] {
  const out: Piece[] = []
  let s = mod(s0, st.P)
  let remaining = len
  while (remaining > 1e-6 && out.length <= 4) {
    const e = edgeOfS(st, s)
    const edgeEnd = e === 'top' ? st.A : e === 'right' ? st.A + st.B : e === 'bottom' ? 2 * st.A + st.B : st.P
    const step = Math.min(remaining, edgeEnd - s)
    out.push({ edge: e, s0: s, s1: s + step })
    remaining -= step
    s = mod(s + step, st.P)
  }
  return out
}

function toScreen(st: Stage, edge: Edge, u: number, n: number): Pt {
  if (edge === 'top') return { x: u, y: n }
  if (edge === 'bottom') return { x: u, y: st.H - n }
  if (edge === 'left') return { x: n, y: u }
  return { x: st.W - n, y: u }
}

const edgeLen = (st: Stage, edge: Edge) => (edge === 'top' || edge === 'bottom' ? st.W : st.H)

function centerU(st: Stage, edge: Edge, s: number): number {
  const p = pointOnCenterline(st, s)
  return edge === 'top' || edge === 'bottom' ? p.x : p.y
}

function arcPts(c: Pt, p0: Pt, p1: Pt, n = 10): Pt[] {
  const r = Math.hypot(p0.x - c.x, p0.y - c.y)
  const v0 = { x: (p0.x - c.x) / r, y: (p0.y - c.y) / r }
  const v1 = { x: (p1.x - c.x) / r, y: (p1.y - c.y) / r }
  const pts: Pt[] = []
  for (let i = 0; i <= n; i++) {
    const t = i / n
    const x = v0.x * (1 - t) + v1.x * t
    const y = v0.y * (1 - t) + v1.y * t
    const l = Math.hypot(x, y) || 1
    pts.push({ x: c.x + (x / l) * r, y: c.y + (y / l) * r })
  }
  return pts
}

const polyD = (pts: Pt[]) => 'M' + pts.map((p) => `${p.x.toFixed(2)} ${p.y.toFixed(2)}`).join('L') + 'Z'
/** 命中多邊形與 SVG d 用同一個兩位小數，兩者完全一致。 */
const flatPoly = (pts: Pt[]): HitShape => pts.flatMap((p) => [Number(p.x.toFixed(2)), Number(p.y.toFixed(2))])

/** 半徑 = min(名目值, 間隙, 突出量)，不為負。 */
export function capRadius(nominal: number, clear: number, protrusion: number): number {
  return Math.max(0, Math.min(nominal, clear, protrusion))
}

export interface BandPiece extends Piece {
  uLo: number
  uHi: number
  startIsLo: boolean
  attachedStart: boolean
  attachedEnd: boolean
  u0: number
  u1: number
}

const innerCorner = (st: Stage): Record<Corner, Pt> => ({
  tl: { x: T, y: T },
  tr: { x: st.W - T, y: T },
  br: { x: st.W - T, y: st.H - T },
  bl: { x: T, y: st.H - T },
})
const inwardDiag: Record<Corner, Pt> = { tl: { x: 1, y: 1 }, tr: { x: -1, y: 1 }, br: { x: -1, y: -1 }, bl: { x: 1, y: -1 } }

export function cornerBetween(e1: Edge, e2: Edge): Corner {
  const k = [e1, e2].sort().join('-')
  return ({ 'left-top': 'tl', 'right-top': 'tr', 'bottom-right': 'br', 'bottom-left': 'bl' } as Record<string, Corner>)[k]
}

/**
 * 帶狀幾何：填色路徑（SVG d）與命中多邊形（舞台本地座標）。兩者是同一組多邊形（AC-EN-14）：看得到的黑色都點得到，
 * 看不到的地方不攔截。
 */
export function bandGeometry(st: Stage, s0: number, len: number): { pieces: BandPiece[]; shapes: string[]; hit: HitShape[] } {
  const ps = pieces(st, s0, len)
  const polys: Pt[][] = []
  const info: BandPiece[] = ps.map((pc, i) => {
    const uA = centerU(st, pc.edge, pc.s0)
    const uB = centerU(st, pc.edge, pc.s1)
    return {
      ...pc,
      uLo: Math.min(uA, uB),
      uHi: Math.max(uA, uB),
      startIsLo: uA <= uB,
      attachedStart: i > 0,
      attachedEnd: i < ps.length - 1,
      u0: 0,
      u1: 0,
    }
  })
  for (const pc of info) {
    const L = edgeLen(st, pc.edge)
    const loAttached = pc.startIsLo ? pc.attachedStart : pc.attachedEnd
    const hiAttached = pc.startIsLo ? pc.attachedEnd : pc.attachedStart
    const u0 = loAttached ? -M : pc.uLo - T / 2
    const u1 = hiAttached ? L + M : pc.uHi + T / 2
    // 間隙：自由端到垂直螢幕邊的距離；突出量：L 形手臂超出另一臂厚度的長度
    const clearLo = u0
    const clearHi = L - u1
    const protLo = info.length > 1 ? L - T - u0 : Infinity
    const protHi = info.length > 1 ? u1 - T : Infinity
    const rLo = loAttached ? 0 : capRadius(R_CAP, clearLo, protLo)
    const rHi = hiAttached ? 0 : capRadius(R_CAP, clearHi, protHi)
    const map = (u: number, n: number) => toScreen(st, pc.edge, u, n)
    const pts: Pt[] = [map(u0, -M), map(u1, -M)]
    if (rHi > 0.01) pts.push(...arcPts(map(u1 - rHi, T - rHi), map(u1, T - rHi), map(u1 - rHi, T)))
    else pts.push(map(u1, T))
    if (rLo > 0.01) pts.push(...arcPts(map(u0 + rLo, T - rLo), map(u0 + rLo, T), map(u0, T - rLo)))
    else pts.push(map(u0, T))
    polys.push(pts)
    // 肩：自由端與螢幕邊交界的反向圓角
    const shoulder = (u: number, dir: number, clear: number, prot: number) => {
      const r = capRadius(R_SH, clear, prot)
      if (r < 0.3) return
      const ue = u + dir * r
      polys.push([map(u - dir, -M), map(ue, -M), ...arcPts(map(ue, r), map(ue, 0), map(u, r)), map(u - dir, r)])
    }
    if (!loAttached) shoulder(u0, -1, clearLo, protLo)
    if (!hiAttached) shoulder(u1, +1, clearHi, protHi)
    pc.u0 = u0
    pc.u1 = u1
  }
  // L 形內側凹角
  const inner = innerCorner(st)
  for (let i = 0; i + 1 < info.length; i++) {
    const a = info[i]
    const b = info[i + 1]
    const corner = cornerBetween(a.edge, b.edge)
    if (!corner) continue
    const q = inner[corner]
    const r = capRadius(R_IN, armProtrusion(st, a), armProtrusion(st, b))
    if (r < 0.3) continue
    const d = inwardDiag[corner]
    const c = { x: q.x + d.x * r, y: q.y + d.y * r }
    polys.push([
      { x: q.x - d.x, y: q.y - d.y },
      { x: q.x + d.x * r, y: q.y - d.y },
      ...arcPts(c, { x: q.x + d.x * r, y: q.y }, { x: q.x, y: q.y + d.y * r }),
      { x: q.x - d.x, y: q.y + d.y * r },
    ])
  }
  return { pieces: info, shapes: polys.map(polyD), hit: polys.map(flatPoly) }
}

function armProtrusion(st: Stage, pc: BandPiece): number {
  const L = edgeLen(st, pc.edge)
  if (pc.attachedStart && pc.attachedEnd) return Infinity
  return pc.u0 < 0 ? pc.u1 - T : L - T - pc.u0
}

/** 整段在同一邊時的頭端：上方與右側在周長起點端（左端、頂端），下方與左側在終點端（左端、頂端）。 */
export const edgeHead = (edge: Edge): HeadEnd => (edge === 'top' || edge === 'right' ? 'ccw' : 'cw')

/** 圖示（頭）所在端：ccw＝周長起點端、cw＝周長終點端。靜止時依邊偏好決定，拖曳中凍結（場景 8／9）。 */
export function preferredHead(ps: Piece[]): HeadEnd {
  const edges = new Set(ps.map((p) => p.edge))
  if (edges.size === 1) return edgeHead(ps[0].edge)
  if (edges.has('top') && edges.has('right')) return 'ccw' // 右上：頭在上臂左端，先右讀再往下
  if (edges.has('bottom') && edges.has('left')) return 'cw' // 左下：頭在左臂上端，先往下再右讀
  if (edges.has('left') && edges.has('top')) return 'cw' // 左上：頭在上臂右端（使用者草圖）
  return 'ccw' // 右下：頭在右臂上端
}

const sub = (a: Pt, b: Pt) => ({ x: a.x - b.x, y: a.y - b.y })
const add = (a: Pt, b: Pt) => ({ x: a.x + b.x, y: a.y + b.y })
const mul = (a: Pt, k: number) => ({ x: a.x * k, y: a.y * k })
const dist = (a: Pt, b: Pt) => Math.hypot(a.x - b.x, a.y - b.y)
const unit = (v: Pt) => {
  const l = Math.hypot(v.x, v.y) || 1
  return { x: v.x / l, y: v.y / l }
}

export interface PathPoint extends Pt {
  vertical: boolean | null
  arc: { t: number; turn: number; fromVertical: boolean } | null
  dir: Pt
}

type Prim =
  | { type: 'line'; a: Pt; b: Pt; len: number; dir: Pt; vertical: boolean }
  | { type: 'arc'; c: Pt; r: number; a0: number; da: number; len: number; fromVertical: boolean }

/** 文字中心線（含轉角小圓弧），從頭端往尾端。 */
export function textPath(st: Stage, ps: Piece[], headEnd: HeadEnd): { total: number; at: (d: number) => PathPoint } {
  let ordered = ps.map((p) => ({ edge: p.edge, a: pointOnCenterline(st, p.s0), b: pointOnCenterline(st, p.s1) }))
  // s1 恰好落在角落時 mod 會繞回 0，終點仍需是這一邊的終點。
  ordered = ordered.map((p, i) => {
    const piece = ps[i]
    if (piece.s1 - piece.s0 > 0 && dist(p.a, p.b) < 1e-6) return p
    return p
  })
  if (headEnd === 'cw') ordered = ordered.reverse().map((p) => ({ edge: p.edge, a: p.b, b: p.a }))
  const rcAt = (j: number) => {
    const before = ordered[j - 1]
    const after = ordered[j]
    return Math.max(0, Math.min(RC_TEXT, dist(before.a, before.b) / 2, dist(after.a, after.b) / 2))
  }
  const prims: Prim[] = []
  for (let i = 0; i < ordered.length; i++) {
    const p = ordered[i]
    let a = { ...p.a }
    let b = { ...p.b }
    const dir = unit(sub(b, a))
    if (i > 0) a = add(a, mul(dir, rcAt(i)))
    if (i < ordered.length - 1) b = sub(b, mul(dir, rcAt(i + 1)))
    if (dist(a, b) > 0.01) prims.push({ type: 'line', a, b, len: dist(a, b), dir, vertical: Math.abs(dir.y) > 0.5 })
    if (i < ordered.length - 1) {
      const nxt = ordered[i + 1]
      const ndir = unit(sub(nxt.b, nxt.a))
      const r = rcAt(i + 1)
      const k = p.b
      const s = sub(k, mul(dir, r))
      const e = add(k, mul(ndir, r))
      const c = add(s, mul(ndir, r))
      const a0 = Math.atan2(s.y - c.y, s.x - c.x)
      const a1 = Math.atan2(e.y - c.y, e.x - c.x)
      let da = a1 - a0
      while (da > Math.PI) da -= 2 * Math.PI
      while (da < -Math.PI) da += 2 * Math.PI
      prims.push({ type: 'arc', c, r, a0, da, len: Math.abs(da) * r, fromVertical: Math.abs(dir.y) > 0.5 })
    }
  }
  const total = prims.reduce((n, p) => n + p.len, 0)
  const at = (dIn: number): PathPoint => {
    if (prims.length === 0) return { x: 0, y: 0, vertical: null, arc: null, dir: { x: 1, y: 0 } }
    // 路徑外沿端點切線外插：讓圖示與文字能貼近帶狀區的視覺端點
    if (dIn < 0 || dIn > total) {
      const base = at(dIn < 0 ? 0 : total)
      const ext = dIn < 0 ? dIn : dIn - total
      return { ...base, x: base.x + base.dir.x * ext, y: base.y + base.dir.y * ext, arc: null }
    }
    let d = dIn
    for (const p of prims) {
      if (d <= p.len || p === prims[prims.length - 1]) {
        const t = p.len ? d / p.len : 0
        if (p.type === 'line') {
          return { x: p.a.x + (p.b.x - p.a.x) * t, y: p.a.y + (p.b.y - p.a.y) * t, vertical: p.vertical, arc: null, dir: p.dir }
        }
        const ang = p.a0 + p.da * t
        const dir = { x: -Math.sin(ang) * Math.sign(p.da), y: Math.cos(ang) * Math.sign(p.da) }
        return {
          x: p.c.x + p.r * Math.cos(ang),
          y: p.c.y + p.r * Math.sin(ang),
          vertical: null,
          arc: { t, turn: Math.sign(p.da), fromVertical: p.fromVertical },
          dir,
        }
      }
      d -= p.len
    }
    return at(total)
  }
  return { total, at }
}

export interface Token {
  s: string
  kind: 'cjk' | 'latin' | 'space'
  adv: number
}

/**
 * 英數字詞（場景 8）：連續的半形可見字元（U+0021–U+007E：英文字母、數字與夾在其中的 `$ , . % - : /` 等符號），
 * 另含帶重音的拉丁字母（U+00C0–U+024F）。空白與任何全形字（中文、全形標點、箭頭、全形英數、跑馬燈分隔點）把字詞切開。
 */
const TOKEN_RE = /[!-~\u00C0-\u024F]+|\s+|./gu
const LATIN_RE = /^[!-~\u00C0-\u024F]+$/u

/** 最寬的數字（量寬用；等寬數字的每一格都以它計算）。 */
function widestDigit(measure: (s: string) => number): string {
  let best = '0'
  let width = -Infinity
  for (const d of '0123456789') {
    const w = measure(d)
    if (w > width) [best, width] = [d, w]
  }
  return best
}

/**
 * 中文單字、英數字詞（整段一起量寬度、在直向段一起旋轉）、空白。`measure` 量英數字詞的寬度。數字一律等寬（場景 10、
 * PRD FR-03）：字詞裡的每個數字都換成最寬的數字再量（canvas `measureText` 不支援 `font-variant-numeric`），繪製用
 * `tabular-nums`，所以倒數每秒換字時長度不變。
 */
export function tokenize(text: string, measure: (s: string) => number): Token[] {
  let widest: string | null = null
  return (text.match(TOKEN_RE) ?? []).map((s) => {
    if (/^\s+$/.test(s)) return { s, kind: 'space' as const, adv: 5 }
    if (LATIN_RE.test(s)) {
      const tabular = /\d/.test(s) ? s.replace(/\d/g, () => (widest ??= widestDigit(measure))) : s
      return { s, kind: 'latin' as const, adv: measure(tabular) + TRACK }
    }
    return { s, kind: 'cjk' as const, adv: FONT + TRACK }
  })
}

/** 點到四條螢幕邊的距離。 */
const edgeDistances = (st: Stage, p: Pt): Record<Edge, number> => ({ top: p.y, right: st.W - p.x, bottom: st.H - p.y, left: p.x })
const isHorizontal = (e: Edge) => e === 'top' || e === 'bottom'

/** 換邊判斷：只有另一邊比目前邊近超過遲滯量才換（場景 5）。 */
export function nearestEdge(st: Stage, p: Pt, current: Edge): Edge {
  const d = edgeDistances(st, p)
  let best = current
  for (const e of ['top', 'right', 'bottom', 'left'] as Edge[]) if (d[e] + HYSTERESIS < d[best]) best = e
  return best
}

/**
 * 游標底下那一臂的邊（場景 4）：瀏海各段所在的邊裡離游標最近的那一條，不偏向中心所在的邊、不加換邊遲滯。`len` 是瀏海
 * 目前（畫出來）的長度；沒給就在四條邊裡找。只在瀏海所在的邊裡找：直邊瀏海貼齊角落時按在端點附近，離垂直那條邊較近，
 * 但瀏海沒有長到那條邊上，那條邊的投影被夾在角落。同樣近（角落那一格）時用中心所在的邊。
 */
export function armUnder(st: Stage, s: number, p: Pt, len?: number): Edge {
  const d = edgeDistances(st, p)
  const edges = len === undefined ? EDGES : pieces(st, centerToStart(s, len), Math.max(1, len - T)).map((pc) => pc.edge)
  const own = edgeOfS(st, s)
  let best = edges.includes(own) ? own : edges[0]
  for (const e of edges) if (d[e] < d[best]) best = e
  return best
}

/** 每個角落相鄰的兩條邊（橫的在前）。 */
const CORNER_ARMS: Record<Corner, [Edge, Edge]> = { tl: ['top', 'left'], tr: ['top', 'right'], br: ['bottom', 'right'], bl: ['bottom', 'left'] }

/**
 * 角落方格（場景 4，QA 第 5 輪）：瀏海在某個角落彎成 L 時，兩臂在螢幕角重疊成 T × T 的方格，方格裡每一點同時在兩臂上。
 * 游標在這樣的方格裡（離兩條螢幕邊都小於 T）就回傳那兩臂的邊（橫的在前），否則 null。`len` 是瀏海畫出來的長度。
 */
export function cornerSquareArms(st: Stage, s: number, p: Pt, len: number): [Edge, Edge] | null {
  const corner: Corner = p.y < st.H / 2 ? (p.x < st.W / 2 ? 'tl' : 'tr') : p.x < st.W / 2 ? 'bl' : 'br'
  const [h, v] = CORNER_ARMS[corner]
  const d = edgeDistances(st, p)
  if (d[h] >= T || d[v] >= T) return null
  const edges = pieces(st, centerToStart(s, len), Math.max(1, len - T)).map((pc) => pc.edge)
  return edges.includes(h) && edges.includes(v) ? [h, v] : null
}

/**
 * 開始拖曳（場景 4）：抓的是哪一邊、抓取點相對中心的偏移。兩者用同一條邊算，這條邊就是游標底下那一臂（`armUnder`），
 * 所以在 L 形上抓住任一臂——包括離角落很近的那一段——開始拖曳後頭端只沿周長移動和游標一樣的距離，不會一換邊就跳。
 * 例外：按在角落方格裡（`cornerSquareArms`）時 `arms` 不是 null，`edge`／`offset` 只是暫定；移動滿拖曳門檻時改用
 * `grabByDirection` 依移動方向選臂。沒給 `len` 不判斷方格。
 */
export function grabStart(st: Stage, s: number, p: Pt, len?: number): { edge: Edge; offset: number; arms: [Edge, Edge] | null } {
  const edge = armUnder(st, s, p, len)
  return { edge, offset: wrapDelta(st, sOnEdge(st, edge, p) - s), arms: len === undefined ? null : cornerSquareArms(st, s, p, len) }
}

/**
 * 角落方格裡按下後，移動滿拖曳門檻那一刻選臂（場景 4，QA 第 5 輪）：從按下點 `start` 到 `p` 橫向移動多就選橫的那一臂、
 * 縱向多就選直的，一樣多選離按下點較近的邊（再一樣選橫的）。這次選擇不加換邊遲滯；抓取偏移在選定的邊上、以按下的位置量
 * （`s` 是那一刻畫出來的中心），所以頭端位移等於游標從按下點起沿那一臂的投影位移。
 */
export function grabByDirection(st: Stage, s: number, start: Pt, p: Pt, arms: readonly [Edge, Edge]): { edge: Edge; offset: number } {
  const h = isHorizontal(arms[0]) ? arms[0] : arms[1]
  const v = h === arms[0] ? arms[1] : arms[0]
  const dx = Math.abs(p.x - start.x)
  const dy = Math.abs(p.y - start.y)
  const d = edgeDistances(st, start)
  const edge = dx > dy ? h : dy > dx ? v : d[v] < d[h] ? v : h
  return { edge, offset: wrapDelta(st, sOnEdge(st, edge, start) - s) }
}

/** 拖曳中的中心：游標在目前那一邊的投影減掉抓取偏移。 */
export function dragCenter(st: Stage, edge: Edge, p: Pt, offset: number): number {
  return mod(sOnEdge(st, edge, p) - offset, st.P)
}

/** 預測 PREDICT_S 秒後的游標位置（限制在舞台內）。 */
export function predictPoint(st: Stage, p: Pt, v: Pt): Pt {
  return { x: clamp(p.x + v.x * PREDICT_S, 0, st.W), y: clamp(p.y + v.y * PREDICT_S, 0, st.H) }
}

/** 速度上限：60 fps 下每幀沿周長最多 120 px（場景 6），目標跳很遠時也是連續滑過去。 */
export const MAX_SPEED = 7200

/**
 * 臨界阻尼彈簧一步（解析解，任何 dt 都準確不回彈）；每幀位移與速度都有上限。`st` 有值時沿周長最短方向（環狀）。
 * dt ≤ 0（rAF 時間戳早於觸發動畫的事件）不移動：負的 dt 會讓位移上限反過來，往正方向先跳一段（場景 6）。
 */
export function springStep(st: Stage | null, x: number, v: number, target: number, omega: number, dt: number): [number, number] {
  if (!(dt > 0)) return [x, v]
  const e0 = -(st ? wrapDelta(st, target - x) : target - x) // 目前位置相對目標的偏移
  const decay = Math.exp(-omega * dt)
  const k = v + omega * e0
  const e1 = (e0 + k * dt) * decay
  const v1 = (v - omega * k * dt) * decay
  const maxStep = MAX_SPEED * dt
  const step = clamp(e1 - e0, -maxStep, maxStep)
  const nx = x + step
  const nv = clamp(v1, -MAX_SPEED, MAX_SPEED)
  return [st ? mod(nx, st.P) : nx, nv]
}

export const centerToStart = (sc: number, len: number) => sc - (len - T) / 2

/** 一幀動畫要的狀態：中心與長度各自的目前值、目標與速度。 */
export interface NotchMotion {
  s: number
  sTarget: number
  v: number
  len: number
  lenTarget: number
  lenV: number
  dragging: boolean
}

/**
 * 一幀（場景 6／18／19）：中心與長度走同一個臨界阻尼彈簧（拖曳中 ω 30、其他 ω 20），所以伸縮時頭端每幀都不動。減少動態時
 * 不跑彈簧：任何目標直接到位、速度歸零。停下來時精確到位。回傳是否還在動（還在動才需要下一幀）。
 */
export function stepMotion(st: Stage, m: NotchMotion, dt: number, reduced: boolean): boolean {
  if (!reduced) {
    const omega = m.dragging ? OMEGA_DRAG : OMEGA_SETTLE
    ;[m.s, m.v] = springStep(st, m.s, m.v, m.sTarget, omega, dt)
    ;[m.len, m.lenV] = springStep(null, m.len, m.lenV, m.lenTarget, omega, dt)
    const moving =
      Math.abs(wrapDelta(st, m.sTarget - m.s)) > 0.05 || Math.abs(m.v) > 0.5 || Math.abs(m.lenTarget - m.len) > 0.05 || Math.abs(m.lenV) > 0.5
    if (moving) return true
  }
  m.s = m.sTarget
  m.len = m.lenTarget
  m.v = 0
  m.lenV = 0
  return false
}

/**
 * 還要不要下一個動畫 frame（場景 19）：還在動；或（非減少動態時）文字淡入中、跑馬燈捲動中。減少動態時文字直接不透明、
 * 跑馬燈不捲動，所以只要停下來就零 frame。
 */
export function notchFrameNeeded(moving: boolean, fadeElapsedMs: number, marquee: boolean, reduced: boolean): boolean {
  if (moving) return true
  if (reduced) return false
  return fadeElapsedMs < TEXT_FADE_MS + 20 || marquee
}

/** 中心正好在角落（以角落為中心的 L 形）就回那個角落。 */
export function cornerAt(st: Stage, sc: number): Corner | null {
  for (const [corner, cs] of Object.entries(st.corners) as [Corner, number][]) {
    if (Math.abs(wrapDelta(st, sc - cs)) < 0.5) return corner
  }
  return null
}

/** 放開吸附（場景 7）：角落 L 形；跨角但不夠近 → 滑回主導邊留 EDGE_GAP；其他留原地。 */
export function settleCenter(st: Stage, sc: number, len: number): number {
  const lc = len - T
  let best: { cs: number; d: number } | null = null
  for (const cs of Object.values(st.corners)) {
    const d = wrapDelta(st, sc - cs)
    if (!best || Math.abs(d) < Math.abs(best.d)) best = { cs, d }
  }
  if (!best) return sc
  if (Math.abs(best.d) < lc * CORNER_SNAP_RATIO) return best.cs
  if (Math.abs(best.d) < lc / 2 + EDGE_GAP) return mod(best.cs + Math.sign(best.d) * (lc / 2 + EDGE_GAP), st.P)
  return sc
}

/**
 * 放開（場景 7）：吸附位置與放開後的最終長度一起決定。停到角落用角落的靜止長度（補轉角弧長），否則用直邊的靜止長度；
 * (b) 的 EDGE_GAP 以這個最終長度計算。拖曳中用的長度（從角落開始拖時含轉角弧長）不參與。
 *
 * 例外（場景 14、INV-24）：拖曳中所有題目在別處結案、瀏海不再畫出時，放開用的文字（空佇列）和最後畫出來的不同；`drawn`
 * 給最後畫出來的長度與頭端，先以頭端為錨換成直邊長度再吸附，所以保存的頭端就在放開處，不偏掉長度差的一半。
 */
export function settleRelease(
  st: Stage,
  sc: number,
  straightLen: number,
  cornerLen: number,
  drawn?: { len: number; head: HeadEnd },
): { center: number; len: number; corner: boolean } {
  const base = drawn ? headAnchoredCenter(st, sc, drawn.len, straightLen, drawn.head) : sc
  const atCorner = settleCenter(st, base, cornerLen)
  if (cornerAt(st, atCorner)) return { center: atCorner, len: cornerLen, corner: true }
  const center = settleCenter(st, base, straightLen)
  if (cornerAt(st, center)) return { center, len: cornerLen, corner: true }
  return { center, len: straightLen, corner: false }
}

/** 長度改變時頭端不動的中心：ccw 頭在周長起點端（起點不動），cw 頭在終點端（終點不動）。 */
export function headAnchoredCenter(st: Stage, sc: number, oldLen: number, newLen: number, head: HeadEnd): number {
  const half = (newLen - oldLen) / 2
  return mod(head === 'ccw' ? sc + half : sc - half, st.P)
}

/**
 * 靜止時完整留在那條直邊上（場景 10b）：超出就整條往回移，只移必要的距離；`margin` 是端點離那條邊盡頭（螢幕邊）至少要留的
 * 距離——還原保存值用 0（剛好貼齊邊界），文字變長用 EDGE_GAP（`resizeCenter`）。放不下 margin 就盡量平分；比整條邊還長
 * （不會發生）就置中。
 */
export function keepOnEdge(st: Stage, sc: number, len: number, edge: Edge, margin = 0): number {
  const { base, span } = edgeSpan(st, edge)
  const half = (len - T) / 2
  const u = wrapDelta(st, sc - base)
  const m = (half + margin) * 2 > span ? Math.max(0, (span - half * 2) / 2) : margin
  return mod(base + (half * 2 > span ? span / 2 : clamp(u, half + m, span - half - m)), st.P)
}

/**
 * 文字長度改變（場景 10b）：頭端不動、只有尾端伸縮；停在角落的 L 形維持以角落為中心。頭端不動優先，但靜止時一定完整
 * 留在原本那條直邊上：尾端會超出那條邊時整條往回移，尾端離螢幕邊 EDGE_GAP（與放開吸附一致）。
 */
export function resizeCenter(st: Stage, sc: number, oldLen: number, newLen: number, head: HeadEnd): number {
  if (oldLen === newLen || cornerAt(st, sc)) return sc
  const edge = edgeOfS(st, sc)
  const anchored = headAnchoredCenter(st, sc, oldLen, newLen, head)
  const fits = keepOnEdge(st, anchored, newLen, edge)
  // 放得下就頭端不動；會超出那條邊時整條往回移，尾端跟放開吸附一樣離螢幕邊 EDGE_GAP（場景 10b、7）。
  return Math.abs(wrapDelta(st, fits - anchored)) < 1e-6 ? anchored : keepOnEdge(st, anchored, newLen, edge, EDGE_GAP)
}

/** hover 延長方向（場景 11）：一律往尾端（頭端永遠不動）；回傳周長正負號。空間不足時尾端沿周長轉過角落成 L 形。 */
export function growDirection(head: HeadEnd): 1 | -1 {
  return head === 'ccw' ? 1 : -1
}

/**
 * 從 hover 狀態開始拖曳（場景 4／11）：hover 立即收回且以頭端為錨（頭端不動、尾端縮回靜止長度），抓取偏移改以收回後的
 * 中心計算——游標到頭端的距離維持按下時的值，第一幀頭端不跳。`s`／`len` 是延長時（畫出來）的中心與長度；`edge` 是按下時
 * 游標底下那一臂的邊（`grabStart`），按在 hover 轉角出來的短臂上也一樣，拖動後頭端不跳。
 */
export function grabAfterCollapse(
  st: Stage,
  edge: Edge,
  p: Pt,
  s: number,
  len: number,
  restLen: number,
  head: HeadEnd,
): { center: number; offset: number } {
  const center = headAnchoredCenter(st, s, len, restLen, head)
  return { center, offset: wrapDelta(st, sOnEdge(st, edge, p) - center) }
}

/**
 * hover 延長／收回後的中心目標（場景 11）。以吸附後的目標（不是動畫中的位置）為基準：放開時游標還停在瀏海上、
 * 動畫還沒走完就開始 hover，也不會把角落 L 形推走。
 */
export function hoverTarget(st: Stage, m: { s: number; sTarget: number }, growDir: 1 | -1, extend: boolean): number {
  const shift = (growDir * HOVER_EXTRA) / 2
  return mod(m.sTarget + (extend ? shift : -shift), st.P)
}

/** 預設位置（場景 3）：主螢幕左側，頂端在選單列下緣再往下 8 px。回傳中心的周長座標。 */
export function defaultCenter(st: Stage, restLen: number, menubar: number): number {
  const lc = restLen - T
  // 左側周長往上增加：頂端（視覺）是 s 較大的那一端；帶的頂端 = 中心線頂端 - T/2。
  const topY = menubar + 8
  const centerTopY = topY + T / 2 // 頂端那個中心線點的 y
  const sTop = 2 * st.A + st.B + (st.H - T / 2 - centerTopY)
  return mod(sTop - lc / 2, st.P)
}

/** 延後原因碼（server `GuardState::defer_reasons`，依顯示優先序）。 */
export type DeferReason = 'screen_shared' | 'camera_on' | 'mic_on' | 'frontmost_fullscreen' | 'dnd' | 'quiet_hours'

/** `defer_reasons` 的回應：原因與勿擾、安靜時段的結束時刻（ISO）。 */
export interface DeferInfo {
  reasons: DeferReason[]
  dnd_until?: string | null
  quiet_until?: string | null
}

export interface RestState {
  /** asking＝最早一題已在強制詢問（forced；蓋板或主視窗後備正在顯示它），文字同 pending（場景 10d）。 */
  kind: 'pending' | 'warning' | 'deferred' | 'asking'
  count: number
  remainingMs: number
  /** 到期被延後時擋住的原因；還沒讀到就是 undefined。 */
  defer?: DeferInfo | null
  /** 最早一題的警告長度（force_at − warn_at）；倒數分鐘依它補位（場景 10、FR-03）。 */
  warnMs?: number
}

/** 台北時間 HH:MM。 */
function taipeiClock(iso: string | null | undefined): string | null {
  const ms = iso ? Date.parse(iso) : NaN
  if (!Number.isFinite(ms)) return null
  return new Intl.DateTimeFormat('en-GB', { timeZone: 'Asia/Taipei', hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).format(ms)
}

/** 到期被延後時要說的話（場景 10c）：只說優先序第一個原因；讀不到或沒有原因就是「馬上跳出來」。 */
function deferredText(defer: DeferInfo | null | undefined): string {
  const first = defer?.reasons[0]
  const at = (iso: string | null | undefined, what: string) => {
    const clock = taipeiClock(iso)
    return clock ? `時間到了，${clock} ${what}結束就跳出來` : `時間到了，${what}結束就跳出來`
  }
  switch (first) {
    case 'screen_shared':
      return '時間到了，分享螢幕結束就跳出來'
    case 'camera_on':
      return '時間到了，鏡頭關掉就跳出來'
    case 'mic_on':
      return '時間到了，麥克風關掉就跳出來'
    case 'frontmost_fullscreen':
      return '時間到了，離開全螢幕就跳出來'
    case 'dnd':
      return at(defer?.dnd_until, '勿擾')
    case 'quiet_hours':
      return at(defer?.quiet_until, '安靜時段')
    default:
      return '時間到了，馬上跳出來'
  }
}

const SPOKEN: Record<DeferReason, string> = {
  screen_shared: '分享螢幕',
  camera_on: '鏡頭',
  mic_on: '麥克風',
  frontmost_fullscreen: '全螢幕',
  dnd: '勿擾',
  quiet_hours: '安靜時段',
}

/** 無障礙名稱用：列出全部延後原因（「分享螢幕、麥克風開著」）；沒有原因回空字串。 */
export function deferReasonsSpoken(defer: DeferInfo | null | undefined): string {
  const reasons = defer?.reasons ?? []
  return reasons.length === 0 ? '' : `${reasons.map((r) => SPOKEN[r]).join('、')}開著`
}

/**
 * 倒數分鐘的位數（場景 10、PRD FR-03，QA 第 5 輪）：這一題的警告長度最多顯示幾分鐘，就補到幾位——預設 3 分鐘是 1 位
 * （「2:59」），10 分鐘以上是 2 位（「10:00」之後是「09:59」）。沒有警告長度就 1 位。
 */
export function warnMinuteDigits(warnMs?: number): number {
  if (warnMs === undefined || !Number.isFinite(warnMs) || warnMs <= 0) return 1
  return String(Math.floor(Math.ceil(warnMs / 1000) / 60)).length
}

/** 靜止文字（場景 10／10c／10d）。warning 的分鐘補位到警告長度的位數，量寬與繪製用同一串字，倒數換字不改變長度。 */
export function restText(state: RestState): string {
  if (state.kind === 'warning') {
    const total = Math.max(0, Math.ceil(state.remainingMs / 1000))
    const minutes = String(Math.floor(total / 60)).padStart(warnMinuteDigits(state.warnMs), '0')
    return `${minutes}:${String(total % 60).padStart(2, '0')} 後強制詢問`
  }
  if (state.kind === 'deferred') return deferredText(state.defer)
  return state.count <= 1 ? '你有一則待處理通知' : `你有 ${state.count} 則待處理通知`
}

/** 跑馬燈（場景 11／12）：依佇列串起題目；隱私 guard 命中只顯示預設文字。 */
export function marqueeText(lines: string[], privacy: boolean, rest: string): string {
  if (privacy || lines.length === 0) return rest
  return lines.join('　·　')
}

/** 文字中心線在轉角走半徑 RC_TEXT 的 1/4 圓弧，比直角少走 2r − πr/2（約 4.3 px）。 */
export const CORNER_ARC_LOSS = 2 * RC_TEXT - (Math.PI * RC_TEXT) / 2

/**
 * 靜止長度（場景 10b）：文字寬＋頭端圖示與留白。停在角落時補上轉角圓弧少走的長度，最後一個字和直邊一樣完整顯示、
 * 不落進尾端淡出區。
 */
export function restLength(text: string, measure: (s: string) => number, corner = false): number {
  const w = tokenize(text, measure).reduce((n, t) => n + t.adv, 0)
  return Math.ceil(w + T / 2 + ICON / 2 + 7 + 12 + (corner ? CORNER_ARC_LOSS : 0))
}

/**
 * 保存的位置（場景 3／3b）：角落；直邊的頭端（圖示端）沿邊比例 `head`；或舊格式（QA 第 2 輪以前）的中心沿邊比例 `ratio`，
 * 只讀相容，下次保存改寫成 `head`。比例都是周長方向沿那一邊的 0..1。
 */
export type SavedPosition = { corner: Corner } | { edge: Edge; head: number } | { edge: Edge; ratio: number }

/** 某一邊在周長上的起點與長度。 */
function edgeSpan(st: Stage, edge: Edge): { base: number; span: number } {
  const base = edge === 'top' ? 0 : edge === 'right' ? st.A : edge === 'bottom' ? st.A + st.B : 2 * st.A + st.B
  return { base, span: edge === 'top' || edge === 'bottom' ? st.A : st.B }
}

/**
 * 持久化（場景 3）：角落記角落；直邊記頭端沿邊比例（需要當下長度 `len` 定位頭端）。記頭端而不是中心：還原時文字長度
 * 不同（例如保存後進入 warning），圖示端也回到原位，只有尾端依當下長度伸縮；換解析度後頭端在同一相對位置。
 */
export function savePosition(st: Stage, sc: number, len: number): SavedPosition {
  const corner = cornerAt(st, sc)
  if (corner) return { corner }
  const edge = edgeOfS(st, sc)
  const { base, span } = edgeSpan(st, edge)
  const half = (len - T) / 2
  const head = mod(sc - base, st.P) + (edgeHead(edge) === 'ccw' ? -half : half)
  return { edge, head: clamp(head / span, 0, 1) }
}

const CORNERS: readonly Corner[] = ['tl', 'tr', 'br', 'bl']
const EDGES: readonly Edge[] = ['top', 'right', 'bottom', 'left']

const isRatio = (r: unknown): r is number => typeof r === 'number' && Number.isFinite(r) && r >= 0 && r <= 1

/**
 * 保存值驗證（場景 3b）：只接受已知角落、已知邊＋0..1 的頭端比例，或舊格式已知邊＋0..1 的中心比例；其餘一律 null。
 * 有 `head` 欄位就只看它（壞掉就整筆不用，不退回舊格式）。
 */
export function parseSavedPosition(v: unknown): SavedPosition | null {
  if (!v || typeof v !== 'object' || Array.isArray(v)) return null
  const o = v as Record<string, unknown>
  if (typeof o.corner === 'string') return (CORNERS as readonly string[]).includes(o.corner) ? { corner: o.corner as Corner } : null
  if (typeof o.edge === 'string' && (EDGES as readonly string[]).includes(o.edge)) {
    const edge = o.edge as Edge
    if (o.head !== undefined) return isRatio(o.head) ? { edge, head: o.head } : null
    if (isRatio(o.ratio)) return { edge, ratio: o.ratio }
  }
  return null
}

/**
 * 網頁記憶體中的保存位置（場景 3b／3e，QA 第 5 輪）。`display`：舞台目前的螢幕（`displayKey`）；`saved`：這台螢幕的上次位置
 * （啟動或換螢幕時讀檔、放開時立即更新）；`resetGen`：設定頁「回到預設位置」的世代，延後的寫檔排程時記下、寫檔前比對。
 */
export interface SavedMemory {
  display: string | null
  saved: unknown
  resetGen: number
}
export const SAVED_MEMORY_INIT: SavedMemory = { display: null, saved: null, resetGen: 0 }

/**
 * 重讀 `edge_info`（舞台重新貼合、螢幕配置改變、睡眠喚醒）：換了螢幕才換成那台螢幕的保存值。同一台螢幕重讀一律不動記憶體——
 * 讀到 null 不代表被重設（放開後延後 400 ms 的寫檔可能還沒發生，或寫檔失敗）；重設只看 `kunsu://edge_position_reset`。
 */
export function savedAfterRead(m: SavedMemory, display: string, incoming: unknown): SavedMemory {
  return m.display === display ? m : { ...m, display, saved: incoming ?? null }
}

/** 放開（場景 3b）：記憶體立即更新；寫檔延後，排程時記下 `resetGen`。 */
export function savedAfterRelease(m: SavedMemory, pos: SavedPosition): SavedMemory {
  return { ...m, saved: pos }
}

/** 設定頁「回到預設位置」（`kunsu://edge_position_reset`，場景 3e）：清掉記憶體位置、世代加一，之前排定的寫檔作廢。 */
export function savedAfterReset(m: SavedMemory): SavedMemory {
  return { ...m, saved: null, resetGen: m.resetGen + 1 }
}

/** 延後的寫檔還能寫嗎：排程之後沒有按過「回到預設位置」。 */
export function saveStillCurrent(m: SavedMemory, gen: number): boolean {
  return m.resetGen === gen
}

/** 還原中心：保存值合格就用它，否則預設位置（左側、頂端在選單列下 8 px）。永遠回有限數字。 */
export function restoreCenter(st: Stage, saved: unknown, restLen: number, menubar: number): number {
  const pos = parseSavedPosition(saved)
  return pos ? restorePosition(st, pos, restLen) : defaultCenter(st, restLen, menubar)
}

/**
 * 保存的位置 → 中心。頭端格式依當下長度 `len` 從頭端往尾端展開（頭端不動）；放不下時整條往回移到尾端剛好貼齊邊界——只保證
 * 完整留在那條直邊上（場景 10b），不像文字變長（`resizeCenter`）那樣離螢幕邊 24 px。
 */
export function restorePosition(st: Stage, pos: SavedPosition, len = T): number {
  if ('corner' in pos) return st.corners[pos.corner]
  const { base, span } = edgeSpan(st, pos.edge)
  if ('head' in pos) {
    const half = (len - T) / 2
    const head = base + clamp(pos.head, 0, 1) * span
    return keepOnEdge(st, mod(edgeHead(pos.edge) === 'ccw' ? head + half : head - half, st.P), len, pos.edge)
  }
  return keepOnEdge(st, mod(base + clamp(pos.ratio, 0, 1) * span, st.P), len, pos.edge)
}

