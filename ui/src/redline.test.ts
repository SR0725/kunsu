// 紅線重檢計算（與 executor.py 護欄同值：>NT$2,000 或偏離 >20%）。

import { describe, expect, it } from 'vitest'
import {
  checkRedline,
  parseMoneyChange,
  recalcLine,
  recheckAfterEdit,
} from './redline'

describe('parseMoneyChange', () => {
  it('解析「300 → 240」', () => {
    const c = parseMoneyChange('日預算 300 → 240（-20%）')
    expect(c).not.toBeNull()
    expect(c!.from).toBe(300)
    expect(c!.to).toBe(240)
  })

  it('支援 NT$ 前綴與千分位', () => {
    const c = parseMoneyChange('adset 441｜日預算 NT$3,000 → NT$3,600（+20%）')
    expect(c!.from).toBe(3000)
    expect(c!.to).toBe(3600)
  })

  it('支援 ASCII 箭頭 ->', () => {
    const c = parseMoneyChange('budget 500 -> 650')
    expect(c!.from).toBe(500)
    expect(c!.to).toBe(650)
  })

  it('非金額變更（ACTIVE → PAUSED）回 null', () => {
    expect(parseMoneyChange('entity 120000000000000001｜ACTIVE → PAUSED｜可逆')).toBeNull()
  })
})

describe('recalcLine：編輯後整行重算（所見即所簽）', () => {
  it('替換目標值並重算百分比', () => {
    const out = recalcLine('日預算 300 → 240（-20%）', 330)
    expect(out).toContain('300 → 330')
    expect(out).toContain('（+10%）')
  })

  it('千分位輸出', () => {
    const out = recalcLine('日預算 NT$3,000 → NT$3,600（+20%）', 4500)
    expect(out).toContain('3,000 → NT$4,500')
    expect(out).toContain('（+50%）')
  })

  it('無百分比標註的行只換數字', () => {
    expect(recalcLine('budget 500 -> 650', 550)).toBe('budget 500 -> 550')
  })

  it('解析不到時原樣返回', () => {
    const line = 'entity｜ACTIVE → PAUSED'
    expect(recalcLine(line, 100)).toBe(line)
  })
})

describe('checkRedline：紅線邊界', () => {
  it('偏離恰 20% 不升紅（邊界含）', () => {
    expect(checkRedline(300, 360).breached).toBe(false)
    expect(checkRedline(300, 240).breached).toBe(false)
  })

  it('偏離 >20% 升紅', () => {
    const r = checkRedline(300, 150)
    expect(r.breached).toBe(true)
    expect(r.reasons.join()).toContain('偏離')
  })

  it('變更額恰 NT$2,000 不升紅；2,001 升紅', () => {
    expect(checkRedline(100_000, 102_000).breached).toBe(false)
    const r = checkRedline(100_000, 102_001)
    expect(r.breached).toBe(true)
    expect(r.reasons.join()).toContain('NT$2,001')
  })

  it('兩條同時超線 → 兩個理由都列', () => {
    const r = checkRedline(3000, 6000)
    expect(r.breached).toBe(true)
    expect(r.reasons).toHaveLength(2)
  })

  it('from=0 無法算偏離（deviationPct=null），只看變更額', () => {
    const ok = checkRedline(0, 1500)
    expect(ok.deviationPct).toBeNull()
    expect(ok.breached).toBe(false)
    expect(checkRedline(0, 2500).breached).toBe(true)
  })
})

describe('recheckAfterEdit：編輯 → 重算 + 重檢一體', () => {
  it('+20% 改 +10%：行重算、不升紅', () => {
    const r = recheckAfterEdit('日預算 300 → 360（+20%）', 330)
    expect(r).not.toBeNull()
    expect(r!.line).toContain('300 → 330')
    expect(r!.redline.breached).toBe(false)
  })

  it('編輯到偏離 50% → 升紅', () => {
    const r = recheckAfterEdit('日預算 300 → 360（+20%）', 450)
    expect(r!.redline.breached).toBe(true)
  })

  it('不可編輯行回 null', () => {
    expect(recheckAfterEdit('ACTIVE → PAUSED', 100)).toBeNull()
  })
})
