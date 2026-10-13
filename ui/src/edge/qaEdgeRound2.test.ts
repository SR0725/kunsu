// QA 第 2 輪（BDD edge-notch）：回頭驗證第 1 輪修正，並重現第 2 輪新發現。
// 標「[重現]」的測試以規格要求的行為斷言；目前失敗＝發現仍存在，修好後應轉綠。標「[回歸]」的是第 1 輪修正的守門。
import { describe, expect, it } from 'vitest'
import { plainLine } from './content'
import {
  HOVER_EXTRA,
  ICON,
  OMEGA_LENGTH,
  OMEGA_SETTLE,
  T,
  centerToStart,
  defaultCenter,
  makeStage,
  pieces,
  preferredHead,
  resizeCenter,
  restLength,
  restoreCenter,
  restorePosition,
  savePosition,
  springStep,
  textPath,
  tokenize,
} from './geometry'

const st = makeStage(1440, 900)
const measure = (s: string) => s.length * 7

describe('AC-EN-11 跑馬燈純文字（QA 第 2 輪）', () => {
  it('[重現] 不是 HTML 標籤的 < 與 > 是題目內容，不能被當成標籤刪掉', () => {
    // 決策面板（remark + rehype-raw）把這些顯示成字面文字；瀏海卻把「<200 且 ROAS>」整段刪掉，變成「CPA3 時加碼？」。
    expect(plainLine('CPA<200 且 ROAS>3 時加碼？')).toBe('CPA<200 且 ROAS>3 時加碼？')
    expect(plainLine('日預算 <500 還是 >800？')).toBe('日預算 <500 還是 >800？')
  })

  it('[重現] 自動連結 <https://…> 顯示網址文字，不是整段消失', () => {
    expect(plainLine('報表 <https://example.com/r>')).toBe('報表 https://example.com/r')
  })

  it('[回歸] 真正的 HTML 標籤與實體照樣去掉／還原', () => {
    expect(plainLine('<b>加碼</b> <span class="x">日預算</span>')).toBe('加碼 日預算')
    expect(plainLine('a &lt;b&gt; c')).toBe('a <b> c')
    expect(plainLine('**加碼** [連結](https://example.com)')).toBe('加碼 連結')
  })
})

describe('AC-EN-08 英數字詞（QA 第 2 輪）', () => {
  it('[重現] 金額「NT$5,940」是一個英數字詞：直向段整段一起旋轉、橫向段不被拆出全形間距', () => {
    // 目前拆成 NT｜$｜5｜,｜940：$ 與逗號被當成中文（直立、13.5 px 全形寬），直向段變成「轉、立、轉、立、轉」，
    // 橫向段讀成「NT $ 5 , 940」。
    for (const amount of ['NT$5,940', 'US$1,200', '$500']) {
      const toks = tokenize(amount, measure)
      expect(toks.map((t) => t.s)).toEqual([amount])
      expect(toks[0].kind).toBe('latin')
    }
    const line = tokenize('日預算 NT$5,940 → NT$8,000', measure)
    const amountToks = line.filter((t) => /[A-Za-z0-9$,]/.test(t.s))
    expect(amountToks.every((t) => t.kind === 'latin')).toBe(true)
  })
})

