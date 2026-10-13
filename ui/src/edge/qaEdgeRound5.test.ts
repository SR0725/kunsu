// QA 第 5 輪（BDD edge-notch，2026-10-09，獨立 QA）：回頭驗證第 4 輪修正（main 2f1dd41..7c6474d）與主視窗改版加的「同一台
// 螢幕的保存值變成 null 時回到預設位置」，並重現第 5 輪新發現。標「[重現]」的測試以規格要求的行為斷言；目前失敗＝發現仍存在，
// 修好後應轉綠。標「[回歸]」的是第 4 輪修正的守門。
//
// 另一個第 5 輪發現在元件層（EdgeNotch.tsx 的 `savedPositionCleared` 用法），node 測試環境沒有 DOM，以瀏覽器重現（vite mock＋
// 假 Tauri IPC，1440×900，見 QA 回報）：第一次拖曳放開後 400 ms 內（寫檔還沒發生）、或寫檔失敗後，只要舞台因為任何原因重讀
// `edge_info`（螢幕配置改變、睡眠喚醒的 `kunsu://edge_stage_changed`），記憶體裡有位置、edge_info 的 saved 還是 null，就被當成
// 「設定頁回到預設位置」：瀏海從放開處（頭端 y 361）跳回預設位置（y 61），接著延後的寫檔照樣把放開處寫進檔案，下次啟動又回到
// 放開處。同一個機制也讓「拖曳後 400 ms 內按回到預設位置」被延後的寫檔蓋回去。
//
// 第 5 輪修正（實作者，2026-10-09 主線決定）時改了兩處測試機制，斷言不變：
// - `dragTargets` 照修正後的 EdgeNotch 流程：按在角落方格裡時 `grabStart` 回傳 `arms`、按下時不決定抓哪一臂，移動滿 4 px
//   那一刻以 `grabByDirection` 依移動方向選臂、抓取偏移在選定的邊上以按下位置量（方格外照舊）。頭端位移＝游標投影位移
//   （≤ 0.5 px）的斷言不變。
// - FR-03：分鐘依這一題的警告長度補位（`RestState.warnMs`＝force_at − warn_at），所以重現測試帶上 10 分鐘的警告長度，
//   「9:59」改成補位後的「09:59」；長度相同的斷言不變。
import { describe, expect, it } from 'vitest'
import {
  T,
  centerToStart,
  dragCenter,
  grabByDirection,
  grabStart,
  makeStage,
  nearestEdge,
  pieces,
  predictPoint,
  preferredHead,
  restLength,
  restText,
  sOnEdge,
  tokenize,
  wrapDelta,
  type Corner,
  type Edge,
  type HeadEnd,
  type Pt,
} from './geometry'

const st = makeStage(1440, 900)
const measure = (s: string) => s.length * 7
const TEXT = '你有一則待處理通知'

/** 頭端在中心線上的周長座標：ccw 在起點端、cw 在終點端。 */
const headOf = (sc: number, len: number, head: HeadEnd) => (head === 'ccw' ? centerToStart(sc, len) : centerToStart(sc, len) + len - T)

/**
 * 照 EdgeNotch 的拖曳流程：pointerdown 時 `grabStart(stage, m.s, p, m.len)`（元件傳畫出來的長度）；移動 ≥ 4 px 開始拖曳——
 * 按在角落方格裡（`grab.arms`）時這一刻才以 `grabByDirection` 依移動方向選臂、量抓取偏移（第 5 輪修正）；之後每個
 * pointermove 以 EMA 平滑速度、預測 120 ms 後的位置選邊（`nearestEdge`，40 px 遲滯），中心＝`dragCenter`。回傳每一個
 * 已開始拖曳的 move 的目標中心（與對應的 move 索引）。
 */
