// QA 第 3 輪（BDD edge-notch，2026-10-09）：回頭驗證第 2 輪修正，並重現第 3 輪新發現。
// 標「[重現]」的測試以規格要求的行為斷言；目前失敗＝發現仍存在，修好後應轉綠。標「[回歸]」的是第 2 輪修正的守門。
//
// 另一個第 3 輪發現在元件層（EdgeNotch.tsx），node 測試環境沒有 DOM，以瀏覽器重現（見 QA 回報）：
// 減少動態時，hover 中（已延長 88 px）直接開始拖曳 → `m.lenTarget = restLen` 但 `m.len` 沒有直接到位，kick() 在減少動態下
// 不跑彈簧，`moving` 永遠為真：瀏海放開後仍多出 88 px，rAF 每幀重繪不停（AC-EN-18／19）。
import { describe, expect, it } from 'vitest'
import { notchHitReport, notchLabel, notchRestState, plainLine } from './content'
import {
  EDGE_GAP,
  HOVER_EXTRA,
  T,
  centerToStart,
  growDirection,
  hoverTarget,
  makeStage,
  parseSavedPosition,
  pieces,
  pointOnCenterline,
  preferredHead,
  restLength,
  restText,
  restorePosition,
  savePosition,
  settleRelease,
  textPath,
  tokenize,
  type Edge,
  type HeadEnd,
} from './geometry'
import type { Question } from '../types'

const st = makeStage(1440, 900)
const measure = (s: string) => s.length * 7
const TEXT = '你有一則待處理通知'

/** 圖示中心（文字路徑起點）。 */
function iconAt(sc: number, len: number, head: HeadEnd) {
  return textPath(st, pieces(st, centerToStart(sc, len), len - T), head).at(0)
}

/**
 * 照 EdgeNotch.startHover：一律往尾端延長（實作者 2026-10-09：主線決定「圖示永遠不動、尾端空間不夠就轉過角落成 L」，
 * 不再有「改由頭端延長」，所以改為回傳延長後佔幾條邊，圖示位移的斷言不變）。
 */
function hoverIconDrift(sc: number, len: number) {
  const head = preferredHead(pieces(st, centerToStart(sc, len), len - T))
  const extended = hoverTarget(st, { s: sc, sTarget: sc }, growDirection(head), true)
  const a = iconAt(sc, len, head)
  const b = iconAt(extended, len + HOVER_EXTRA, head)
  const arms = pieces(st, centerToStart(extended, len + HOVER_EXTRA), len + HOVER_EXTRA - T).length
  return { arms, drift: Math.hypot(a.x - b.x, a.y - b.y) }
}

describe('AC-EN-11 hover 延長時頭端不動（QA 第 3 輪）', () => {
  const len = restLength(TEXT, measure)
  const half = (len - T) / 2
  // 尾端離下一個角落只剩 50 px（< 88）：尾端轉過角落成 L。
  const nearTailCorner: [string, number][] = [
    ['左側靠下（頭在上端）', st.corners.bl + 50 + half],
    ['右側靠下（頭在上端）', st.corners.br - 50 - half],
    ['上方靠右（頭在左端）', st.corners.tr - 50 - half],
    ['下方靠右（頭在左端）', st.corners.br + 50 + half],
  ]

  it('[回歸] 尾端空間足夠時沿尾端延長，圖示位移 ≤ 0.5 px', () => {
    const r = hoverIconDrift(restorePosition(st, { edge: 'left', head: 0.8 }, len), len)
    expect(r.arms).toBe(1)
    expect(r.drift).toBeLessThanOrEqual(0.5)
  })

  for (const [name, sc] of nearTailCorner) {
    it(`[重現] ${name}：尾端空間不足時尾端轉過角落成 L，圖示所在的頭端不動（≤ 0.5 px）`, () => {
      const r = hoverIconDrift(sc, len)
      // 規格（BDD 場景 11，2026-10-09 決定）：一律往尾端延長，空間不足時尾端沿周長轉過角落成 L 形；頭端每幀位移 ≤ 0.5 px。
      // 修正前是改由頭端延長：把整段往頭端推 88 px，圖示跟著移 88 px。
      expect(r.arms).toBe(2)
      expect(r.drift).toBeLessThanOrEqual(0.5)
    })
  }
})