describe('AC-EN-03／10 保存與頭端（QA 第 2 輪）', () => {
  /** 上方直條：頭端在左端（ccw），文字由左往右。 */
  const head = (sc: number, len: number) => centerToStart(sc, len)

  it('[重現] 拖曳後文字長度改變，重啟（或換解析度）還原時頭端要回到重啟前的位置', () => {
    const len1 = 166 // 「你有一則待處理通知」
    const len2 = 143 // 「2:59 後強制詢問」
    const c1 = restorePosition(st, { edge: 'top', ratio: 0.3 })
    expect(preferredHead(pieces(st, centerToStart(c1, len1), len1 - T))).toBe('ccw')
    // 放開時保存（EdgeNotch settle → savePosition，記憶體與檔案都是這個值）。修正後保存的是頭端，需要當下長度定位頭端
    // （實作者 2026-10-09：只改呼叫方式，斷言不變）。
    const saved = savePosition(st, c1, len1)
    // 之後進入 warning：文字變短，頭端不動（resizeCenter），但沒有重新保存。
    const c2 = resizeCenter(st, c1, len1, len2, 'ccw')
    const headBeforeQuit = head(c2, len2)
    expect(headBeforeQuit).toBeCloseTo(head(c1, len1), 6)
    // 重啟（或舞台重新貼合）：restoreCenter 用保存的「中心」還原。
    const c3 = restoreCenter(st, saved, len2, 37)
    expect(Math.abs(head(c3, len2) - headBeforeQuit)).toBeLessThan(0.5)
  })

  it('[回歸] 沒有保存值時預設位置依目前文字長度計算，頂端永遠在選單列下 8 px', () => {
    for (const len of [143, 166, 179]) {
      const sc = restoreCenter(st, null, len, 37)
      expect(sc).toBeCloseTo(defaultCenter(st, len, 37), 6)
      // 左側：頭端是周長終點端（cw），中心線頂點 y = H - T/2 - (s - (2A + B))
      const top = sc + (len - T) / 2
      const y = st.H - T / 2 - (top - (2 * st.A + st.B)) - T / 2
      expect(y).toBeCloseTo(37 + 8, 6)
    }
  })
})

describe('AC-EN-10／11 伸縮時頭端不動（QA 第 2 輪，低嚴重度）', () => {
  /**
   * 照 EdgeNotch 的 kick()：中心 s 以 OMEGA_SETTLE、長度 len 以 OMEGA_LENGTH 各自走彈簧（60 fps）。兩個 ω 不同，
   * 中間過程頭端 = s - (len - T)/2 不是常數。
   */
  function maxHeadDrift(s0: number, len0: number, sTarget: number, lenTarget: number): number {
    let s = s0
    let v = 0
    let len = len0
    let lenV = 0
    const head0 = centerToStart(s0, len0)
    let worst = 0
    for (let t = 0; t < 0.6; t += 1 / 60) {
      ;[s, v] = springStep(st, s, v, sTarget, OMEGA_SETTLE, 1 / 60)
      ;[len, lenV] = springStep(null, len, lenV, lenTarget, OMEGA_LENGTH, 1 / 60)
      worst = Math.max(worst, Math.abs(centerToStart(s, len) - head0))
    }
    return worst
  }

  it('[重現] hover 沿尾端延長 88 px 的過程中，頭端（圖示）位移 ≤ 0.5 px', () => {
    const len = 166
    const sc = restorePosition(st, { edge: 'top', ratio: 0.3 })
    // hoverTarget：中心往尾端移 88/2，長度 +88，頭端在終點應不變。
    const drift = maxHeadDrift(sc, len, sc + HOVER_EXTRA / 2, len + HOVER_EXTRA)
    expect(drift).toBeLessThanOrEqual(0.5)
  })
})

describe('AC-EN-08／10 角落 L 形的靜止長度（QA 第 2 輪，低嚴重度）', () => {
  it('[重現] 停在角落時文字可用長度不比直邊短：最後一個字不會被淡出成半透明', () => {
    // 靜止長度＝文字＋T/2＋28，是照直邊算的；角落 L 形的文字中心線走半徑 10 的圓弧，少了 2r − πr/2 ≈ 4.3 px，
    // 最後一個字落進尾端淡出區（實測「知」不透明度 0.50，直邊 0.93）。
    const text = '你有一則待處理通知'
    // 修正後靜止長度依位置：角落多補轉角弧長（restLength 第三個參數；實作者 2026-10-09：只改長度取得方式，斷言不變）。
    const span = (sc: number, len: number) => {
      const ps = pieces(st, centerToStart(sc, len), len - T)
      const tp = textPath(st, ps, preferredHead(ps))
      return tp.total + T / 2 - 12 - (ICON / 2 + 7)
    }
    const straight = span(restorePosition(st, { edge: 'left', ratio: 0.5 }), restLength(text, measure))
    for (const corner of ['tl', 'tr', 'br', 'bl'] as const) {
      expect(straight - span(st.corners[corner], restLength(text, measure, true))).toBeLessThan(0.5)
    }
  })
})
