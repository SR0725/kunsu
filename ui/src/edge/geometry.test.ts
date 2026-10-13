// 邊緣瀏海幾何（BDD edge-notch AC-EN-02～14、18；移植自 docs/prototypes/edge-notch.html）。
import { describe, expect, it } from 'vitest'
import {
  CORNER_ARC_LOSS,
  CORNER_SNAP_RATIO,
  EDGE_GAP,
  HOVER_EXTRA,
  HYSTERESIS,
  OMEGA_LENGTH,
  OMEGA_SETTLE,
  RC_TEXT,
  T,
  bandGeometry,
  capRadius,
  centerToStart,
  defaultCenter,
  dragCenter,
  grabAfterCollapse,
  grabByDirection,
  grabStart,
  growDirection,
  headAnchoredCenter,
  hoverTarget,
  makeStage,
  marqueeText,
  nearestEdge,
  notchFrameNeeded,
  parseSavedPosition,
  pieces,
  pointOnCenterline,
  preferredHead,
  resizeCenter,
  restLength,
  restoreCenter,
  restText,
  sOnEdge,
  deferReasonsSpoken,
  type DeferInfo,
  restorePosition,
  savePosition,
  settleCenter,
  settleRelease,
  springStep,
  stepMotion,
  textPath,
  tokenize,
  wrapDelta,
  type Corner,
  type Edge,
  type HeadEnd,
  type NotchMotion,
  type Pt,
} from './geometry'

const st = makeStage(1440, 900)
const measure = (s: string) => s.length * 7

type P2 = [number, number]
/** SVG d（M x yL x y…Z）→ 多邊形頂點。 */
const parseD = (d: string): P2[] =>
  d
    .replace(/[MZ]/g, ' ')
    .split('L')
    .map((p) => p.trim().split(/\s+/).map(Number) as P2)
/** 原生層收到的扁平陣列 → 多邊形頂點。 */
const flatPts = (f: number[]): P2[] => {
  const out: P2[] = []
  for (let i = 0; i + 1 < f.length; i += 2) out.push([f[i], f[i + 1]])
  return out
}
/** even-odd 點在多邊形內（與原生層 edge_hit 同一規則）。 */
function inPoly(pts: P2[], x: number, y: number): boolean {
  let c = false
  for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
    const [xi, yi] = pts[i]
    const [xj, yj] = pts[j]
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) c = !c
  }
  return c
}
/** 某一列（y）被多邊形聯集覆蓋、且在螢幕內的區間總長。 */
function rowCover(polys: P2[][], y: number): number {
  const spans: P2[] = []
  for (const pts of polys) {
    const xs: number[] = []
    for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
      const [xi, yi] = pts[i]
      const [xj, yj] = pts[j]
      if (yi > y !== yj > y) xs.push(((xj - xi) * (y - yi)) / (yj - yi) + xi)
    }
    xs.sort((a, b) => a - b)
    for (let k = 0; k + 1 < xs.length; k += 2) {
      const a = Math.max(0, xs[k])
      const b = Math.min(st.W, xs[k + 1])
      if (b > a) spans.push([a, b])
    }
  }
  spans.sort((a, b) => a[0] - b[0])
  let total = 0
  let cur: P2 | null = null
  for (const sp of spans) {
    if (!cur || sp[0] > cur[1]) {
      if (cur) total += cur[1] - cur[0]
      cur = [sp[0], sp[1]]
    } else cur[1] = Math.max(cur[1], sp[1])
  }
  if (cur) total += cur[1] - cur[0]
  return total
}
/** 畫出來的黑色（螢幕內）聯集面積，每列取中線。 */
function blackArea(s0: number, len: number) {
  const polys = bandGeometry(st, s0, len).shapes.map(parseD)
  let area = 0
  for (let y = 0; y < st.H; y++) area += rowCover(polys, y + 0.5)
  return area
}

describe('周長與分段', () => {
  it('直邊一段、跨角兩段，總長不變', () => {
    expect(pieces(st, 100, 200).map((p) => p.edge)).toEqual(['top'])
    const across = pieces(st, st.corners.tr - 50, 200)
    expect(across.map((p) => p.edge)).toEqual(['top', 'right'])
    expect(across.reduce((n, p) => n + (p.s1 - p.s0), 0)).toBeCloseTo(200)
    expect(pieces(st, st.P - 30, 100).map((p) => p.edge)).toEqual(['left', 'top'])
  })

  it('中心線貼齊邊緣內縮 T/2', () => {
    expect(pointOnCenterline(st, 0)).toEqual({ x: T / 2, y: T / 2 })
    expect(pointOnCenterline(st, st.corners.br)).toEqual({ x: 1440 - T / 2, y: 900 - T / 2 })
  })
})

describe('帶狀幾何', () => {
  it('左側直條貼齊 x=0、厚度 32', () => {
    const geo = bandGeometry(st, st.corners.bl + 200, 160)
    expect(geo.pieces).toHaveLength(1)
    const polys = geo.shapes.map(parseD)
    const mid = (geo.pieces[0].u0 + geo.pieces[0].u1) / 2
    // 中段那一列：黑色剛好是 [0, 32]
    expect(rowCover(polys, mid)).toBeCloseTo(T, 5)
    expect(polys.some((p) => inPoly(p, 0.5, mid))).toBe(true)
    expect(polys.some((p) => inPoly(p, T + 0.5, mid))).toBe(false)
  })

  it('沿周長每 1 px 取樣跨過角落，畫出的面積連續（不跳形）', () => {
    const len = 180
    let prev = blackArea(st.corners.tr - len - 20, len)
    for (let s = st.corners.tr - len - 19; s <= st.corners.tr + 20; s++) {
      const area = blackArea(s, len)
      expect(Math.abs(area - prev)).toBeLessThanOrEqual(3 * T)
      prev = area
    }
  })

  it('半徑一律取 min(名目值, 間隙)', () => {
    expect(capRadius(12, 100, 100)).toBe(12)
    expect(capRadius(12, 5, 100)).toBe(5)
    expect(capRadius(12, 100, 0.5)).toBe(0.5)
    expect(capRadius(12, -3, 100)).toBe(0)
  })
})

describe('頭端與文字方向', () => {
  const head = (s0: number, len: number) => preferredHead(pieces(st, s0, len))
  it('頭端偏好表（場景 8）', () => {
    expect(head(st.corners.bl + 200, 150)).toBe('cw') // 左
    expect(head(200, 150)).toBe('ccw') // 上
    expect(head(st.corners.tr + 200, 150)).toBe('ccw') // 右
    expect(head(st.corners.br + 200, 150)).toBe('cw') // 下
    expect(head(st.P - 60, 150)).toBe('cw') // 左上 L
    expect(head(st.corners.tr - 60, 150)).toBe('ccw') // 右上 L
    expect(head(st.corners.bl - 60, 150)).toBe('cw') // 左下 L
    expect(head(st.corners.br - 60, 150)).toBe('ccw') // 右下 L
  })

  it('左側由上往下讀、上方由左往右讀', () => {
    const left = pieces(st, st.corners.bl + 200, 150)
    const tp = textPath(st, left, 'cw')
    expect(tp.at(40).y).toBeGreaterThan(tp.at(10).y)
    const top = pieces(st, 200, 150)
    const tp2 = textPath(st, top, 'ccw')
    expect(tp2.at(40).x).toBeGreaterThan(tp2.at(10).x)
    expect(tp2.total).toBeCloseTo(150, 0)
  })

  it('中文逐字、英數成詞、空白獨立', () => {
    const toks = tokenize('你有 2:41 後', measure)
    expect(toks.map((t) => [t.s, t.kind])).toEqual([
      ['你', 'cjk'],
      ['有', 'cjk'],
      [' ', 'space'],
      ['2:41', 'latin'],
      [' ', 'space'],
      ['後', 'cjk'],
    ])
  })
})