function dragTargets(s: number, len: number, press: Pt, moves: Pt[], dtMs: number): { i: number; target: number }[] {
  const grab = grabStart(st, s, press, len)
  let edge: Edge = grab.edge
  let offset = grab.offset
  let dragged = false
  let v = { x: 0, y: 0 }
  let last = press
  const out: { i: number; target: number }[] = []
  moves.forEach((p, i) => {
    const dt = Math.max(1, dtMs) / 1000
    const a = 0.35
    v = { x: v.x * (1 - a) + ((p.x - last.x) / dt) * a, y: v.y * (1 - a) + ((p.y - last.y) / dt) * a }
    last = p
    if (!dragged && Math.hypot(p.x - press.x, p.y - press.y) >= 4) {
      dragged = true
      if (grab.arms) ({ edge, offset } = grabByDirection(st, s, press, p, grab.arms))
    }
    if (!dragged) return
    edge = nearestEdge(st, predictPoint(st, p, v), edge)
    out.push({ i, target: dragCenter(st, edge, p, offset) })
  })
  return out
}

/** 四個角落的螢幕角點與兩臂。 */
const CORNER_PT: Record<Corner, Pt> = { tl: { x: 0, y: 0 }, tr: { x: st.W, y: 0 }, br: { x: st.W, y: st.H }, bl: { x: 0, y: st.H } }
const CORNER_EDGES: Record<Corner, [Edge, Edge]> = { tl: ['top', 'left'], tr: ['top', 'right'], br: ['right', 'bottom'], bl: ['bottom', 'left'] }
/** 從角點往螢幕內、離 `edge` 那條螢幕邊 `d` px 的方向（沿另一軸）。 */
const inward = (c: Corner): Pt => ({ x: CORNER_PT[c].x === 0 ? 1 : -1, y: CORNER_PT[c].y === 0 ? 1 : -1 })

describe('AC-EN-04 角落 L 形：在兩臂重疊的角落方格裡按下、沿其中一臂拖（QA 第 5 輪）', () => {
  // 角落 L 形（停在角落的靜止長度）。兩臂在角落重疊成 T×T（32×32）的方格：方格裡的每一點同時在兩臂的黑色上。
  const len = restLength(TEXT, measure, true)
  // 游標以 125 px/s（每 8 ms 1 px）沿那一臂遠離角落移動 48 px。
  const D = 48
  for (const corner of ['tl', 'tr', 'br', 'bl'] as Corner[]) {
    const sc = st.corners[corner]
    const head = preferredHead(pieces(st, centerToStart(sc, len), len - T))
    for (const along of CORNER_EDGES[corner]) {
      const other = CORNER_EDGES[corner].find((e) => e !== along)!
      it(`[重現] ${corner}：按在角落方格裡（離 ${other} 邊 4 px、離 ${along} 邊 28 px），沿 ${along} 臂拖，頭端位移＝游標在 ${along} 邊上的投影位移`, () => {
        const k = CORNER_PT[corner]
        const n = inward(corner)
        // 離 along 那條螢幕邊 28 px（仍在 along 臂的 32 px 厚度內）、離 other 那條螢幕邊 4 px。
        const alongIsHorizontal = along === 'top' || along === 'bottom'
        const press: Pt = alongIsHorizontal ? { x: k.x + n.x * 4, y: k.y + n.y * 28 } : { x: k.x + n.x * 28, y: k.y + n.y * 4 }
        // 沿 along 臂遠離角落：上下邊的臂沿 x 走，左右邊的臂沿 y 走。
        const away: Pt = alongIsHorizontal ? { x: n.x, y: 0 } : { x: 0, y: n.y }
        const moves = Array.from({ length: D }, (_, i) => ({ x: press.x + away.x * (i + 1), y: press.y + away.y * (i + 1) }))
        const targets = dragTargets(sc, len, press, moves, 8)
        const h0 = headOf(sc, len, head)
        const p0 = sOnEdge(st, along, press)
        let worst = 0
        let worstAt = ''
        for (const { i, target } of targets) {
          // 游標在 along 那條邊上的投影走了多少（角落附近被夾住的那幾 px 不算）＝頭端應該沿周長走的距離。
          const want = Math.abs(wrapDelta(st, sOnEdge(st, along, moves[i]) - p0))
          const moved = Math.abs(wrapDelta(st, headOf(target, len, head) - h0))
          if (Math.abs(moved - want) > Math.abs(worst)) {
            worst = moved - want
            worstAt = `游標移 ${i + 1} px（投影 ${want.toFixed(1)} px）時頭端移 ${moved.toFixed(1)} px`
          }
        }
        // 實測（瀏覽器，1440×900）：tl 按 (28,4) 往下拖 48 px 頭端完全不動；按 (20,10) 頭端先停住、落後最多 42 px，換邊後
        // 一次追上，最後仍差 2 px。原因：方格裡離 other 邊較近，抓取的邊選了 other；沿 along 移動要等預測點比 other 近 40 px
        // （場景 5 的換邊遲滯）才換邊，期間 other 邊上的投影被夾在角落，頭端不動。
        expect(Math.abs(worst), worstAt).toBeLessThanOrEqual(0.5)
      })
    }
  }

  it('[回歸] 對照：按在方格外、離角落 30 px 的那一臂上，沿同一臂拖，頭端位移＝游標位移（第 4 輪修正）', () => {
    const sc = st.corners.tl
    const head = preferredHead(pieces(st, centerToStart(sc, len), len - T))
    const press = { x: 46, y: 16 }
    const moves = Array.from({ length: D }, (_, i) => ({ x: press.x + i + 1, y: press.y }))
    const h0 = headOf(sc, len, head)
    for (const { i, target } of dragTargets(sc, len, press, moves, 8)) {
      expect(Math.abs(Math.abs(wrapDelta(st, headOf(target, len, head) - h0)) - (i + 1))).toBeLessThanOrEqual(0.5)
    }
  })
})

