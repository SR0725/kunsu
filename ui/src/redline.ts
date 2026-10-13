// 紅線重檢（設計正本 §4 / BDD answer-flow 場景 3 / AC-AN-04）。
// 與 executor.py 護欄同值：變更額 > NT$2,000 或偏離 > 20% → 升 red 並鎖一鍵。
// 純函數，UI 與 mock adapter 共用；真正的最後防線永遠在 executor.py（本檔只做視覺升紅與鎖鍵）。

/** 與 executor.py 同值的紅線常數。 */
export const REDLINE_CHANGE_TWD = 2000
export const REDLINE_DEVIATION = 0.2

export interface MoneyChange {
  /** 現值（變更前）。 */
  from: number
  /** 目標值（變更後）。 */
  to: number
  /** match 的起訖（recalcLine 重寫用）。 */
  start: number
  end: number
}

const CHANGE_RE = /(?:NT\$\s*)?([\d,]+)\s*(?:→|->)\s*(?:NT\$\s*)?([\d,]+)/

function parseNum(s: string): number {
  return Number(s.replace(/,/g, ''))
}

/**
 * 從 params_line / 選項 label 解析「X → Y」金額變更。
 * 支援 NT$ 前綴與千分位；解析不到（如 ACTIVE → PAUSED）回 null。
 */
export function parseMoneyChange(line: string): MoneyChange | null {
  const m = CHANGE_RE.exec(line)
  if (!m) return null
  const from = parseNum(m[1])
  const to = parseNum(m[2])
  if (!Number.isFinite(from) || !Number.isFinite(to)) return null
  return { from, to, start: m.index, end: m.index + m[0].length }
}

function fmt(n: number): string {
  return n.toLocaleString('en-US')
}

/**
 * 鉛筆編輯後重算整行（所見即所簽）：
 * 替換「→ 後的目標值」，並同步重算行內既有的（±N%）標註。
 */
export function recalcLine(line: string, newTo: number): string {
  const c = parseMoneyChange(line)
  if (!c) return line
  const matched = line.slice(c.start, c.end)
  const rewritten = matched.replace(/((?:→|->)\s*(?:NT\$\s*)?)([\d,]+)/, `$1${fmt(newTo)}`)
  let out = line.slice(0, c.start) + rewritten + line.slice(c.end)
  // 重算既有百分比標註（（+20%）/ (-20%) 兩種括號都認）
  const pctRe = /[（(]\s*[+\-−]?\d+(?:\.\d+)?\s*%\s*[)）]/
  if (pctRe.test(out) && c.from > 0) {
    const pct = ((newTo - c.from) / c.from) * 100
    const sign = pct > 0 ? '+' : pct < 0 ? '-' : ''
    const rounded = Math.round(Math.abs(pct) * 10) / 10
    out = out.replace(pctRe, `（${sign}${rounded}%）`)
  }
  return out
}

export interface RedlineResult {
  /** true = 超紅線：視覺升 red、鎖一鍵（只能走使用者終端通道）。 */
  breached: boolean
  changeAbs: number
  /** from=0 時無法算偏離，回 null。 */
  deviationPct: number | null
  reasons: string[]
}

/** 紅線重檢：邊界值（恰 2,000 / 恰 20%）不算超線，嚴格大於才升紅。 */
export function checkRedline(from: number, to: number): RedlineResult {
  const changeAbs = Math.abs(to - from)
  const deviationPct = from > 0 ? changeAbs / from : null
  const reasons: string[] = []
  if (changeAbs > REDLINE_CHANGE_TWD) {
    reasons.push(`變更額 NT$${fmt(changeAbs)} 超過紅線 NT$${fmt(REDLINE_CHANGE_TWD)}`)
  }
  if (deviationPct !== null && deviationPct > REDLINE_DEVIATION) {
    reasons.push(`偏離 ${Math.round(deviationPct * 1000) / 10}% 超過 20% 上限`)
  }
  return { breached: reasons.length > 0, changeAbs, deviationPct, reasons }
}

/** 對一行 params/選項做「編輯成 newTo」的完整重檢（重算行 + 紅線）。 */
export function recheckAfterEdit(
  line: string,
  newTo: number,
): { line: string; redline: RedlineResult } | null {
  const c = parseMoneyChange(line)
  if (!c) return null
  return { line: recalcLine(line, newTo), redline: checkRedline(c.from, newTo) }
}