describe('拖曳力學', () => {
  it('往下拖帶一點水平漂移仍留在左側；靠近上方明確往右才換到上方（遲滯）', () => {
    expect(nearestEdge(st, { x: 30, y: 500 }, 'left')).toBe('left')
    expect(nearestEdge(st, { x: 60, y: 40 }, 'left')).toBe('left')
    expect(nearestEdge(st, { x: 60 + HYSTERESIS, y: 10 }, 'left')).toBe('top')
    expect(nearestEdge(st, { x: 1420, y: 450 }, 'top')).toBe('right')
  })

  it('彈簧：目標跳半圈時每幀位移 ≤ 120 px；放開後 380 ms 內穩定', () => {
    let x = 0
    let v = 0
    const target = st.P / 2
    for (let i = 0; i < 60; i++) {
      const [nx, nv] = springStep(st, x, v, target, 30, 1 / 60)
      expect(Math.abs(nx - x)).toBeLessThanOrEqual(120)
      x = nx
      v = nv
    }
    x = 0
    v = 0
    for (let t = 0; t < 0.38; t += 1 / 60) [x, v] = springStep(st, x, v, 200, 20, 1 / 60)
    expect(Math.abs(x - 200)).toBeLessThan(1)
  })

  it('放開吸附：角落 L 形、跨角滑回主導邊留 24 px、其他留原地', () => {
    const len = 200
    const lc = len - T
    const corner = st.corners.tr
    expect(settleCenter(st, corner + lc * CORNER_SNAP_RATIO * 0.5, len)).toBe(corner)
    const near = settleCenter(st, corner - (lc * CORNER_SNAP_RATIO + 10), len)
    expect(near).toBeCloseTo(corner - (lc / 2 + EDGE_GAP))
    expect(settleCenter(st, 400, len)).toBe(400)
  })

  it('hover 一律沿尾端延長；尾端空間不足時轉過角落成 L，頭端不動（左側靠下）', () => {
    // 2026-10-09 決定：圖示（頭端）永遠不動，不再「空間不足改由頭端延長」。
    expect(growDirection('ccw')).toBe(1) // 頭在周長起點端 → 尾端往周長增加方向
    expect(growDirection('cw')).toBe(-1)
    const rest = 180
    const ext = rest + HOVER_EXTRA
    /** 頭端（左側是頂端＝周長終點端）的周長座標。 */
    const headS = (sc: number, len: number) => sc + (len - T) / 2
    const mid = defaultCenter(st, rest, 37)
    const midExt = hoverTarget(st, { s: mid, sTarget: mid }, growDirection('cw'), true)
    expect(pieces(st, centerToStart(midExt, ext), ext - T).map((p) => p.edge)).toEqual(['left'])
    expect(headS(midExt, ext)).toBeCloseTo(headS(mid, rest), 6)
    // 尾端（底端）離左下角只剩 60 px：尾端轉到底邊，成 L 形；頂端（圖示）不動
    const low = st.corners.bl + 60 + (rest - T) / 2
    expect(preferredHead(pieces(st, centerToStart(low, rest), rest - T))).toBe('cw')
    const lowExt = hoverTarget(st, { s: low, sTarget: low }, growDirection('cw'), true)
    expect(pieces(st, centerToStart(lowExt, ext), ext - T).map((p) => p.edge)).toEqual(['bottom', 'left'])
    expect(headS(lowExt, ext)).toBeCloseTo(headS(low, rest), 6)
    // 收回以頭端為錨：回到原本的靜止中心
    expect(hoverTarget(st, { s: lowExt, sTarget: lowExt }, growDirection('cw'), false)).toBeCloseTo(low, 6)
  })

  it('hover 以吸附後的目標為基準：放開時游標還在瀏海上也不會把角落 L 形推走', async () => {
    const { hoverTarget } = await import('./geometry')
    const corner = st.corners.tl
    // 動畫還在路上（s 離角落 100px），hover 延長後收回，目標仍是角落。
    const extended = hoverTarget(st, { s: corner + 100, sTarget: corner }, -1, true)
    const back = hoverTarget(st, { s: corner + 60, sTarget: extended }, -1, false)
    expect(back).toBeCloseTo(corner)
  })

  it('預設在主螢幕左側、頂端在選單列下 8 px', () => {
    const rest = 180
    const sc = defaultCenter(st, rest, 37)
    const geo = bandGeometry(st, centerToStart(sc, rest), rest - T)
    expect(geo.pieces.map((p) => p.edge)).toEqual(['left'])
    expect(geo.pieces[0].u0).toBeCloseTo(37 + 8, 5)
  })
})

describe('文字與持久化', () => {
  it('靜止文字依狀態（場景 10）', () => {
    expect(restText({ kind: 'pending', count: 1, remainingMs: 600_000 })).toBe('你有一則待處理通知')
    expect(restText({ kind: 'pending', count: 3, remainingMs: 600_000 })).toBe('你有 3 則待處理通知')
    expect(restText({ kind: 'warning', count: 1, remainingMs: 161_000 })).toBe('2:41 後強制詢問')
    expect(restText({ kind: 'deferred', count: 2, remainingMs: 0 })).toBe('時間到了，馬上跳出來')
  })

  it('到期被延後時說出擋住的原因，只顯示優先序第一個（場景 10c）', () => {
    const deferred = (defer: DeferInfo) => restText({ kind: 'deferred', count: 1, remainingMs: 0, defer })
    expect(deferred({ reasons: ['screen_shared', 'mic_on'] })).toBe('時間到了，分享螢幕結束就跳出來')
    expect(deferred({ reasons: ['camera_on'] })).toBe('時間到了，鏡頭關掉就跳出來')
    expect(deferred({ reasons: ['mic_on'] })).toBe('時間到了，麥克風關掉就跳出來')
    expect(deferred({ reasons: ['frontmost_fullscreen'] })).toBe('時間到了，離開全螢幕就跳出來')
    // 台北時間：2026-10-09T03:30Z = 11:30；2026-10-10T01:00Z = 09:00。
    expect(deferred({ reasons: ['dnd'], dnd_until: '2026-10-09T03:30:00Z' })).toBe('時間到了，11:30 勿擾結束就跳出來')
    expect(deferred({ reasons: ['quiet_hours'], quiet_until: '2026-10-10T01:00:00Z' })).toBe('時間到了，09:00 安靜時段結束就跳出來')
    expect(deferred({ reasons: [] })).toBe('時間到了，馬上跳出來')
    // 沒帶時刻時不編時間。
    expect(deferred({ reasons: ['dnd'] })).toBe('時間到了，勿擾結束就跳出來')
  })

  it('無障礙名稱列出全部延後原因（場景 10c）', () => {
    expect(deferReasonsSpoken({ reasons: ['screen_shared', 'mic_on'] })).toBe('分享螢幕、麥克風開著')
    expect(deferReasonsSpoken({ reasons: [] })).toBe('')
  })

  it('跑馬燈依佇列串起題目；隱私 guard 命中只顯示預設文字（場景 12）', () => {
    const lines = ['要不要加碼？', '今天的日記寫了嗎？']
    expect(marqueeText(lines, false, '你有 2 則待處理通知')).toBe('要不要加碼？　·　今天的日記寫了嗎？')
    expect(marqueeText(lines, true, '你有 2 則待處理通知')).toBe('你有 2 則待處理通知')
  })

  it('位置以角落或邊＋頭端沿邊比例保存，換解析度後頭端還原到同一相對位置', () => {
    const big = makeStage(2560, 1440)
    const sc = st.corners.bl + 300
    const pos = savePosition(st, sc, 180)
    expect(pos).toEqual({ edge: 'left', head: expect.any(Number) })
    const restored = restorePosition(big, pos, 180)
    // 左側頭端是頂端（周長終點端）
    const p0 = pointOnCenterline(st, sc + (180 - T) / 2)
    const p1 = pointOnCenterline(big, restored + (180 - T) / 2)
    expect(p1.x).toBeCloseTo(T / 2)
    expect((p1.y - T / 2) / big.B).toBeCloseTo((p0.y - T / 2) / st.B, 6)
    expect(restorePosition(big, savePosition(st, st.corners.tr, 180), 180)).toBe(big.corners.tr)
  })
})