describe('AC-EN-10／PRD FR-03 倒數數字等寬，但提前警告可以設到 10 分鐘以上（QA 第 5 輪）', () => {
  const warnLen = (ms: number, warnMs?: number) => restLength(restText({ kind: 'warning', count: 1, remainingMs: ms, warnMs }), measure)

  it('[回歸] 預設三分鐘警告：3:00 到 0:01 每一秒靜止長度相同（第 4 輪修正）', () => {
    const lens = new Set<number>()
    for (let s = 180; s >= 1; s--) lens.add(warnLen(s * 1000))
    expect([...lens]).toHaveLength(1)
  })

  it('[重現] 設定頁允許提前警告 1 到「週期−1」分鐘（PRD §27.3、CONTRACT §15）：設 10 分鐘以上時倒數跨過 10:00 → 9:59，靜止長度改變', () => {
    // restText 的分鐘不補位：「10:00 後強制詢問」→「9:59 後強制詢問」少一位數，尾端在那一秒縮回並跑一次伸縮彈簧。
    // 第 5 輪決定：分鐘依這一題的警告長度（force_at − warn_at，這裡 10 分鐘）補位，「9:59」→「09:59」。
    const warnMs = 600_000
    expect(restText({ kind: 'warning', count: 1, remainingMs: 600_000, warnMs })).toBe('10:00 後強制詢問')
    expect(restText({ kind: 'warning', count: 1, remainingMs: 599_000, warnMs })).toBe('09:59 後強制詢問')
    expect(warnLen(599_000, warnMs), 'warning 期間倒數每秒換字時長度完全相同（場景 10）').toBe(warnLen(600_000, warnMs))
  })

  it('[回歸] 量寬：英數字詞裡每個數字都以最寬數字計算（第 4 輪修正）', () => {
    const widths: Record<string, number> = { '0': 8, '1': 4, '2': 7, '3': 7, '4': 7, '5': 7, '6': 7, '7': 6, '8': 7, '9': 7, ':': 3 }
    const m = (s: string) => [...s].reduce((n, ch) => n + (widths[ch] ?? 7), 0)
    const adv = (s: string) => tokenize(s, m)[0].adv
    expect(adv('1:11')).toBe(adv('0:00'))
    expect(adv('2:59')).toBe(adv('0:00'))
  })
})
