// QA 第 4 輪（BDD edge-notch，2026-10-09，獨立 QA）：回頭驗證第 3 輪修正與主線「文字變長超出那條邊時整條往回移、尾端離螢幕邊
// 24 px」，並重現第 4 輪新發現。標「[重現]」的測試以規格要求的行為斷言；目前失敗＝發現仍存在，修好後應轉綠。標「[回歸]」的是
// 第 3 輪修正的守門。
//
// 另外三個第 4 輪發現在元件層（EdgeNotch.tsx）或殼層組合，node 測試環境沒有 DOM，以瀏覽器重現（vite mock＋假 Tauri IPC，
// 1440×900，見 QA 回報）：
// 1. INV-24／AC-EN-14：拖曳中舞台被殼層隱藏（勿擾、⌘⇧K、別處設定勿擾）時，原生 `poll_once` 在隱藏時把自己的 dragging 清掉，
//    但不通知網頁；網頁的手勢與 `m.dragging` 留著。瀏海之後再出現、幾何一改變（題數、倒數文字），網頁就回報
//    `edge_hit {dragging: true}`，原生 `set_hit` 重新接管全螢幕指標事件——當下若正按著滑鼠在拖別的 app，會被接管。
// 2. INV-24：拖曳中所有題目在別處結案時，網頁照放開處理，但 settle() 用的是空佇列的靜止文字長度（「你有一則待處理通知」），
//    中心保持不變、長度換掉，保存的頭端偏移「拖曳中長度與空佇列長度差的一半」（實測「時間到了，分享螢幕結束就跳出來」時
//    放開處 410、保存與再出現 450.5，差 40.5 px）。
// 3. AC-EN-21 附帶：瀏海靜止、沒有 hover 時，隱私 guard 解除（例如麥克風關掉）的推送會重設 textFadeStart，靜止文字從
//    0.08 透明度重新淡入 180 ms（閃一下）。
// 另有一個殼層發現只能讀程式確認（需真機）：舞台隱藏期間目標螢幕換成另一台同尺寸的螢幕時，`edge::prepare_stage` 貼合新螢幕
// 但不發 `kunsu://edge_stage_changed`，網頁也收不到 resize（大小沒變），沿用舊螢幕的 display_id 與保存值。
import { describe, expect, it } from 'vitest'
import contract from '../../../docs/CONTRACT.md?raw'
import {
  EDGE_GAP,
  HOVER_EXTRA,
  T,
  bandGeometry,
  centerToStart,
  dragCenter,
  edgeOfS,
  grabAfterCollapse,
  grabStart,
  growDirection,
  hoverTarget,
  makeStage,
  mod,
  nearestEdge,
  pieces,
  pointOnCenterline,
  predictPoint,
  preferredHead,
  resizeCenter,
  restLength,
  restText,
  restorePosition,
  stepMotion,
  wrapDelta,
  type Corner,
  type Edge,
  type HeadEnd,
  type NotchMotion,
  type Pt,
} from './geometry'

const st = makeStage(1440, 900)
const measure = (s: string) => s.length * 7
const TEXT = '你有一則待處理通知'

/** 頭端在中心線上的周長座標：ccw 在起點端、cw 在終點端。 */
const headOf = (sc: number, len: number, head: HeadEnd) => (head === 'ccw' ? centerToStart(sc, len) : centerToStart(sc, len) + len - T)

/**
 * 照 EdgeNotch 的拖曳流程：pointerdown 時 grabStart；移動 ≥ 4 px 開始拖曳（從 hover 開始時改用 grabAfterCollapse 的抓取
 * 偏移）；之後每個 pointermove 以 EMA 平滑速度、預測 120 ms 後的位置選邊（nearestEdge），中心＝dragCenter。回傳每一步的
 * 目標中心。`events` 每 `dtMs` 一個。
 */