// ---------------------------------------------------------------- QA 第 1 輪（2026-10-09）

describe('AC-EN-03 保存值驗證', () => {
  it('壞掉的保存值一律退回預設位置，位置永遠是有限數字', () => {
    const bads: unknown[] = [
      { corner: 'xx' },
      { edge: 'middle', ratio: 0.3 },
      { edge: 'left', ratio: Number.NaN },
      { edge: 'left', ratio: 2 },
      { edge: 'left', ratio: -0.1 },
      { edge: 'left' },
      { edge: 'left', head: 2 },
      { edge: 'left', head: Number.NaN },
      { edge: 'left', head: '0.3', ratio: 0.3 },
      { ratio: 0.5 },
      { head: 0.5 },
      'left',
      42,
      null,
      undefined,
      [],
      {},
    ]
    const fallback = defaultCenter(st, 180, 37)
    for (const bad of bads) {
      expect(parseSavedPosition(bad), JSON.stringify(bad)).toBeNull()
      const sc = restoreCenter(st, bad, 180, 37)
      expect(Number.isFinite(sc), JSON.stringify(bad)).toBe(true)
      expect(sc).toBeCloseTo(fallback)
    }
    expect(parseSavedPosition({ corner: 'tr' })).toEqual({ corner: 'tr' })
    expect(parseSavedPosition({ edge: 'left', ratio: 0.25 })).toEqual({ edge: 'left', ratio: 0.25 })
    expect(restoreCenter(st, { corner: 'tr' }, 180, 37)).toBe(st.corners.tr)
    const pos = savePosition(st, st.corners.bl + 300, 180)
    expect(restoreCenter(st, pos, 180, 37)).toBeCloseTo(st.corners.bl + 300)
  })
})

describe('AC-EN-04 抓取點', () => {
  it('L 形抓住中心不在的那一臂開始拖曳，第一幀不跳（四個角）', () => {
    const len = 200
    const lc = len - T
    for (const corner of ['tl', 'tr', 'br', 'bl'] as Corner[]) {
      const sc = st.corners[corner]
      const ps = pieces(st, centerToStart(sc, len), lc)
      expect(ps).toHaveLength(2)
      for (const pc of ps) {
        const p = pointOnCenterline(st, (pc.s0 + pc.s1) / 2)
        const g = grabStart(st, sc, p)
        expect(g.edge, `${corner} 抓 ${pc.edge} 臂`).toBe(pc.edge)
        // 還沒移動：拖曳算出的中心就是原本的中心
        expect(Math.abs(wrapDelta(st, dragCenter(st, g.edge, p, g.offset) - sc)), `${corner} 抓 ${pc.edge} 臂`).toBeLessThan(0.5)
        // 沿這一臂移 10 px：中心沿周長也只移 10 px
        const vertical = pc.edge === 'left' || pc.edge === 'right'
        const moved = vertical ? { x: p.x, y: p.y + 10 } : { x: p.x + 10, y: p.y }
        expect(Math.abs(wrapDelta(st, dragCenter(st, g.edge, moved, g.offset) - sc))).toBeCloseTo(10, 5)
      }
    }
  })
})

describe('AC-EN-06 放開彈簧', () => {
  it('放開 ω = 20：最遠吸附 120 px 加 1500 px/s 殘留速度，380 ms 內穩定', () => {
    expect(OMEGA_SETTLE).toBe(20)
    let x = 0
    let v = 1500
    for (let t = 0; t < 0.38 - 1e-9; t += 1 / 60) [x, v] = springStep(st, x, v, 120, OMEGA_SETTLE, 1 / 60)
    expect(Math.abs(x - 120)).toBeLessThan(1)
  })
})

describe('AC-EN-10 文字長度改變時頭端不動', () => {
  /** 單一直邊上那一段的兩端（u0＝低端：左側是頂端、上方是左端）。 */
  const ends = (sc: number, len: number) => {
    const geo = bandGeometry(st, centerToStart(sc, len), len - T)
    expect(geo.pieces).toHaveLength(1)
    return { lo: geo.pieces[0].u0, hi: geo.pieces[0].u1 }
  }
  it('左側預設位置：1 → 2 → 12 題頂端都在選單列下 8 px', () => {
    const menubar = 37
    const lens = [166, 170, 176, 166]
    let len = lens[0]
    let sc = defaultCenter(st, len, menubar)
    const head = preferredHead(pieces(st, centerToStart(sc, len), len - T))
    expect(head).toBe('cw')
    for (const next of lens) {
      sc = resizeCenter(st, sc, len, next, head)
      len = next
      expect(ends(sc, len).lo).toBeCloseTo(menubar + 8, 5)
    }
  })
  it('上方：左端不動', () => {
    let len = 166
    let sc = 400
    const head = preferredHead(pieces(st, centerToStart(sc, len), len - T))
    expect(head).toBe('ccw')
    const left = ends(sc, len).lo
    for (const next of [176, 190, 160]) {
      sc = resizeCenter(st, sc, len, next, head)
      len = next
      expect(ends(sc, len).lo).toBeCloseTo(left, 5)
    }
  })
  it('角落 L 形維持以角落為中心', () => {
    for (const corner of ['tl', 'tr', 'br', 'bl'] as Corner[]) {
      const sc = st.corners[corner]
      const head = preferredHead(pieces(st, centerToStart(sc, 166), 166 - T))
      expect(resizeCenter(st, sc, 166, 190, head)).toBe(sc)
    }
  })
})

