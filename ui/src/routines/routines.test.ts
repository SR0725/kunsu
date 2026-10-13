// 排程督導（BDD routine-check-in AC-RC-05～08）：統計、歷史與「可能沒在跑」都是純函數，時鐘可注入。
import { describe, expect, it } from 'vitest'
import type { Question } from '../types'
import { STATUS_LABEL, answerLine, routineHistory, staleText, summarizeRoutines } from './routines'

const NOW = Date.parse('2026-10-08T12:00:00Z')
const hoursAgo = (h: number) => new Date(NOW - h * 3_600_000).toISOString()

function q(id: string, partial: Partial<Question> & { routine?: string; name?: string; every?: number; agent?: 'claude' | 'codex' }): Question {
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
    source: {
      agent: partial.agent ?? 'claude',
      cwd: '/w',
      ...(partial.routine ? { routine_id: partial.routine, trigger: 'scheduled' } : { trigger: 'interactive' }),
      ...(partial.name ? { routine_name: partial.name } : {}),
      ...(partial.every ? { expected_every_hours: partial.every } : {}),
    },
    state: 'pending',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-08',
    last_seen: hoursAgo(0),
    created_at: hoursAgo(0),
    next_transition_at: hoursAgo(0),
    ...partial,
  } as Question
}

const diary = [
  q('rt-diary-1006', { routine: 'diary', name: '每天寫日記', every: 24, created_at: hoursAgo(54), state: 'expired' }),
  q('rt-diary-1007', {
    routine: 'diary',
    name: '每天寫日記',
    every: 24,
    created_at: hoursAgo(30),
    state: 'answered',
    answer: {
      answer_id: 'a7',
      free_text: '寫完了：三件事是…',
      channel: 'ui',
      answered_at: hoursAgo(29),
      delivery: { state: 'resumed', updated_at: hoursAgo(29) },
      report: { status: 'done', summary_md: '已寫入 **vault 日記**', reported_at: hoursAgo(28) },
    },
  }),
  q('rt-diary-1008', { routine: 'diary', name: '每天寫日記', every: 24, created_at: hoursAgo(2) }),
]
const weekly = [
  q('rt-weekly-0928', { routine: 'weekly', name: '每週目標檢討', every: 168, agent: 'codex', created_at: hoursAgo(400), state: 'cancelled' }),
]
const interactive = q('pa-x', { created_at: hoursAgo(1) })

describe('summarizeRoutines', () => {
  it('每個 routine 一列，依最近報到排序，含狀態與 30 天統計（AC-RC-05）', () => {
    const rows = summarizeRoutines([...diary, ...weekly, interactive], NOW)
    expect(rows.map((r) => r.routineId)).toEqual(['diary', 'weekly'])
    const d = rows[0]
    expect(d).toMatchObject({ name: '每天寫日記', agent: 'claude', everyHours: 24, latestStatus: 'pending', answered30d: 1, missed30d: 1 })
    expect(d.lastCheckInMs).toBe(Date.parse(hoursAgo(2)))
    expect(rows[1]).toMatchObject({ name: '每週目標檢討', agent: 'codex', latestStatus: 'cancelled' })
  })

  it('超過宣告頻率兩倍沒報到 → 可能沒在跑（AC-RC-07）', () => {
    const rows = summarizeRoutines([...diary, ...weekly], NOW)
    expect(rows[0].staleHours).toBeNull()
    expect(rows[1].staleHours).toBe(400)
    expect(staleText(rows[1].staleHours!)).toBe('排程可能沒在跑：已 400 小時沒報到，請到 Claude／Codex 檢查排程設定')
    const late = summarizeRoutines([q('rt-d', { routine: 'd', every: 24, created_at: hoursAgo(49) })], NOW)
    expect(late[0].staleHours).toBe(49)
  })

  it('沒有 routine 名稱時用 routine_id', () => {
    expect(summarizeRoutines([q('rt-n', { routine: 'nameless', every: 24 })], NOW)[0].name).toBe('nameless')
  })
})

describe('退回（AC-RC-05／06，2026-10-09 作者核准）', () => {
  const rejected = q('rt-diary-r', {
    routine: 'diary',
    name: '每天寫日記',
    every: 24,
    created_at: hoursAgo(5),
    state: 'cancelled',
    rejected: { at: hoursAgo(4), reason: '今天休假' },
  })
  const plain = q('rt-diary-p', { routine: 'diary', name: '每天寫日記', every: 24, created_at: hoursAgo(6), state: 'cancelled', rejected: { at: hoursAgo(5) } })

  it('使用者推掉的 check-in 標「退回」，不和 agent 取消混在一起，30 天另計', () => {
    const rows = summarizeRoutines([...diary, rejected, plain, ...weekly], NOW)
    const d = rows.find((r) => r.routineId === 'diary')!
    expect(d).toMatchObject({ answered30d: 1, missed30d: 1, rejected30d: 2 })
    expect(rows.find((r) => r.routineId === 'weekly')!.latestStatus).toBe('cancelled')
    expect(STATUS_LABEL.rejected).toBe('退回')
  })

  it('歷史的回答欄寫「你退回了這題」與原因', () => {
    const items = routineHistory([rejected, plain], 'diary')
    expect(items.map((i) => i.status)).toEqual(['rejected', 'rejected'])
    expect(answerLine(items[0])).toBe('你退回了這題：今天休假')
    expect(answerLine(items[1])).toBe('你退回了這題')
  })
})