/**
 * 照 EdgeNotch 的放開流程。修正前：settle() 用拖曳中的靜止長度（從角落開始拖時含轉角弧長）算吸附中心，接著 setCorner 讓
 * 靜止長度改變，restLen effect 以 resizeCenter 保持頭端不動、尾端縮回 → 28 px。修正後（實作者 2026-10-09：流程改了，
 * 斷言不變）：settle() 以 settleRelease 用放開後的最終長度（角落或直邊）同時決定吸附中心與長度，並把記錄的靜止長度同步成
 * 最終長度，restLen effect 不再移動中心。拖曳中的長度不再參與。回傳最終中心與長度。
 */
function releaseFrom(startedAtCorner: boolean, sc: number) {
  const dragLen = restLength(TEXT, measure, startedAtCorner)
  const r = settleRelease(st, sc, restLength(TEXT, measure), restLength(TEXT, measure, true))
  // 從角落開始拖時，拖曳中的長度和放開後的最終長度確實不同（這正是修正前多出 4 px 的來源）。
  expect(r.len === dragLen).toBe(!startedAtCorner)
  return { target: r.center, len: r.len }
}

/** 直條在 edge 上、靠 corner 那一端的帶狀端點到垂直螢幕邊的距離。 */
function gapToCorner(sc: number, len: number, edge: Edge, towardHigherS: boolean): number {
  const end = towardHigherS ? sc + (len - T) / 2 : sc - (len - T) / 2
  const p = pointOnCenterline(st, end)
  if (edge === 'top' || edge === 'bottom') return towardHigherS === (edge === 'top') ? st.W - (p.x + T / 2) : p.x - T / 2
  return towardHigherS === (edge === 'right') ? st.H - (p.y + T / 2) : p.y - T / 2
}

describe('AC-EN-07 放開吸附 (b) 端點距垂直邊 24 px（QA 第 3 輪）', () => {
  const lc = restLength(TEXT, measure, true) - T
  // 跨角但不夠近：中心距角落 0.4 × 長度（> 30%、< 一半＋24）。尾端都在靠角落那一側。
  const cases: { name: string; sc: number; edge: Edge; towardHigherS: boolean }[] = [
    { name: '上方跨右上角（尾端靠角）', sc: st.corners.tr - lc * 0.4, edge: 'top', towardHigherS: true },
    { name: '右側跨右下角（尾端靠角）', sc: st.corners.br - lc * 0.4, edge: 'right', towardHigherS: true },
    { name: '下方跨右下角（尾端靠角）', sc: st.corners.br + lc * 0.4, edge: 'bottom', towardHigherS: false },
    { name: '左側跨左下角（尾端靠角）', sc: st.corners.bl + lc * 0.4, edge: 'left', towardHigherS: false },
  ]

  for (const c of cases) {
    it(`[回歸] ${c.name}：從直邊開始拖，放開後端點距垂直邊 24 px`, () => {
      const r = releaseFrom(false, c.sc)
      expect(gapToCorner(r.target, r.len, c.edge, c.towardHigherS)).toBeCloseTo(EDGE_GAP, 6)
    })

    it(`[重現] ${c.name}：從角落 L 形開始拖，放開後端點仍距垂直邊 24 px`, () => {
      // 拖曳中靜止長度含轉角弧長（corner 狀態只在放開時更新）；吸附算完才縮回直邊長度，尾端退開約 4 px → 28 px。
      const r = releaseFrom(true, c.sc)
      expect(gapToCorner(r.target, r.len, c.edge, c.towardHigherS)).toBeCloseTo(EDGE_GAP, 1)
    })
  }
})