describe('AC-EN-14 命中區＝畫出的形狀', () => {
  /** 舊命中矩形（主體 u0..u1 × 0..T）：用來確認測試真的涵蓋肩部。 */
  const insideBandRect = (pc: { edge: string; u0: number; u1: number }, x: number, y: number) => {
    const u = pc.edge === 'top' || pc.edge === 'bottom' ? x : y
    const n = pc.edge === 'top' ? y : pc.edge === 'bottom' ? st.H - y : pc.edge === 'left' ? x : st.W - x
    return u >= pc.u0 && u <= pc.u1 && n >= 0 && n <= T
  }
  const cases: [string, number, number][] = [
    ['左側直條', st.corners.bl + 300, 150],
    ['上方橫條', 300, 170],
    ['左上 L', st.corners.tl - 70, 140],
    ['右下 L', st.corners.br - 80, 160],
    ['跨角途中（半徑縮小）', st.corners.tr - 150, 160],
  ]
  for (const [name, s0, len] of cases) {
    it(`${name}：肩部看得到就點得到；端點圓角外點不到`, () => {
      const geo = bandGeometry(st, s0, len)
      const black = geo.shapes.map(parseD)
      const hit = geo.hit.map(flatPts)
      // 只看瀏海附近（螢幕內外接矩形外擴 4 px），逐像素中心取樣
      const all = black.flat().filter(([x, y]) => x >= 0 && x <= st.W && y >= 0 && y <= st.H)
      const x0 = Math.max(0, Math.floor(Math.min(...all.map((p) => p[0]))) - 4)
      const x1 = Math.min(st.W, Math.ceil(Math.max(...all.map((p) => p[0]))) + 4)
      const y0 = Math.max(0, Math.floor(Math.min(...all.map((p) => p[1]))) - 4)
      const y1 = Math.min(st.H, Math.ceil(Math.max(...all.map((p) => p[1]))) + 4)
      let blackNotHit = 0
      let hitNotBlack = 0
      let shoulderBlack = 0
      for (let x = x0 + 0.5; x < x1; x += 1) {
        for (let y = y0 + 0.5; y < y1; y += 1) {
          const b = black.some((p) => inPoly(p, x, y))
          const h = hit.some((p) => inPoly(p, x, y))
          if (b && !h) blackNotHit++
          if (h && !b) hitNotBlack++
          if (b && geo.pieces.every((pc) => !insideBandRect(pc, x, y))) shoulderBlack++
        }
      }
      expect(shoulderBlack, '測試要涵蓋主體矩形以外的黑色（肩部）').toBeGreaterThan(0)
      expect(blackNotHit, '看得到但會穿透的像素').toBe(0)
      expect(hitNotBlack, '看不到但會攔截的像素').toBe(0)
    })
  }

  it('回報給原生層的座標與 SVG path 同一組兩位小數的點', () => {
    for (const [, s0, len] of cases) {
      const geo = bandGeometry(st, s0, len)
      expect(geo.hit).toHaveLength(geo.shapes.length)
      geo.hit.forEach((flat, i) => {
        expect(flat.map((v) => v.toFixed(2))).toEqual(parseD(geo.shapes[i]).flat().map((v) => v.toFixed(2)))
      })
    }
  })
})

// ---------------------------------------------------------------- QA 第 2 輪（2026-10-09）

describe('AC-EN-08 英數字詞（QA 第 2 輪）', () => {
  it('金額、百分比、時間、網址片段各是一個英數字詞；空白與全形字把字詞切開', () => {
    const words = [
      'NT$5,940',
      'US$1,200.50',
      '-12.5%',
      '+15%',
      '23:30',
      '2:41',
      'example.com/r?a=1',
      'https://kunsu.local/q/42#top',
      'ROAS>3',
      'CPA<200',
      'A/B',
      '(beta)',
      'Café',
    ]
    for (const word of words) {
      expect(tokenize(word, measure).map((t) => [t.s, t.kind]), word).toEqual([[word, 'latin']])
    }
    expect(tokenize('日預算 NT$5,940→NT$8,000，約12%', measure).map((t) => [t.s, t.kind])).toEqual([
      ['日', 'cjk'],
      ['預', 'cjk'],
      ['算', 'cjk'],
      [' ', 'space'],
      ['NT$5,940', 'latin'],
      ['→', 'cjk'],
      ['NT$8,000', 'latin'],
      ['，', 'cjk'],
      ['約', 'cjk'],
      ['12%', 'latin'],
    ])
    // 全形英數與跑馬燈分隔點維持直立、全形寬
    expect(tokenize('ＡＢ', measure).every((t) => t.kind === 'cjk')).toBe(true)
    expect(tokenize('甲　·　乙', measure).find((t) => t.s === '·')?.kind).toBe('cjk')
  })
})

describe('AC-EN-03 頭端保存（QA 第 2 輪）', () => {
  /** 頭端（圖示端）在中心線上的點：上方／右側是周長起點端，下方／左側是終點端。 */
  const headPoint = (stage: ReturnType<typeof makeStage>, sc: number, len: number) => {
    const start = centerToStart(sc, len)
    const head = preferredHead(pieces(stage, start, len - T))
    return pointOnCenterline(stage, head === 'ccw' ? start : start + len - T)
  }
  it('直邊保存頭端：換解析度、文字長度和保存時不同，頭端都回到原位', () => {
    const big = makeStage(2560, 1440)
    const cases: [string, number][] = [
      ['左側', st.corners.bl + 300],
      ['上方', 400],
      ['右側', st.corners.tr + 250],
      ['下方', st.corners.br + 500],
    ]
    for (const [name, sc] of cases) {
      const pos = savePosition(st, sc, 166)
      expect(pos, name).toEqual({ edge: expect.any(String), head: expect.any(Number) })
      const h0 = headPoint(st, sc, 166)
      // 同一螢幕、文字變短（進入 warning）後重啟
      const h1 = headPoint(st, restoreCenter(st, pos, 143, 37), 143)
      expect(Math.hypot(h1.x - h0.x, h1.y - h0.y), name).toBeLessThan(0.5)
      // 換解析度：頭端在同一相對位置
      const h2 = headPoint(big, restoreCenter(big, pos, 190, 37), 190)
      const rel = (stage: ReturnType<typeof makeStage>, p: { x: number; y: number }) =>
        name === '左側' || name === '右側' ? (p.y - T / 2) / stage.B : (p.x - T / 2) / stage.A
      expect(rel(big, h2), name).toBeCloseTo(rel(st, h0), 6)
    }
  })

  it('舊格式中心比例照樣讀得到；頭端格式優先，頭端值壞就整筆不用', () => {
    expect(parseSavedPosition({ edge: 'left', ratio: 0.25 })).toEqual({ edge: 'left', ratio: 0.25 })
    expect(restoreCenter(st, { edge: 'left', ratio: 0.25 }, 180, 37)).toBeCloseTo(2 * st.A + st.B + 0.25 * st.B)
    expect(parseSavedPosition({ edge: 'top', head: 0.4 })).toEqual({ edge: 'top', head: 0.4 })
    expect(parseSavedPosition({ edge: 'top', head: 0.4, ratio: 0.9 })).toEqual({ edge: 'top', head: 0.4 })
    expect(parseSavedPosition({ edge: 'top', head: 1.2, ratio: 0.9 })).toBeNull()
    // 頭端格式：上方頭端在左端 → 中心 = 頭端 + (len − T)/2
    expect(restoreCenter(st, { edge: 'top', head: 0.4 }, 180, 37)).toBeCloseTo(0.4 * st.A + (180 - T) / 2)
  })
})