function dragTargets(
  s: number,
  press: Pt,
  moves: Pt[],
  dtMs: number,
  hover?: { len: number; rest: number; head: HeadEnd },
): number[] {
  const grab = grabStart(st, s, press)
  let edge: Edge = grab.edge
  let offset = grab.offset
  let dragged = false
  let v = { x: 0, y: 0 }
  let last = press
  const out: number[] = []
  for (const p of moves) {
    const dt = Math.max(1, dtMs) / 1000
    const a = 0.35
    v = { x: v.x * (1 - a) + ((p.x - last.x) / dt) * a, y: v.y * (1 - a) + ((p.y - last.y) / dt) * a }
    last = p
    if (!dragged && Math.hypot(p.x - press.x, p.y - press.y) >= 4) {
      dragged = true
      if (hover) offset = grabAfterCollapse(st, edge, press, s, hover.len, hover.rest, hover.head).offset
    }
    if (!dragged) continue
    edge = nearestEdge(st, predictPoint(st, p, v), edge)
    out.push(dragCenter(st, edge, p, offset))
  }
  return out
}

/** 角落中心線點（L 形兩臂中心線的交點）。 */
const cornerPt = (c: Corner): Pt => pointOnCenterline(st, st.corners[c])

/** 沿某一臂、離角落中心線點 `d` px 的點，與沿這一臂遠離角落的方向。 */
function armPoint(c: Corner, edge: Edge, d: number): { p: Pt; away: Pt } {
  const k = cornerPt(c)
  const away: Pt =
    edge === 'top' || edge === 'bottom' ? { x: k.x < st.W / 2 ? 1 : -1, y: 0 } : { x: 0, y: k.y < st.H / 2 ? 1 : -1 }
  return { p: { x: k.x + away.x * d, y: k.y + away.y * d }, away }
}

const CORNER_EDGES: Record<Corner, [Edge, Edge]> = { tl: ['top', 'left'], tr: ['top', 'right'], br: ['right', 'bottom'], bl: ['bottom', 'left'] }

describe('AC-EN-04 L 形抓住靠近角落的那一臂開始拖曳（QA 第 4 輪）', () => {
  const len = restLength(TEXT, measure, true)
  // 游標沿抓住的那一臂、遠離角落，以 250 px/s 移動（每 8 ms 2 px），共 24 px。
  const MOVES = 12
  const STEP = 2
  const DT = 8

  for (const corner of ['tl', 'tr', 'br', 'bl'] as Corner[]) {
    const sc = st.corners[corner]
    const head = preferredHead(pieces(st, centerToStart(sc, len), len - T))
    for (const edge of CORNER_EDGES[corner]) {
      it(`[重現] ${corner} 角落 L 形：在 ${edge} 臂離角落 30 px 處按下、沿這一臂拖 24 px，頭端也只沿周長移 24 px`, () => {
        // 30 px：落在換邊遲滯區（游標到另一邊的距離 30＋16 < 16＋40），按下時用另一邊投影（被夾在角落），抓取偏移差 30 px。
        const { p, away } = armPoint(corner, edge, 30)
        const moves = Array.from({ length: MOVES }, (_, i) => ({ x: p.x + away.x * STEP * (i + 1), y: p.y + away.y * STEP * (i + 1) }))
        const targets = dragTargets(sc, p, moves, DT)
        const h0 = headOf(sc, len, head)
        // 每一步：頭端沿周長的位移＝游標沿這一臂的位移（≤ 0.5 px 誤差）。修正前：選邊一換到這一臂，頭端多跳 30 px。
        targets.forEach((target, i) => {
          const cursor = Math.hypot(moves[i + (MOVES - targets.length)].x - p.x, moves[i + (MOVES - targets.length)].y - p.y)
          const moved = Math.abs(wrapDelta(st, headOf(target, len, head) - h0))
          expect(Math.abs(moved - cursor), `${corner}/${edge} 第 ${i + 1} 步：游標 ${cursor} px、頭端 ${moved.toFixed(2)} px`).toBeLessThanOrEqual(0.5)
        })
      })
    }
  }
})

