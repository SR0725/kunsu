// 卡片 lint 顯示邏輯：六區缺一的 malformed 呈現（CONTRACT §2 十秒可決 lint 鏡像）。

import { describe, expect, it } from 'vitest'
import { lintCard, checkOneClick } from './lint'
import type { Question } from './types'

function baseQuestion(over: Partial<Question> = {}): Question {
  return {
    id: 'pa-test-0001',
    title: '測試題',
    question_line: '要不要加碼？',
    body_md: '',
    kind: 'choice',
    tier: 't0',
    requested_tier: 't0',
    product: 'nova',
    gate: 'yellow',
    evidence: [{ text: 'ROAS 3.2 連三日', source: 'meta-api' }],
    params_line: undefined,
    options: [
      { id: 'a', label: '加碼 20%', recommended: true },
      { id: 'b', label: '維持現狀' },
    ],
    allow_free_text: true,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'none' },
    source: { agent: 'claude', cwd: '/tmp' },
    state: 'pending',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-07-31',
    last_seen: '2026-07-31T06:00:00Z',
    created_at: '2026-07-31T06:00:00Z',
    next_transition_at: '2026-07-31T06:00:00Z',
    ...over,
  }
}

function moneyQuestion(over: Partial<Question> = {}): Question {
  return baseQuestion({
    money: { is_money_op: true, amount_twd: 600, entity_id: 'adset-123' },
    prediction_ref: 'projects/meta-ads/predictions/2026-07-31-test.md',
    params_line: 'adset 123 日預算 500 → 600（+20%）',
    ...over,
  })
}

describe('lintCard：完整卡通過', () => {
  it('六區齊全 → ok、非 malformed', () => {
    const r = lintCard(moneyQuestion())
    expect(r.ok).toBe(true)
    expect(r.malformed).toBe(false)
    expect(r.missingZones).toEqual([])
  })

  it('證據空 → 不算 malformed，但標 evidenceEmpty（區內畫佔位）', () => {
    const r = lintCard(baseQuestion({ evidence: [] }))
    expect(r.malformed).toBe(false)
    expect(r.evidenceEmpty).toBe(true)
  })
})

describe('lintCard：六區缺一的 malformed 呈現', () => {
  it('缺問題句 → question 區缺損', () => {
    const r = lintCard(baseQuestion({ question_line: '  ' }))
    expect(r.malformed).toBe(true)
    expect(r.missingZones).toContain('question')
    expect(r.problems.map((p) => p.code)).toContain('question_line_missing')
  })

  it('問題句 >24 字 → question 區缺損', () => {
    const r = lintCard(baseQuestion({ question_line: '這是一句超過二十四個字上限的超長非決策句子所以要被擋下來' }))
    expect(r.malformed).toBe(true)
    expect(r.problems.map((p) => p.code)).toContain('question_line_too_long')
  })

  it('證據缺身世標籤 → evidence 區缺損', () => {
    const r = lintCard(
      baseQuestion({ evidence: [{ text: '連 3 天成交 0', source: '' }] }),
    )
    expect(r.malformed).toBe(true)
    expect(r.missingZones).toContain('evidence')
  })

  it('證據超過 3 條 → evidence 區缺損', () => {
    const e = { text: 'x', source: 's' }
    const r = lintCard(baseQuestion({ evidence: [e, e, e, e] }))
    expect(r.problems.map((p) => p.code)).toContain('evidence_too_many')
  })

  it('動錢題缺參數行 → params 區缺損（十秒可決 lint）', () => {
    const r = lintCard(moneyQuestion({ params_line: undefined }))
    expect(r.malformed).toBe(true)
    expect(r.missingZones).toContain('params')
    expect(r.problems.map((p) => p.code)).toContain('money_missing_params')
  })

  it('動錢題缺 prediction_ref → params 區缺損（INV-1 鏡像）', () => {
    const r = lintCard(moneyQuestion({ prediction_ref: undefined }))
    expect(r.malformed).toBe(true)
    expect(r.problems.map((p) => p.code)).toContain('money_missing_prediction')
  })

  it('choice 題選項只有 1 個 → options 區缺損（十秒可決 lint）', () => {
    const r = lintCard(baseQuestion({ options: [{ id: 'a', label: '唯一選項' }] }))
    expect(r.malformed).toBe(true)
    expect(r.missingZones).toContain('options')
    expect(r.problems.map((p) => p.code)).toContain('choice_options_out_of_range')
  })

  it('choice 題選項 5 個 → options 區缺損', () => {
    const opts = [1, 2, 3, 4, 5].map((n) => ({ id: `o${n}`, label: `選項 ${n}` }))
    const r = lintCard(baseQuestion({ options: opts }))
    expect(r.problems.map((p) => p.code)).toContain('choice_options_out_of_range')
  })

  it('紅燈題非 choice → options 區缺損（INV-11 鏡像）', () => {
    const r = lintCard(baseQuestion({ gate: 'red', kind: 'confirm', options: [] }))
    expect(r.malformed).toBe(true)
    expect(r.problems.map((p) => p.code)).toContain('red_gate_needs_choice')
  })

  it('pick_option 指到不存在選項 → footer 區缺損', () => {
    const r = lintCard(
      baseQuestion({ default_action: { on_expire: 'pick_option', option_id: 'ghost' } }),
    )
    expect(r.malformed).toBe(true)
    expect(r.missingZones).toContain('footer')
  })
})

describe('checkOneClick：一鍵准入', () => {
  it('黃燈動錢題（prediction 齊）→ 不鎖', () => {
    expect(checkOneClick(moneyQuestion(), false).locked).toBe(false)
  })

  it('紅燈題永不一鍵（INV-11）', () => {
    const r = checkOneClick(moneyQuestion({ gate: 'red' }), false)
    expect(r.locked).toBe(true)
    expect(r.reason).toContain('紅燈')
  })

  it('紅線重檢 breached → 鎖', () => {
    const r = checkOneClick(moneyQuestion(), true)
    expect(r.locked).toBe(true)
  })

  it('動錢缺 prediction → 鎖（鐵律鏡像）', () => {
    const r = checkOneClick(moneyQuestion({ prediction_ref: undefined }), false)
    expect(r.locked).toBe(true)
  })
})
