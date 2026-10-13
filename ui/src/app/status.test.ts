// 主視窗的狀態對照（BDD application-shell AC-AS-04／05／06、settings AC-ST-10／11／12）：小標一律有文字與圖示、不只靠顏色。
import { describe, expect, it } from 'vitest'
import type { Question } from '../types'
import {
  HISTORY_FILTERS,
  OUTCOME_PILL,
  closedAt,
  filterHistory,
  formatUptime,
  historyCounts,
  historyOutcome,
  inboxStats,
  integrationName,
  integrationPill,
  ledgerHealth,
  priorityPill,
} from './status'

function q(id: string, partial: Partial<Question> = {}): Question {
  return {
    id,
    title: id,
    question_line: `題目 ${id}`,
    body_md: '',
    kind: 'choice',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [
      { id: 'a', label: '照做' },
      { id: 'b', label: '先不要' },
    ],
    allow_free_text: true,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'none' },
    source: { agent: 'claude', cwd: '/w' },
    state: 'pending',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-01',
    last_seen: '2026-10-08T10:00:00Z',
    created_at: '2026-10-08T09:00:00Z',
    next_transition_at: '2026-10-08T10:00:00Z',
    ...partial,
  } as Question
}

describe('待決事項', () => {
  it('優先小標：現在需要你＝紅、今天＝琥珀，都有文字與圖示', () => {
    expect(priorityPill(q('a', { tier: 't0' }))).toMatchObject({ label: '現在需要你', tone: 'danger' })
    expect(priorityPill(q('b', { tier: 't1' }))).toMatchObject({ label: '今天', tone: 'waiting' })
    expect(priorityPill(q('c', { tier: 't2' }))).toMatchObject({ label: '策略', tone: 'info' }) // AC-AS-16
    expect(priorityPill(q('d', { tier: 't3' }))).toMatchObject({ label: '政策', tone: 'neutral' })
    for (const tier of ['t0', 't1', 't2', 't3'] as const) expect(priorityPill(q('x', { tier })).icon).toBeTruthy()
  })

  it('三格統計：待決、現在需要你、可批次回答', () => {
    expect(inboxStats([q('a', { tier: 't0' }), q('b'), q('c', { tier: 't2' })])).toEqual({ total: 3, urgent: 1, batchable: 2 })
    expect(inboxStats([])).toEqual({ total: 0, urgent: 0, batchable: 0 })
  })
})

describe('歷史紀錄', () => {
  const answered = q('ans', { state: 'answered', answer: { choice_id: 'a', free_text: '週五前', channel: 'ui', answered_at: '2026-10-08T11:00:00Z' } })
  const expired = q('exp', { state: 'expired', last_seen: '2026-10-07T00:00:00Z' })
  const autoDefault = q('auto', {
    state: 'expired',
    default_action: { on_expire: 'pick_option', option_id: 'b' },
    answer: { choice_id: 'b', channel: 'auto_policy', answered_at: '2026-10-07T01:00:00Z' },
  })
  const rejected = q('rej', { state: 'cancelled', rejected: { at: '2026-10-08T12:00:00Z', reason: '這該問同事' } })
  const cancelled = q('can', { state: 'cancelled' })
  const superseded = q('sup', { state: 'superseded' })
  const all = [answered, expired, autoDefault, rejected, cancelled, superseded]

  it('依實際終態分類', () => {
    expect(all.map(historyOutcome)).toEqual(['answered', 'missed', 'missed', 'rejected', 'cancelled', 'superseded'])
  })

  it('篩選小標：全部／已回答／未回應／退回／取消／被取代，各帶數量', () => {
    expect(HISTORY_FILTERS.map((f) => f.label)).toEqual(['全部', '已回答', '未回應', '退回', '取消', '被取代'])
    expect(historyCounts(all)).toEqual({ all: 6, answered: 1, missed: 2, rejected: 1, cancelled: 1, superseded: 1 })
    expect(filterHistory(all, 'missed').map((x) => x.id)).toEqual(['exp', 'auto'])
    expect(filterHistory(all, 'all')).toHaveLength(6)
  })

  it('結果小標有文字、色調與圖示', () => {
    expect(OUTCOME_PILL.answered).toMatchObject({ label: '已回答', tone: 'success' })
    expect(OUTCOME_PILL.missed).toMatchObject({ label: '未回應', tone: 'waiting' })
    expect(OUTCOME_PILL.rejected).toMatchObject({ label: '退回', tone: 'danger' })
    expect(OUTCOME_PILL.cancelled).toMatchObject({ label: '取消', tone: 'neutral' })
    expect(OUTCOME_PILL.superseded).toMatchObject({ label: '被取代', tone: 'info' })
    for (const pill of Object.values(OUTCOME_PILL)) expect(pill.icon).toBeTruthy()
  })

  it('結案時間：回答時間 → 退回時間 → 最後更新', () => {
    expect(closedAt(answered)).toBe('2026-10-08T11:00:00Z')
    expect(closedAt(rejected)).toBe('2026-10-08T12:00:00Z')
    expect(closedAt(expired)).toBe('2026-10-07T00:00:00Z')
  })
})

describe('Agent 連線與資料健康', () => {
  it('四個檢查項目的名稱', () => {
    expect(integrationName('claude_mcp')).toBe('Claude MCP')
    expect(integrationName('codex_mcp')).toBe('Codex MCP')
    expect(integrationName('cli')).toContain('kunsu')
    expect(integrationName('skill')).toContain('skill')
  })

  it('已設定／未設定／無法確認，都有文字', () => {
    expect(integrationPill('ok')).toMatchObject({ label: '已設定', tone: 'success' })
    expect(integrationPill('missing')).toMatchObject({ label: '未設定', tone: 'waiting' })
    expect(integrationPill('unknown')).toMatchObject({ label: '無法確認', tone: 'neutral' })
  })

  it('帳本健康：正常、待補寫、OPEN.json 待重寫、卡住；有問題就用人話說明並建議打開完全取用磁碟', () => {
    expect(ledgerHealth({ backlog: 0, open_json_dirty: false, stuck_s: 0 })).toMatchObject({ label: '正常', tone: 'success', problem: false })
    const backlog = ledgerHealth({ backlog: 3, open_json_dirty: false, stuck_s: 0 })
    expect(backlog).toMatchObject({ label: '有 3 行待補寫', tone: 'waiting', problem: true })
    expect(backlog.explain).toContain('答案都安全在資料庫')
    expect(ledgerHealth({ backlog: 0, open_json_dirty: true, stuck_s: 0 }).label).toBe('OPEN.json 待重寫')
    const stuck = ledgerHealth({ backlog: 2, open_json_dirty: true, stuck_s: 150 })
    expect(stuck).toMatchObject({ label: '寫入卡住 150 秒', tone: 'danger', problem: true })
    expect(stuck.restart).toContain('重新啟動 Kunsu')
    expect(ledgerHealth({ backlog: 1, open_json_dirty: false, stuck_s: 60 }).restart).toBeUndefined()
  })

  it('運作時間用人話', () => {
    expect(formatUptime(42)).toBe('不到 1 分鐘')
    expect(formatUptime(3 * 3600 + 12 * 60)).toBe('3 小時 12 分')
    expect(formatUptime(2 * 86400 + 3600)).toBe('2 天 1 小時')
  })
})