describe('AC-EN-04 從 hover 轉角的 L 開始拖曳（QA 第 4 輪）', () => {
  const rest = restLength(TEXT, measure)
  const half = (rest - T) / 2
  // 尾端離下一個角落只剩 50 px：hover 時尾端轉過角落成 L（第 3 輪場景 11），轉過去的那一臂只有 88 − 50 ≈ 38 px。
  const cases: { name: string; sc: number; corner: Corner }[] = [
    { name: '左側靠下（尾端轉到底邊）', sc: st.corners.bl + 50 + half, corner: 'bl' },
    { name: '上方靠右（尾端轉到右側）', sc: st.corners.tr - 50 - half, corner: 'tr' },
    { name: '右側靠下（尾端轉到底邊）', sc: st.corners.br - 50 - half, corner: 'br' },
    { name: '下方靠右（尾端轉到右側）', sc: st.corners.br + 50 + half, corner: 'br' },
  ]
  for (const c of cases) {
    it(`[重現] ${c.name}：在轉過去的那一臂按下、沿它拖 16 px，頭端只移 16 px（游標到頭端的距離維持按下時的值）`, () => {
      const head = preferredHead(pieces(st, centerToStart(c.sc, rest), rest - T))
      const len = rest + HOVER_EXTRA
      const ext = hoverTarget(st, { s: c.sc, sTarget: c.sc }, growDirection(head), true)
      const ps = pieces(st, centerToStart(ext, len), len - T)
      expect(ps.length, '確實轉角成 L').toBe(2)
      // 轉過去的那一臂＝不是原本那條邊的那一段；在它上面離角落中心線點 20 px 處按下，沿它遠離角落移動。
      const restEdge = edgeOfS(st, c.sc)
      const turned = ps.find((pc) => pc.edge !== restEdge)!
      const { p, away } = armPoint(c.corner, turned.edge, 20)
      const moves = Array.from({ length: 8 }, (_, i) => ({ x: p.x + away.x * 2 * (i + 1), y: p.y + away.y * 2 * (i + 1) }))
      const targets = dragTargets(ext, p, moves, 8, { len, rest, head })
      const h0 = headOf(c.sc, rest, head) // hover 前（也是收回後）的頭端
      const last = targets.at(-1)!
      const cursor = Math.hypot(moves.at(-1)!.x - p.x, moves.at(-1)!.y - p.y)
      const moved = Math.abs(wrapDelta(st, headOf(last, rest, head) - h0))
      // 修正前：抓取偏移用原本那條邊的投影（被夾在角落）算，選邊換到轉過去的那一臂時頭端多跳約 20 px。
      expect(Math.abs(moved - cursor), `游標 ${cursor} px、頭端 ${moved.toFixed(2)} px`).toBeLessThanOrEqual(0.5)
    })
  }
})

/** 直邊上那一段的兩端（螢幕座標沿邊）與所在的邊。 */
function ends(sc: number, len: number) {
  const geo = bandGeometry(st, centerToStart(sc, len), len - T)
  return { edges: geo.pieces.map((p) => p.edge), lo: geo.pieces[0].u0, hi: geo.pieces[0].u1 }
}
/** 那條邊在螢幕座標上的長度（帶狀端點可以到 0 與這個值）。 */
const edgeScreenLen = (edge: Edge) => (edge === 'top' || edge === 'bottom' ? st.W : st.H)