describe('第 2 輪修正彼此不衝突（QA 第 3 輪）', () => {
  const NOW = Date.parse('2026-10-09T10:00:00Z')
  const at = (ms: number) => new Date(NOW + ms).toISOString()
  function q(id: string, forceIn: number, extra: Partial<Question> = {}): Question {
    return {
      id,
      title: id,
      question_line: `${id} 的題目`,
      body_md: '',
      kind: 'choice',
      tier: 't1',
      requested_tier: 't1',
      evidence: [],
      options: [{ id: 'a', label: 'A' }],
      allow_free_text: false,
      default_action: { on_expire: 'noop_log' },
      wake_on_answer: { mode: 'none' },
      source: { agent: 'claude', cwd: '/w' },
      state: 'pending',
      snooze_count: 0,
      forced_today_used: false,
      first_proposed: '2026-10-09',
      last_seen: at(0),
      created_at: at(forceIn - 15 * 60_000),
      cycle_started_at: at(forceIn - 15 * 60_000),
      warn_at: at(forceIn - 3 * 60_000),
      force_at: at(forceIn),
      ...extra,
    } as Question
  }

  it('[回歸] forced 時不顯示「時間到了」，延後原因（即使殘留）也不影響文字；warned 到期才用原因', () => {
    const stale = { reasons: ['screen_shared' as const] }
    const asking = notchRestState([q('a', -5_000, { state: 'forced' })], NOW, stale)
    expect(asking.kind).toBe('asking')
    expect(restText(asking)).toBe('你有一則待處理通知')
    expect(notchLabel([q('a', -5_000, { state: 'forced' })], asking)).toContain('最早一題正在詢問')
    const deferred = notchRestState([q('a', -5_000, { state: 'warned' })], NOW, stale)
    expect(restText(deferred)).toBe('時間到了，分享螢幕結束就跳出來')
    // 勿擾／安靜時段的時間以台北時間顯示
    const quiet = notchRestState([q('a', -5_000, { state: 'warned' })], NOW, {
      reasons: ['quiet_hours'],
      quiet_until: '2026-10-10T01:00:00Z',
    })
    expect(restText(quiet)).toBe('時間到了，09:00 安靜時段結束就跳出來')
  })

  it('[回歸] 只剩 internal 題時回報空命中區（notchAnchor 量不到圖示就不送）', () => {
    const doctor = q('doctor', 600_000, { internal: true })
    expect(notchHitReport([doctor], [[0, 0, 32, 0, 32, 100]], true)).toEqual({ shapes: [], dragging: false })
  })

  it('[回歸] 新保存格式：savePosition 寫出的值一定能被 parseSavedPosition 讀回，四邊與四角', () => {
    const len = restLength(TEXT, measure)
    for (const pos of [
      { edge: 'top', head: 0.3 },
      { edge: 'right', head: 0.4 },
      { edge: 'bottom', head: 0.7 },
      { edge: 'left', head: 0.6 },
    ] as const) {
      const sc = restorePosition(st, pos, len)
      const saved = savePosition(st, sc, len)
      expect(parseSavedPosition(JSON.parse(JSON.stringify(saved)))).toEqual(saved)
      expect('head' in saved && saved.head).toBeCloseTo(pos.head, 9)
    }
    for (const corner of ['tl', 'tr', 'br', 'bl'] as const) {
      expect(savePosition(st, st.corners[corner], restLength(TEXT, measure, true))).toEqual({ corner })
    }
  })

  it('[回歸] 英數字詞與跑馬燈純文字', () => {
    for (const w of ['NT$5,940', '-12.5%', '23:30', 'example.com/r?a=1']) {
      expect(tokenize(w, measure).map((t) => t.kind)).toEqual(['latin'])
    }
    expect(plainLine('CPA<200 且 ROAS>3 時加碼？')).toBe('CPA<200 且 ROAS>3 時加碼？')
    expect(plainLine('看 <https://example.com/r>')).toBe('看 https://example.com/r')
  })
})