describe('AC-EN-10 角落靜止長度（QA 第 2 輪）', () => {
  it('停在角落時靜止長度補上轉角弧長 2r − πr/2', () => {
    expect(CORNER_ARC_LOSS).toBeCloseTo(2 * RC_TEXT - (Math.PI * RC_TEXT) / 2, 9)
    for (const text of ['你有一則待處理通知', '2:41 後強制詢問', '你有 12 則待處理通知', '時間到了，馬上跳出來']) {
      const straight = restLength(text, measure)
      const corner = restLength(text, measure, true)
      expect(corner - straight, text).toBeGreaterThanOrEqual(Math.floor(CORNER_ARC_LOSS))
      expect(corner - straight, text).toBeLessThanOrEqual(Math.ceil(CORNER_ARC_LOSS))
    }
  })
})

describe('AC-EN-11 伸縮時頭端不動（QA 第 2 輪）', () => {
  it('hover 伸縮與文字長度改變的動畫中，中心與長度走同一個彈簧', () => {
    expect(OMEGA_LENGTH).toBe(OMEGA_SETTLE)
  })
})

// ---------------------------------------------------------------- QA 第 3 輪（2026-10-09）

/** 頭端在中心線上的周長座標：ccw 在起點端、cw 在終點端。 */
const headOf = (sc: number, len: number, head: HeadEnd) => (head === 'ccw' ? centerToStart(sc, len) : centerToStart(sc, len) + len - T)

describe('AC-EN-04 從 hover 開始拖曳（QA 第 3 輪）', () => {
  it('從 hover 狀態開始拖曳：hover 以頭端為錨收回，抓取偏移以收回後的位置計算，第一幀頭端不跳（QA 第 3 輪）', () => {
    const rest = 166
    const cases: [string, number][] = [
      ['左側預設位置', defaultCenter(st, rest, 37)],
      ['上方', 400],
      ['右側', st.corners.tr + 300],
      ['左側靠下（hover 尾端轉角成 L）', st.corners.bl + 50 + (rest - T) / 2],
    ]
    for (const [name, sc] of cases) {
      const head = preferredHead(pieces(st, centerToStart(sc, rest), rest - T))
      const dir = growDirection(head)
      const len = rest + HOVER_EXTRA
      const ext = hoverTarget(st, { s: sc, sTarget: sc }, dir, true)
      // 抓在原本靜止那段裡（離頭端 60 px）與延長出來的那段裡（離頭端 160 px）
      for (const along of [60, 160]) {
        const p = pointOnCenterline(st, headOf(ext, len, head) + dir * along)
        const grab = grabStart(st, ext, p)
        const c = grabAfterCollapse(st, grab.edge, p, ext, len, rest, head)
        // 收回後頭端與延長時同一點，也就是 hover 之前的頭端
        const h0 = pointOnCenterline(st, headOf(sc, rest, head))
        const h1 = pointOnCenterline(st, headOf(c.center, rest, head))
        expect(Math.hypot(h1.x - h0.x, h1.y - h0.y), `${name} ${along}`).toBeLessThan(0.5)
        expect(c.center, `${name} ${along}`).toBeCloseTo(headAnchoredCenter(st, ext, len, rest, head), 9)
        // 拖曳第一幀（游標還在按下處）：中心就是收回後的中心
        expect(Math.abs(wrapDelta(st, dragCenter(st, grab.edge, p, c.offset) - c.center)), `${name} ${along}`).toBeLessThan(0.5)
        // 沿這條邊移 10 px：頭端也只移 10 px（游標到頭端的距離維持按下時的值）
        const vertical = grab.edge === 'left' || grab.edge === 'right'
        const moved = vertical ? { x: p.x, y: p.y + 10 } : { x: p.x + 10, y: p.y }
        expect(Math.abs(wrapDelta(st, dragCenter(st, grab.edge, moved, c.offset) - c.center)), `${name} ${along}`).toBeCloseTo(10, 5)
      }
    }
  })
})

describe('AC-EN-07 放開吸附用最終長度（QA 第 3 輪）', () => {
  it('放開時吸附與最終長度一起決定：從角落 L 形拖到直邊，端點距垂直邊 24 px（QA 第 3 輪）', () => {
    const text = '你有一則待處理通知'
    const straight = restLength(text, measure)
    const corner = restLength(text, measure, true)
    expect(corner).toBeGreaterThan(straight)
    // (a) 靠近角落 → 角落 L 形，用角落長度
    expect(settleRelease(st, st.corners.tr + 5, straight, corner)).toEqual({ center: st.corners.tr, len: corner, corner: true })
    // (b) 跨角但不夠近 → 滑回主導邊，用直邊長度，端點距垂直邊 24 px（以最終長度計算）
    const lc = corner - T
    const top = settleRelease(st, st.corners.tr - lc * 0.4, straight, corner)
    expect(top).toMatchObject({ len: straight, corner: false })
    const end = pointOnCenterline(st, top.center + (straight - T) / 2)
    expect(st.W - (end.x + T / 2)).toBeCloseTo(EDGE_GAP, 6)
    const left = settleRelease(st, st.corners.bl + lc * 0.4, straight, corner)
    const bottom = pointOnCenterline(st, left.center - (straight - T) / 2)
    expect(st.H - (bottom.y + T / 2)).toBeCloseTo(EDGE_GAP, 6)
    // (c) 其他 → 原地，直邊長度
    expect(settleRelease(st, 400, straight, corner)).toEqual({ center: 400, len: straight, corner: false })
  })
})