describe('AC-EN-10 文字變長超出那條邊：整條往回移、尾端離螢幕邊 24 px（主線 c39099c；QA 第 4 輪回歸）', () => {
  const short = restLength(TEXT, measure)
  const long = restLength('時間到了，分享螢幕結束就跳出來', measure)
  // 四邊；尾端都離尾端那一側的角落 30 px（放得下），文字變長後超出。
  const cases: { edge: Edge; head: HeadEnd }[] = [
    { edge: 'top', head: 'ccw' },
    { edge: 'right', head: 'ccw' },
    { edge: 'bottom', head: 'cw' },
    { edge: 'left', head: 'cw' },
  ]
  for (const c of cases) {
    it(`[回歸] ${c.edge}：超出時尾端離螢幕邊 24 px、頭端只移必要距離；放得下時頭端不動；變短時頭端不動`, () => {
      const base = { top: 0, right: st.A, bottom: st.A + st.B, left: 2 * st.A + st.B }[c.edge]
      const span = c.edge === 'top' || c.edge === 'bottom' ? st.A : st.B
      const halfS = (short - T) / 2
      // 尾端側：ccw 頭在周長起點端，尾端在 base + span 那一側；cw 相反。
      const tailHigh = c.head === 'ccw'
      const sc = mod(tailHigh ? base + span - 30 - halfS : base + 30 + halfS, st.P)
      const before = ends(sc, short)
      const tailGap = (e: ReturnType<typeof ends>) => {
        // 尾端在螢幕座標上的 lo 或 hi：上、右兩邊周長方向與螢幕座標同向，下、左兩邊反向。
        const sameDir = c.edge === 'top' || c.edge === 'right'
        return sameDir === tailHigh ? edgeScreenLen(c.edge) - e.hi : e.lo
      }
      expect(tailGap(before)).toBeCloseTo(30, 6)
      // 長 4 px：放得下，頭端不動（尾端到 26 px）
      const fit = resizeCenter(st, sc, short, short + 4, c.head)
      expect(Math.abs(wrapDelta(st, headOf(fit, short + 4, c.head) - headOf(sc, short, c.head)))).toBeLessThan(1e-6)
      expect(tailGap(ends(fit, short + 4))).toBeCloseTo(26, 6)
      // 變很長：超出 → 整條往回移，尾端離螢幕邊 24 px，仍完整在這條邊上
      const grown = resizeCenter(st, sc, short, long, c.head)
      const g = ends(grown, long)
      expect(g.edges).toEqual([c.edge])
      expect(tailGap(g)).toBeCloseTo(EDGE_GAP, 6)
      // 頭端只移必要的距離：長度差 − 原本 30 px 間隙 ＋ 24 px
      const headMoved = Math.abs(wrapDelta(st, headOf(grown, long, c.head) - headOf(sc, short, c.head)))
      expect(headMoved).toBeCloseTo(long - short - 30 + EDGE_GAP, 6)
      // 之後變短：頭端不動
      const shrunk = resizeCenter(st, grown, long, short, c.head)
      expect(Math.abs(wrapDelta(st, headOf(shrunk, short, c.head) - headOf(grown, long, c.head)))).toBeLessThan(1e-6)
    })
  }

  it('[回歸] 還原保存值放不下時只保證完整在那條邊上（貼齊邊界，不加 24 px）', () => {
    const len = restLength('時間到了，分享螢幕結束就跳出來', measure)
    const sc = restorePosition(st, { edge: 'left', head: 30 / st.B }, len)
    const e = ends(sc, len)
    expect(e.edges).toEqual(['left'])
    expect(e.hi).toBeCloseTo(st.H, 6)
  })
})

