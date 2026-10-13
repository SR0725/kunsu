// QA 第 6 輪（排程督導，BDD routine-check-in AC-RC-05／06）：main 新合進的「退回」在督導頁列表與歷史的顯示——退回與 agent 取消
// 分開、30 天另計、只算近 30 天、原因是純文字、全空白原因等於沒寫、退回的題不顯示「AI 尚未回報」。
// 斷言寫規格期望；失敗訊息寫實際行為。
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { RoutinesView } from './RoutinesView'
import { STATUS_LABEL, answerLine, routineHistory, summarizeRoutines } from './routines'

const NOW = Date.parse('2026-10-09T12:00:00Z')
const hoursAgo = (h: number) => new Date(NOW - h * 3_600_000).toISOString()
const ipc = { openDeepLink: async () => {} } as unknown as KunsuIpc

function checkIn(id: string, hours: number, partial: Partial<Question> = {}): Question {
  return {
    id,
    title: 't',
    question_line: `今天最重要的三件事？（${id}）`,
    body_md: '',
    kind: 'text',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [],
    allow_free_text: true,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'continue' },
    source: { agent: 'claude', cwd: '/w', routine_id: 'diary', trigger: 'scheduled', routine_name: '每天寫日記', expected_every_hours: 24 },
    state: 'pending',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-09',
    last_seen: hoursAgo(hours),
    created_at: hoursAgo(hours),
    next_transition_at: hoursAgo(hours),
    ...partial,
  } as Question
}

const rejectedToday = checkIn('rt-diary-1009', 2, {
  state: 'cancelled',
  rejected: { at: hoursAgo(1), reason: '<img src=x onerror="alert(1)"> 今天休假' },
})
const blankReason = checkIn('rt-diary-1008', 26, { state: 'cancelled', rejected: { at: hoursAgo(25), reason: '   ' } })
const agentCancelled = checkIn('rt-diary-1007', 50, { state: 'cancelled' })
const answered = checkIn('rt-diary-1006', 74, {
  state: 'routed',
  answer: { answer_id: 'a6', free_text: '寫完了', channel: 'ui', answered_at: hoursAgo(73), acked_at: hoursAgo(73) },
})
const oldRejected = checkIn('rt-diary-0901', 24 * 38, { state: 'cancelled', rejected: { at: hoursAgo(24 * 38) } })
const all = [rejectedToday, blankReason, agentCancelled, answered, oldRejected]

describe('QA6 督導頁：退回', () => {
  it('R6-U1 最近一次是退回時列表狀態寫「退回」；30 天只算近 30 天的退回，不混進未回應或已取消', () => {
    const row = summarizeRoutines(all, NOW).find((r) => r.routineId === 'diary')!
    expect(row.latestStatus).toBe('rejected')
    expect(STATUS_LABEL[row.latestStatus]).toBe('退回')
    expect({ answered: row.answered30d, missed: row.missed30d, rejected: row.rejected30d }).toEqual({ answered: 1, missed: 0, rejected: 2 })
  })

  it('R6-U2 歷史的回答欄：原因去掉空白後是空的就寫「你退回了這題」', () => {
    const items = routineHistory(all, 'diary')
    expect(items.map((i) => i.status)).toEqual(['rejected', 'rejected', 'cancelled', 'answered', 'rejected'])
    expect(answerLine(items[1])).toBe('你退回了這題')
    expect(answerLine(items[2])).toBe('沒有回答')
  })

  it('R6-U3 渲染：列表與歷史都寫出退回次數與狀態，原因是純文字，退回的題不寫「AI 尚未回報」', () => {
    const list = renderToStaticMarkup(createElement(RoutinesView, { ipc, questions: all, nowMs: NOW }))
    expect(list).toContain('退回 2')
    expect(list).toMatch(/data-status="rejected"[^>]*>退回</)
    const history = renderToStaticMarkup(createElement(RoutinesView, { ipc, questions: all, nowMs: NOW, initialRoutine: 'diary' }))
    expect(history).toContain('退回 2 次')
    expect(history).toContain('你退回了這題：&lt;img src=x onerror=&quot;alert(1)&quot;&gt; 今天休假')
    expect(history).not.toMatch(/<img/)
    const entries = history.split('<li ').slice(1)
    const rejectedEntry = entries.find((e) => e.includes('rt-diary-1009') || e.includes('今天休假'))!
    expect(rejectedEntry).not.toContain('AI 尚未回報')
  })
})
