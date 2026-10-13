// 卡片十秒可決 lint 的 UI 端鏡像（CONTRACT §2 硬驗證 / 設計正本 §4）。
// 正常情況 server 已把 malformed 題擋在門外；UI 仍防禦性檢查——
// 六區缺一的題以 degraded（malformed）呈現：不給一鍵執行，只留
// 「改用文字」「退回重擬」「稍後」逃生口。純函數，vitest 直測。

import type { Question } from './types'

export type Zone = 'header' | 'question' | 'evidence' | 'params' | 'options' | 'footer'

export type LintCode =
  | 'question_line_missing'
  | 'question_line_too_long'
  | 'evidence_missing_source'
  | 'evidence_too_many'
  | 'money_missing_params'
  | 'money_missing_prediction'
  | 'choice_options_out_of_range'
  | 'red_gate_needs_choice'
  | 'pick_option_unknown'

export interface LintProblem {
  code: LintCode
  zone: Zone
  message: string
}

export interface LintResult {
  ok: boolean
  problems: LintProblem[]
  /** 缺損的區（六區呈現時標記）。 */
  missingZones: Zone[]
  /** true → degraded 呈現：無主行動，只留文字/退回/稍後。 */
  malformed: boolean
  /** 空證據不算 malformed，但證據區要畫佔位（soft warning）。 */
  evidenceEmpty: boolean
}

export function lintCard(q: Question): LintResult {
  const problems: LintProblem[] = []
  const push = (code: LintCode, zone: Zone, message: string) =>
    problems.push({ code, zone, message })

  const ql = q.question_line?.trim() ?? ''
  if (ql.length === 0) {
    push('question_line_missing', 'question', '缺問題句：無法十秒可決')
  } else if ([...ql].length > 24) {
    push('question_line_too_long', 'question', `問題句 ${[...ql].length} 字超過 24 字上限`)
  }

  if (q.evidence.length > 3) {
    push('evidence_too_many', 'evidence', `證據 ${q.evidence.length} 條超過 3 條上限`)
  }
  q.evidence.forEach((e, i) => {
    if (!e.source || e.source.trim() === '') {
      push('evidence_missing_source', 'evidence', `證據第 ${i + 1} 行缺身世標籤（來源＋期間）`)
    }
  })

  const isMoney = q.money?.is_money_op === true
  if (isMoney && (!q.params_line || q.params_line.trim() === '')) {
    push('money_missing_params', 'params', '動錢題缺參數區（所見即所簽的那一行）')
  }
  if (isMoney && (!q.prediction_ref || q.prediction_ref.trim() === '')) {
    push('money_missing_prediction', 'params', '動錢題缺配對 prediction（鐵律：不配對不落地）')
  }

  if (q.kind === 'choice' && (q.options.length < 2 || q.options.length > 4)) {
    push(
      'choice_options_out_of_range',
      'options',
      `choice 題選項 ${q.options.length} 個，必須 2–4 個`,
    )
  }
  if (q.gate === 'red' && q.kind !== 'choice') {
    push('red_gate_needs_choice', 'options', '紅燈題必須多選項拍板，不能 confirm/text')
  }

  if (
    q.default_action.on_expire === 'pick_option' &&
    (!q.default_action.option_id || !q.options.some((o) => o.id === q.default_action.option_id))
  ) {
    push('pick_option_unknown', 'footer', '預設動作指到不存在的選項')
  }

  const missingZones = [...new Set(problems.map((p) => p.zone))]
  return {
    ok: problems.length === 0,
    problems,
    missingZones,
    malformed: problems.length > 0,
    evidenceEmpty: q.evidence.length === 0,
  }
}

export interface OneClickCheck {
  locked: boolean
  reason?: string
}

/**
 * 一鍵執行（同意並執行）准入：
 * 只覆蓋 gate=yellow 的動錢題（INV-11：red 永不一鍵）；
 * 紅線重檢 breached 視同升 red；缺 prediction_ref 一律鎖。
 */
export function checkOneClick(q: Question, redlineBreached: boolean): OneClickCheck {
  if (q.gate === 'red') {
    return { locked: true, reason: '紅燈題永不一鍵：拍板只落 ledger，執行走本人終端或互動 session' }
  }
  if (redlineBreached) {
    return { locked: true, reason: '編輯後超紅線（>NT$2,000 或偏離 >20%）：已升紅並鎖一鍵' }
  }
  if (q.money?.is_money_op && !q.prediction_ref) {
    return { locked: true, reason: '缺配對 prediction：錢類決策不允許落地' }
  }
  return { locked: false }
}