describe('AC-EN-10 靜止時完整留在原本那條直邊（QA 第 3 輪）', () => {
  /** 那一段的邊與第一段兩端（螢幕座標，沿邊）。 */
  const ends = (sc: number, len: number) => {
    const geo = bandGeometry(st, centerToStart(sc, len), len - T)
    return { edges: geo.pieces.map((p) => p.edge), lo: geo.pieces[0].u0, hi: geo.pieces[0].u1 }
  }
  it('文字變長會超出原本那條直邊時，整條往回移，尾端離螢幕邊 EDGE_GAP（與放開吸附一致；QA 第 3 輪）', () => {
    // 上方，尾端（右端）距右上角 24 px（例如吸附 (b) 之後），文字變長 64 px
    const short = 166
    const long = 230
    const top = st.A - EDGE_GAP - (short - T) / 2
    expect(ends(top, short).hi).toBeCloseTo(st.W - EDGE_GAP, 6)
    const grown = resizeCenter(st, top, short, long, 'ccw')
    expect(ends(grown, long).edges).toEqual(['top'])
    expect(ends(grown, long).hi).toBeCloseTo(st.W - EDGE_GAP, 6) // 尾端離右邊界 24 px，跟吸附一樣
    // 頭端（左端）整條往回移文字變長的長度：64 px
    expect(ends(top, short).lo - ends(grown, long).lo).toBeCloseTo(long - short, 6)
    // 之後文字變短：頭端不動、尾端縮回
    const shrunk = resizeCenter(st, grown, long, short, 'ccw')
    expect(ends(shrunk, short).lo).toBeCloseTo(ends(grown, long).lo, 6)

    // 左側，尾端（底端）距左下角 24 px：往上移，底端仍離下邊 24 px
    const left = st.corners.bl + EDGE_GAP + (short - T) / 2
    const leftGrown = resizeCenter(st, left, short, long, 'cw')
    expect(ends(leftGrown, long).edges).toEqual(['left'])
    expect(ends(leftGrown, long).hi).toBeCloseTo(st.H - EDGE_GAP, 6)
    expect(ends(left, short).lo - ends(leftGrown, long).lo).toBeCloseTo(long - short, 6)

    // 沒超出：照舊頭端不動
    const free = resizeCenter(st, 400, short, long, 'ccw')
    expect(ends(free, long).lo).toBeCloseTo(ends(400, short).lo, 6)
  })

  it('還原保存值時也完整留在那條直邊上（QA 第 3 輪）', () => {
    const len = 230
    // 上方頭端在右上角前 100 px，文字長到放不下：整條往回移到尾端貼齊右邊界
    const sc = restorePosition(st, { edge: 'top', head: (st.A - 100) / st.A }, len)
    expect(ends(sc, len).edges).toEqual(['top'])
    expect(ends(sc, len).hi).toBeCloseTo(st.W, 6)
    // 左側頭端（頂端）貼近左下角：往上移到底端貼齊下邊界
    const low = restoreCenter(st, { edge: 'left', head: 50 / st.B }, len, 37)
    expect(ends(low, len).edges).toEqual(['left'])
    expect(ends(low, len).hi).toBeCloseTo(st.H, 6)
    // 舊格式中心比例也一樣
    const legacy = restorePosition(st, { edge: 'bottom', ratio: 0.001 }, len)
    expect(ends(legacy, len).edges).toEqual(['bottom'])
  })
})

describe('AC-EN-18／19 動畫步進（QA 第 3 輪）', () => {
  const motion = (m: Partial<NotchMotion>): NotchMotion => ({
    s: 300,
    sTarget: 300,
    v: 0,
    len: 166,
    lenTarget: 166,
    lenV: 0,
    dragging: false,
    ...m,
  })

  it('減少動態時任何目標改變都直接到位、速度歸零，不再要下一幀（QA 第 3 輪）', () => {
    // 從 hover（多 88 px）直接開始拖曳：長度目標縮回靜止長度
    const m = motion({ s: 300, sTarget: 340, v: 120, len: 254, lenTarget: 166, lenV: -50, dragging: true })
    expect(stepMotion(st, m, 1 / 60, true)).toBe(false)
    expect(m).toMatchObject({ s: 340, len: 166, v: 0, lenV: 0 })
    // 放開後不會多出 88 px，也不再要 frame
    m.dragging = false
    m.sTarget = 900
    m.lenTarget = 166 + HOVER_EXTRA
    expect(stepMotion(st, m, 1 / 60, true)).toBe(false)
    expect(m).toMatchObject({ s: 900, len: 166 + HOVER_EXTRA, v: 0, lenV: 0 })
    expect(notchFrameNeeded(false, 0, true, true)).toBe(false)
  })

  it('靜止時零動畫 frame；減少動態不為文字淡入跑 frame（QA 第 3 輪）', () => {
    const m = motion({})
    expect(stepMotion(st, m, 1 / 60, false)).toBe(false)
    expect(notchFrameNeeded(false, 1e9, false, false)).toBe(false)
    expect(notchFrameNeeded(false, 50, false, false)).toBe(true) // 文字淡入中
    expect(notchFrameNeeded(false, 50, false, true)).toBe(false) // 減少動態：文字直接不透明
    expect(notchFrameNeeded(false, 1e9, true, false)).toBe(true) // 跑馬燈捲動
    expect(notchFrameNeeded(false, 1e9, true, true)).toBe(false) // 減少動態：跑馬燈不捲動
    expect(notchFrameNeeded(true, 1e9, false, false)).toBe(true)
    // 目標改變：有限幀後停下並精確到位
    m.sTarget = 420
    let frames = 0
    while (stepMotion(st, m, 1 / 60, false) && frames < 600) frames++
    expect(frames).toBeLessThan(40)
    expect(m).toMatchObject({ s: 420, v: 0, len: 166, lenV: 0 })
  })

  it('拖曳中中心與長度也走同一個彈簧，收回時頭端每幀不動（QA 第 3 輪）', () => {
    const rest = 166
    const len = rest + HOVER_EXTRA
    for (const head of ['ccw', 'cw'] as HeadEnd[]) {
      const s = head === 'ccw' ? 400 : defaultCenter(st, len, 37)
      const h0 = headOf(s, len, head)
      for (const dragging of [true, false]) {
        const m = motion({ s, sTarget: headAnchoredCenter(st, s, len, rest, head), len, lenTarget: rest, dragging })
        let worst = 0
        let frames = 0
        while (stepMotion(st, m, 1 / 60, false) && frames++ < 600) {
          worst = Math.max(worst, Math.abs(wrapDelta(st, headOf(m.s, m.len, head) - h0)))
        }
        expect(worst, `${head} dragging=${dragging}`).toBeLessThanOrEqual(0.5)
        expect(Math.abs(wrapDelta(st, headOf(m.s, m.len, head) - h0))).toBeLessThan(1e-6)
      }
    }
  })
})

describe('AC-EN-04 抓取的邊（QA 第 4 輪）', () => {
  it('抓取的邊是游標底下那一臂：直邊瀏海貼齊角落時按在端點附近，不換到垂直的那條邊（QA 第 4 輪）', () => {
    const len = 200
    const lc = len - T
    // 角落 L 形：在任一臂離角落中心線點 30 px 處按下（換邊遲滯區內），抓取的邊就是那一臂；沿它移 10 px，中心也只移 10 px。
    for (const corner of ['tl', 'tr', 'br', 'bl'] as Corner[]) {
      const sc = st.corners[corner]
      const k = pointOnCenterline(st, sc)
      for (const pc of pieces(st, centerToStart(sc, len), lc)) {
        const vertical = pc.edge === 'left' || pc.edge === 'right'
        const away = vertical ? { x: 0, y: k.y < st.H / 2 ? 1 : -1 } : { x: k.x < st.W / 2 ? 1 : -1, y: 0 }
        const p = { x: k.x + away.x * 30, y: k.y + away.y * 30 }
        const g = grabStart(st, sc, p, len)
        expect(g.edge, `${corner} 抓 ${pc.edge} 臂`).toBe(pc.edge)
        expect(Math.abs(wrapDelta(st, dragCenter(st, g.edge, p, g.offset) - sc)), `${corner} ${pc.edge}`).toBeLessThan(0.5)
        const moved = { x: p.x + away.x * 10, y: p.y + away.y * 10 }
        expect(Math.abs(wrapDelta(st, dragCenter(st, g.edge, moved, g.offset) - sc)), `${corner} ${pc.edge}`).toBeCloseTo(10, 5)
      }
    }
    // 直邊瀏海尾端貼齊螢幕邊（還原保存值放不下時）：按在端點附近、離垂直那條邊比較近，抓取的邊仍是瀏海所在的邊——
    // 瀏海沒有長到那條邊上，那條邊的投影被夾在角落，用它算抓取偏移會一換邊就跳。
    const leftFlush = restorePosition(st, { edge: 'left', head: 0.95 }, len) // 左側尾端（底端）貼齊螢幕下緣
    expect(pieces(st, centerToStart(leftFlush, len), lc).map((pc) => pc.edge)).toEqual(['left'])
    const nearBottom = { x: 24, y: st.H - 18 } // 離下緣 18 px、離左緣 24 px：四邊裡最近的是下緣
    const g = grabStart(st, leftFlush, nearBottom, len)
    expect(g.edge).toBe('left')
    const up = { x: 24, y: st.H - 48 }
    expect(Math.abs(wrapDelta(st, dragCenter(st, g.edge, up, g.offset) - leftFlush))).toBeCloseTo(30, 5)
  })
})

