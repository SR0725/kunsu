// QA 第 5 輪（排程督導，BDD routine-check-in AC-RC-06）：main 新合進的題組（kind: batch）與純確認題（choice_id = "confirm"）
// 在督導頁歷史「你的回答」那一欄的顯示。斷言寫規格期望；失敗訊息寫實際行為。
import { describe, expect, it } from 'vitest'
import type { Question } from '../types'
import { answerLine, routineHistory, summarizeRoutines } from './routines'

const NOW = Date.parse('2026-10-09T12:00:00Z')
const hoursAgo = (h: number) => new Date(NOW - h * 3_600_000).toISOString()

function checkIn(id: string, partial: Partial<Question>): Question {
  return {
    id,
    title: 't',
    question_line: `題目 ${id}`,
    body_md: '',
    kind: 'text',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [],
    allow_free_text: true,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'continue' },
    source: { agent: 'claude', cwd: '/w', routine_id: 'weekly', trigger: 'scheduled', routine_name: '每週回信', expected_every_hours: 168 },
    state: 'answered',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-09',
    last_seen: hoursAgo(3),
    created_at: hoursAgo(3),
    next_transition_at: hoursAgo(3),
    ...partial,
  } as Question
}

describe('QA5 督導頁：題組與確認題的回答', () => {
  // R5-1：題組 check-in 已回答（狀態「已回應」），歷史的「你的回答」卻寫「沒有回答」——answerText 只看 choice_id／free_text，
  // 題組的答案在 answer.items。規格 AC-RC-06：歷史列出使用者的完整回答；未回應才標「沒有回答」。
  it('R5-1 題組 check-in 的回答不是「沒有回答」', () => {
    const q = checkIn('rt-weekly-1009', {
      kind: 'batch',
      items: [
        { id: 'i1', title: '陳同學', context_md: '來信', proposal: '草稿一', editable: true, content_hash: 'sha256:a' },
        { id: 'i2', title: '某品牌窗口', context_md: '來信', proposal: '草稿二', editable: true, content_hash: 'sha256:b' },
      ],
      answer: {
        answer_id: 'a1',
        channel: 'ui',
        answered_at: hoursAgo(2),
        items: [
          { item_id: 'i1', action: 'approve', text: '改過的回信', content_hash: 'sha256:a' },
          { item_id: 'i2', action: 'skip', content_hash: 'sha256:b' },
        ],
        delivery: { state: 'resumed', updated_at: hoursAgo(2) },
      },
    } as Partial<Question>)
    const [row] = summarizeRoutines([q], NOW)
    expect(row.latestStatus).toBe('answered')
    const [entry] = routineHistory([q], 'weekly')
    const line = answerLine(entry)
    expect(line, `題組已回應，歷史「你的回答」卻顯示：${line}`).not.toBe('沒有回答')
    expect(line).not.toBe('還沒回答')
  })

  // R5-2：純確認題 check-in（「今天的日記寫了嗎？」按「確認」）的回答顯示成英文 id「confirm」。決策面板回執與交給 agent 的訊息
  // 都顯示「確認」（CONTRACT §2、DecisionColumn）。規格 AC-RC-06：使用者的完整回答。
  it('R5-2 確認題 check-in 的回答顯示「確認」而不是 confirm', () => {
    const q = checkIn('rt-diary-1009', {
      kind: 'confirm',
      options: [],
      source: { agent: 'claude', cwd: '/w', routine_id: 'diary', trigger: 'scheduled', routine_name: '每天寫日記', expected_every_hours: 24 },
      answer: { answer_id: 'a2', choice_id: 'confirm', channel: 'ui', answered_at: hoursAgo(2) },
    } as Partial<Question>)
    const [entry] = routineHistory([q], 'diary')
    const line = answerLine(entry)
    expect(line, `確認題的回答顯示成：${line}`).toBe('確認')
  })
})
