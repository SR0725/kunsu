import { describe, expect, it } from 'vitest'
import type { Question } from './types'
import { compareForceQueue, forceCycleSnapshot } from './forceCycle'

function question(overrides: Partial<Question> = {}): Question {
  return {
    id: 'q-1',
    title: '需要你的決定',
    question_line: '現在要採用哪個方案？',
    body_md: '',
    kind: 'choice',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [
      { id: 'a', label: 'A', detail: '' },
      { id: 'b', label: 'B', detail: '' },
    ],
    allow_free_text: true,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'none' },
    source: { agent: 'codex', cwd: '/tmp/kunsu' },
    state: 'pending',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-08-05',
    last_seen: '2026-08-05T12:00:00Z',
    created_at: '2026-08-05T12:00:00Z',
    next_transition_at: '2026-08-05T12:12:00Z',
    cycle_started_at: '2026-08-05T12:00:00Z',
    warn_at: '2026-08-05T12:12:00Z',
    force_at: '2026-08-05T12:15:00Z',
    cycle_number: 1,
    ...overrides,
  }
}

describe('forceCycleSnapshot', () => {
  it('tracks a clamped 100 → 0 ring and enters the three-minute warning', () => {
    expect(forceCycleSnapshot(question(), Date.parse('2026-08-05T12:00:00Z'))?.progress).toBe(1)
    const warning = forceCycleSnapshot(question(), Date.parse('2026-08-05T12:12:00Z'))!
    expect(warning.progress).toBeCloseTo(0.2)
    expect(warning.warning).toBe(true)
    expect(warning.label).toContain('3 分鐘後強制詢問')
    const overdue = forceCycleSnapshot(question(), Date.parse('2026-08-05T12:16:00Z'))!
    expect(overdue.progress).toBe(0)
    expect(overdue.overdue).toBe(true)
  })

  it('clamps wall-clock jumps and has no cycle for internal questions', () => {
    expect(forceCycleSnapshot(question(), Date.parse('2026-08-05T11:00:00Z'))?.progress).toBe(1)
    expect(forceCycleSnapshot(question({ internal: true }), Date.parse('2026-08-05T12:15:00Z'))).toBeNull()
  })
})

describe('force queue', () => {
  it('orders earliest overdue first, then risk, then creation time', () => {
    const qs = [
      question({ id: 'new-t0', tier: 't0', force_at: '2026-08-05T12:20:00Z' }),
      question({ id: 'late-t2', tier: 't2', force_at: '2026-08-05T12:10:00Z' }),
      question({ id: 'late-t0', tier: 't0', force_at: '2026-08-05T12:10:00Z' }),
    ]
    expect(qs.sort((a, b) => compareForceQueue(a, b, Date.parse('2026-08-05T12:30:00Z'))).map((q) => q.id))
      .toEqual(['late-t0', 'late-t2', 'new-t0'])
  })
})