describe('AC-EN-10 數字等寬（QA 第 4 輪）', () => {
  it('數字等寬：英數字詞裡的數字以最寬數字的寬度量，倒數每秒換字靜止長度不變（QA 第 4 輪）', () => {
    // 比例寬度的數字（Geist Sans 12 px 實測近似）：0 最寬、1 最窄。
    const widths: Record<string, number> = {
      '0': 8.06, '1': 4.62, '2': 7.43, '3': 7.36, '4': 7.63, '5': 7.51, '6': 7.49, '7': 7.02, '8': 7.25, '9': 7.49, ':': 3.52,
    }
    const prop = (s: string) => [...s].reduce((n, ch) => n + (widths[ch] ?? 7), 0)
    const adv = (s: string) => tokenize(s, prop)[0].adv
    // 每個數字都以最寬數字（這裡是 0）的寬度量
    expect(adv('1:11')).toBeCloseTo(adv('0:00'), 9)
    expect(adv('2:47')).toBeCloseTo(prop('0:00') + 1.5, 9)
    // 倒數每一秒、題數 2–9 則：靜止長度都相同
    const warn = new Set<number>()
    for (let sec = 180; sec >= 1; sec--) warn.add(restLength(restText({ kind: 'warning', count: 1, remainingMs: sec * 1000 }), prop))
    expect(warn.size).toBe(1)
    const counts = new Set<number>()
    for (let n = 2; n <= 9; n++) counts.add(restLength(restText({ kind: 'pending', count: n, remainingMs: 0 }), prop))
    expect(counts.size).toBe(1)
    // 沒有數字的字詞照原本量
    expect(adv('ROAS')).toBeCloseTo(prop('ROAS') + 1.5, 9)
  })
})

describe('INV-24／AC-EN-14 拖曳中所有題目結案時的放開（QA 第 4 輪）', () => {
  it('拖曳中所有題目結案：以結案前畫出來的長度定位頭端，保存的頭端在放開處（QA 第 4 輪）', () => {
    const drawn = restLength('時間到了，分享螢幕結束就跳出來', measure)
    const empty = restLength('你有一則待處理通知', measure)
    const emptyCorner = restLength('你有一則待處理通知', measure, true)
    expect(drawn).toBeGreaterThan(empty)
    // 左側（頭端在上，cw）與上方（頭端在左，ccw）、都離角落夠遠：放開處就是留原地（場景 7 (c)）。
    const cases: [string, number, HeadEnd][] = [
      ['左側', st.corners.bl + st.B / 2, 'cw'],
      ['上方', st.A / 2, 'ccw'],
    ]
    for (const [name, sc, head] of cases) {
      const released = headOf(sc, drawn, head) // 消失那一刻畫出來的頭端
      const r = settleRelease(st, sc, empty, emptyCorner, { len: drawn, head })
      expect(r.corner, name).toBe(false)
      expect(r.len, name).toBe(empty)
      // 放開後（以空佇列長度）頭端仍在放開處
      expect(Math.abs(wrapDelta(st, headOf(r.center, r.len, head) - released)), name).toBeLessThan(0.5)
      // 保存再還原（新題目讓瀏海再出現，文字是一般待處理文字）：頭端在放開處
      const back = restorePosition(st, savePosition(st, r.center, r.len), empty)
      expect(Math.abs(wrapDelta(st, headOf(back, empty, head) - released)), name).toBeLessThan(0.5)
    }
    // 沒有給畫出來的長度（一般放開）：行為不變
    expect(settleRelease(st, 400, empty, emptyCorner)).toEqual({ center: 400, len: empty, corner: false })
  })
})

describe('AC-EN-06 frame 時間差（QA 第 4 輪）', () => {
  it('frame 時間差為負或 0：這一幀不移動，不會先往反方向跳（QA 第 4 輪）', () => {
    // 瀏覽器的 rAF 時間戳可能早於觸發動畫的事件（開始拖曳的那一刻），時間差是負的幾毫秒。
    for (const dt of [-0.000274, -0.004, -0.016, 0]) {
      // 目標在後方 4 px（往周長負方向）：不能往正方向跳
      expect(springStep(st, 0, 0, st.P - 4, OMEGA_SETTLE, dt), `dt ${dt}`).toEqual([0, 0])
      expect(springStep(null, 176, 0, 176, OMEGA_SETTLE, dt), `dt ${dt}`).toEqual([176, 0])
      const m: NotchMotion = { s: 0, sTarget: st.P - 4, v: 0, len: 176, lenTarget: 176, lenV: 0, dragging: true }
      stepMotion(st, m, dt, false)
      expect([m.s, m.len], `dt ${dt}`).toEqual([0, 176])
    }
    // 正常的時間差照常朝目標移動
    const [x] = springStep(st, 0, 0, st.P - 4, OMEGA_SETTLE, 1 / 60)
    expect(wrapDelta(st, x)).toBeLessThan(0)
    expect(wrapDelta(st, x)).toBeGreaterThan(-4)
  })
})