describe('AC-EN-11 hover 尾端轉角成 L：延長與收回的每一幀頭端不動（QA 第 4 輪回歸，逐幀）', () => {
  const rest = restLength(TEXT, measure)
  const half = (rest - T) / 2
  const cases: [string, number][] = [
    ['左側靠下', st.corners.bl + 40 + half],
    ['上方靠右', st.corners.tr - 40 - half],
    ['右側靠下', st.corners.br - 40 - half],
    ['下方靠右', st.corners.br + 40 + half],
    ['角落 L（bl）', st.corners.bl],
  ]
  for (const [name, sc] of cases) {
    it(`[回歸] ${name}`, () => {
      const head = preferredHead(pieces(st, centerToStart(sc, rest), rest - T))
      const h0 = headOf(sc, rest, head)
      const m: NotchMotion = { s: sc, sTarget: sc, v: 0, len: rest, lenTarget: rest, lenV: 0, dragging: false }
      // 延長
      m.sTarget = hoverTarget(st, m, growDirection(head), true)
      m.lenTarget = rest + HOVER_EXTRA
      let worst = 0
      let frames = 0
      while (stepMotion(st, m, 1 / 60, false) && frames++ < 600) worst = Math.max(worst, Math.abs(wrapDelta(st, headOf(m.s, m.len, head) - h0)))
      expect(pieces(st, centerToStart(m.s, m.len), m.len - T).length, `${name} 延長後`).toBeGreaterThanOrEqual(2)
      // 收回（以頭端為錨：EdgeNotch.endHover 用 headAnchoredCenter，等同 hoverTarget 反向）
      m.sTarget = hoverTarget(st, m, growDirection(head), false)
      m.lenTarget = rest
      while (stepMotion(st, m, 1 / 60, false) && frames++ < 1200) worst = Math.max(worst, Math.abs(wrapDelta(st, headOf(m.s, m.len, head) - h0)))
      expect(worst, name).toBeLessThanOrEqual(0.5)
      expect(Math.abs(wrapDelta(st, m.s - sc))).toBeLessThan(1e-6)
    })
  }
})

describe('AC-EN-10／PRD FR-03 三分鐘警告倒數不造成 layout shift（QA 第 4 輪）', () => {
  // 瀏覽器實測（Geist Sans 12 px，canvas measureText）：數字是比例寬度，0＝8.06、1＝4.62 … 冒號 3.52；「1:11」17.38 px、
  // 「0:00」27.71 px。EdgeNotch 以 measureText 量英數字詞決定靜止長度，所以倒數每秒換字時尾端伸縮 1–3 px（實測上方
  // 159 → 158 → 156 → 159 px），每秒跑一段彈簧動畫（12 秒 133 frame）。
  const GEIST_12: Record<string, number> = {
    '0': 8.06, '1': 4.62, '2': 7.43, '3': 7.36, '4': 7.63, '5': 7.51, '6': 7.49, '7': 7.02, '8': 7.25, '9': 7.49, ':': 3.52,
  }
  const geist = (s: string) => [...s].reduce((n, ch) => n + (GEIST_12[ch] ?? 7), 0)

  it('[重現] warning 期間每一秒的靜止長度相同（倒數換字不讓尾端伸縮）', () => {
    const lengths = new Set<number>()
    for (let sec = 180; sec >= 1; sec--) {
      lengths.add(restLength(restText({ kind: 'warning', count: 1, remainingMs: sec * 1000 }), geist))
    }
    // CLAUDE.md §6「倒數、warning 與狀態…也不能造成 layout shift」；PRD FR-03（P0）「剩餘 3 分鐘時顯示不造成 layout shift
    // 的警告」。修法例：倒數數字用等寬數字（tabular figures）量寬與繪製，或以固定樣板（「0:00 後強制詢問」）決定長度。
    expect([...lengths].sort((a, b) => a - b)).toHaveLength(1)
  })
})

describe('文件一致性（QA 第 4 輪）', () => {
  it('[重現] CONTRACT §9「長度與動畫」：文字變長超出時尾端離螢幕邊 24 px（EDGE_GAP），不是貼齊邊界', () => {
    // 主線 0d7a25f 改了 PRD／DESIGN／BDD，CONTRACT 仍寫「resizeCenter／restorePosition 經 keepOnEdge：超出就整條往回移到尾端
    // 剛好貼齊邊界」，與 c39099c 的實作（resizeCenter 超出時留 EDGE_GAP）不一致。
    const line = contract.split('\n').find((l: string) => l.includes('resizeCenter') && l.includes('keepOnEdge')) ?? ''
    expect(line, 'CONTRACT 有描述 resizeCenter 的那一行').not.toBe('')
    expect(line).toMatch(/24 px|EDGE_GAP/)
  })
})