describe('routineHistory', () => {
  it('由新到舊，含回答、AI 回饋與交付狀態；未回應／取消不隱藏（AC-RC-06）', () => {
    const items = routineHistory([...diary, ...weekly, interactive], 'diary')
    expect(items.map((i) => i.id)).toEqual(['rt-diary-1008', 'rt-diary-1007', 'rt-diary-1006'])
    expect(items[0]).toMatchObject({ status: 'pending', answer: null, feedback: null })
    expect(items[1]).toMatchObject({ status: 'answered', answer: '寫完了：三件事是…', feedback: '已寫入 **vault 日記**', handoff: '已在原對話追加處理' })
    expect(items[2]).toMatchObject({ status: 'missed', answer: null })
  })

  it('選項題的回答顯示選項文字與補充', () => {
    const choice = q('rt-c', {
      routine: 'c',
      every: 24,
      kind: 'choice',
      options: [{ id: 'y', label: '有做到' }, { id: 'n', label: '沒做到' }],
      state: 'answered',
      answer: { answer_id: 'x', choice_id: 'n', free_text: '今天加班', channel: 'ui', answered_at: hoursAgo(1) },
    })
    expect(routineHistory([choice], 'c')[0].answer).toBe('沒做到（今天加班）')
  })
})

describe('督導頁成員規則（AC-RC-05）', () => {
  it('只列宣告為 check-in 的排程題：scheduled 且有名稱或頻率；只帶 routine_id 的營運決策題不算', () => {
    const ops = q('pa-mkt', {})
    ops.source = { agent: 'claude', cwd: '/w', routine_id: 'mkt-daily-marketing-os', trigger: 'scheduled' }
    const named = q('rt-n', { routine: 'named', name: '每週檢討' })
    const timed = q('rt-t', { routine: 'timed', every: 168 })
    expect(summarizeRoutines([ops, named, timed], NOW).map((r) => r.routineId).sort()).toEqual(['named', 'timed'])
  })

  it('排除 internal、kunsu- 保留前綴、interactive 與沒有 trigger 的舊資料', () => {
    const doctor = q('pa-doctor-1', { routine: 'kunsu-doctor', internal: true })
    const activation = q('pa-kg1-activate', {})
    activation.source = { agent: 'other', cwd: '/vault', routine_id: 'kunsu-kongko' }
    const interactiveWithRoutine = q('pa-i', { routine: 'mkt' })
    interactiveWithRoutine.source = { ...interactiveWithRoutine.source, trigger: 'interactive' }
    const legacy = q('rt-legacy', {})
    legacy.source = { agent: 'claude', cwd: '/w', routine_id: 'legacy', routine_name: '舊資料' }
    const rows = summarizeRoutines([doctor, activation, interactiveWithRoutine, legacy, ...diary], NOW)
    expect(rows.map((r) => r.routineId)).toEqual(['diary'])
    expect(routineHistory([doctor], 'kunsu-doctor')).toEqual([])
  })
})

describe('逾時依預設自動選定（AC-RC-06）', () => {
  it('不算使用者的回答：回答為空、標出預設選項、計入未回應', () => {
    const expired = q('rt-auto', {
      routine: 'g',
      every: 24,
      kind: 'choice',
      options: [{ id: 'y', label: '有做到' }, { id: 'n', label: '沒做到' }],
      default_action: { on_expire: 'pick_option', option_id: 'y' },
      state: 'expired',
      answer: { answer_id: 'x', choice_id: 'y', channel: 'auto_policy', answered_at: hoursAgo(1) },
    })
    const [item] = routineHistory([expired], 'g')
    expect(item).toMatchObject({ status: 'missed', answer: null, autoDefault: '有做到' })
    expect(answerLine(item)).toBe('沒有回答（逾時依預設：有做到）')
    const [row] = summarizeRoutines([expired], NOW)
    expect(row).toMatchObject({ answered30d: 0, missed30d: 1 })
  })

  it('沒有回答時的文字', () => {
    const [pending, , missed] = routineHistory(diary, 'diary')
    expect(answerLine(pending)).toBe('還沒回答')
    expect(answerLine(missed)).toBe('沒有回答')
  })
})

describe('修正前就被收割的舊資料（AC-RC-09）', () => {
  it('routed 但沒有答案算未回應，不是待回應', () => {
    const old = q('rt-old', { routine: 'diary', name: '每天寫日記', every: 24, state: 'routed', created_at: hoursAgo(30) })
    const [item] = routineHistory([old], 'diary')
    expect(item.status).toBe('missed')
    expect(summarizeRoutines([old], NOW)[0]).toMatchObject({ latestStatus: 'missed', missed30d: 1 })
  })
})