describe('AC-EN-04 角落方格（QA 第 5 輪）', () => {
  /** 從螢幕角往內：離直的螢幕邊 `a` px、離橫的螢幕邊 `b` px 的點，與往內的方向。 */
  const inside = (c: Corner, a: number, b: number): { p: Pt; inX: number; inY: number } => {
    const inX = c === 'tl' || c === 'bl' ? 1 : -1
    const inY = c === 'tl' || c === 'tr' ? 1 : -1
    return { p: { x: inX > 0 ? a : st.W - a, y: inY > 0 ? b : st.H - b }, inX, inY }
  }
  const horizontal = (e: Edge) => e === 'top' || e === 'bottom'

  it('角落方格裡按下：按下時不決定，移動滿 4 px 時依方向選臂、不加遲滯，抓取偏移在選定的邊上以按下位置量（QA 第 5 輪）', () => {
    const len = 200
    for (const corner of ['tl', 'tr', 'br', 'bl'] as Corner[]) {
      const sc = st.corners[corner]
      const edges = pieces(st, centerToStart(sc, len), len - T).map((pc) => pc.edge)
      const horiz = edges.find(horizontal)!
      const vert = edges.find((e) => !horizontal(e))!
      // 方格裡：離直的螢幕邊 6 px、離橫的螢幕邊 20 px（離直的那條邊較近）
      const { p: press, inX, inY } = inside(corner, 6, 20)
      const g = grabStart(st, sc, press, len)
      expect(g.arms, `${corner} 方格裡`).not.toBeNull()
      expect([...g.arms!].sort(), corner).toEqual([horiz, vert].sort())
      // 橫向多於縱向 → 橫的那一臂，即使離直的那條邊比較近（換邊遲滯 40 px 也不擋這次選擇）
      const h = grabByDirection(st, sc, press, { x: press.x + inX * 4, y: press.y + inY }, g.arms!)
      expect(h.edge, `${corner} 橫著拖`).toBe(horiz)
      expect(h.offset, `${corner} 抓取偏移以按下位置量`).toBeCloseTo(wrapDelta(st, sOnEdge(st, horiz, press) - sc), 9)
      // 縱向多於橫向 → 直的那一臂
      const v = grabByDirection(st, sc, press, { x: press.x + inX, y: press.y + inY * 4 }, g.arms!)
      expect(v.edge, `${corner} 直著拖`).toBe(vert)
      expect(v.offset).toBeCloseTo(wrapDelta(st, sOnEdge(st, vert, press) - sc), 9)
      // 一樣多：離按下點較近的邊（這裡是直的那條）
      expect(grabByDirection(st, sc, press, { x: press.x + inX * 3, y: press.y + inY * 3 }, g.arms!).edge, `${corner} 斜著拖`).toBe(vert)
      // 選定後沿那一臂移動：中心沿周長的位移＝游標在那條邊上的投影位移（按下點算起，角落附近被夾住的部分不算）
      for (const [pick, to] of [
        [h, { x: press.x + inX * 30, y: press.y }],
        [v, { x: press.x, y: press.y + inY * 30 }],
      ] as const) {
        const want = wrapDelta(st, sOnEdge(st, pick.edge, to) - sOnEdge(st, pick.edge, press))
        expect(wrapDelta(st, dragCenter(st, pick.edge, to, pick.offset) - sc), `${corner} ${pick.edge}`).toBeCloseTo(want, 9)
      }
    }
  })

  it('方格外、直邊瀏海貼齊角落、沒給長度：按下時就決定（arms 為 null，照第 4 輪）（QA 第 5 輪）', () => {
    const len = 200
    // 角落 L 形、方格外（離直的螢幕邊 46 px）：照游標底下那一臂
    const tl = grabStart(st, st.corners.tl, { x: 46, y: 16 }, len)
    expect(tl.arms).toBeNull()
    expect(tl.edge).toBe('top')
    // 剛好在方格邊界上（離螢幕邊 T）不算方格裡
    expect(grabStart(st, st.corners.tl, { x: T, y: 10 }, len).arms).toBeNull()
    // 直邊瀏海尾端貼齊螢幕下緣：只在左邊上，角落那一格也只有一臂
    const leftFlush = restorePosition(st, { edge: 'left', head: 0.95 }, len)
    const flush = grabStart(st, leftFlush, { x: 6, y: st.H - 6 }, len)
    expect(flush.arms).toBeNull()
    expect(flush.edge).toBe('left')
    // 沒給長度：不判斷方格
    expect(grabStart(st, st.corners.tl, { x: 6, y: 20 }).arms).toBeNull()
  })

  it('hover 轉角成 L、在角落方格裡按下沿轉過去的那一臂拖：選臂後以收回後的位置量偏移，頭端位移＝游標投影位移（QA 第 5 輪）', () => {
    const rest = 200
    const half = (rest - T) / 2
    // 左側靠下，尾端離下緣 50 px：hover 延長 88 px 時尾端轉到底邊成 L
    const sc = st.corners.bl + 50 + half
    const head = preferredHead(pieces(st, centerToStart(sc, rest), rest - T))
    const len = rest + HOVER_EXTRA
    const ext = hoverTarget(st, { s: sc, sTarget: sc }, growDirection(head), true)
    expect(pieces(st, centerToStart(ext, len), len - T).map((pc) => pc.edge)).toEqual(['bottom', 'left'])
    const press = { x: 6, y: st.H - 20 }
    const g = grabStart(st, ext, press, len)
    expect(g.arms).not.toBeNull()
    const to = { x: press.x + 4, y: press.y }
    const pick = grabByDirection(st, ext, press, to, g.arms!)
    expect(pick.edge).toBe('bottom')
    const c = grabAfterCollapse(st, pick.edge, press, ext, len, rest, head)
    const far = { x: press.x + 40, y: press.y }
    const want = Math.abs(wrapDelta(st, sOnEdge(st, 'bottom', far) - sOnEdge(st, 'bottom', press)))
    const moved = Math.abs(wrapDelta(st, headOf(dragCenter(st, pick.edge, far, c.offset), rest, head) - headOf(c.center, rest, head)))
    expect(moved).toBeCloseTo(want, 9)
    expect(Math.abs(wrapDelta(st, headOf(c.center, rest, head) - headOf(sc, rest, head)))).toBeLessThan(1e-6)
  })
})

describe('AC-EN-10 倒數分鐘依警告長度補位（QA 第 5 輪）', () => {
  it('倒數分鐘依這一題的警告長度補位：量寬與繪製同一個位數，提前警告 10 分鐘以上時長度也不變（QA 第 5 輪）', () => {
    const warn = (remainingMs: number, warnMs?: number) => restText({ kind: 'warning', count: 1, remainingMs, warnMs })
    // 預設 3 分鐘：1 位（不補）
    expect(warn(179_000, 180_000)).toBe('2:59 後強制詢問')
    expect(warn(1_000, 180_000)).toBe('0:01 後強制詢問')
    // 沒有警告長度（舊資料）：照原本不補
    expect(warn(161_000)).toBe('2:41 後強制詢問')
    // 10 分鐘：2 位
    expect(warn(600_000, 600_000)).toBe('10:00 後強制詢問')
    expect(warn(599_000, 600_000)).toBe('09:59 後強制詢問')
    expect(warn(1_000, 600_000)).toBe('00:01 後強制詢問')
    // 9 分 59.5 秒的警告（不是整分）：最多顯示 10:00，也是 2 位
    expect(warn(599_500, 599_500)).toBe('10:00 後強制詢問')
    expect(warn(598_000, 599_500)).toBe('09:58 後強制詢問')
    // 9 分鐘：1 位
    expect(warn(540_000, 540_000)).toBe('9:00 後強制詢問')
    // 100 分鐘以上（週期最長 120、警告最多 119）：3 位
    expect(warn(6_000_000, 6_000_000)).toBe('100:00 後強制詢問')
    expect(warn(5_999_000, 6_000_000)).toBe('099:59 後強制詢問')
    // 比例寬度的數字（0 最寬、1 最窄）：每一秒的靜止長度都一樣
    const widths: Record<string, number> = { '0': 8.06, '1': 4.62, '2': 7.43, '3': 7.36, '4': 7.63, '5': 7.51, '6': 7.49, '7': 7.02, '8': 7.25, '9': 7.49, ':': 3.52 }
    const prop = (s: string) => [...s].reduce((n, ch) => n + (widths[ch] ?? 7), 0)
    for (const warnMs of [60_000, 180_000, 600_000, 14 * 60_000, 119 * 60_000]) {
      const lens = new Set<number>()
      for (let sec = Math.ceil(warnMs / 1000); sec >= 1; sec--) lens.add(restLength(warn(sec * 1000, warnMs), prop))
      expect([...lens], `警告 ${warnMs / 60_000} 分鐘`).toHaveLength(1)
    }
  })
})
