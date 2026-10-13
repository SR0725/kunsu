import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { Question } from '../types'
import { CapsuleBar } from './CapsuleBar'
import { ForceCycleRing } from './ForceCycleRing'

function warningQuestion(): Question {
  const now = Date.now()
  const startedAt = new Date(now - 12 * 60_000).toISOString()
  return {
    id: 'q-warning',
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
    state: 'warned',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-08-05',
    last_seen: startedAt,
    created_at: startedAt,
    next_transition_at: new Date(now + 3 * 60_000).toISOString(),
    cycle_started_at: startedAt,
    warn_at: new Date(now - 1_000).toISOString(),
    force_at: new Date(now + 3 * 60_000).toISOString(),
    cycle_number: 1,
  }
}

describe('ForceCycleRing accessibility contract', () => {
  it('keeps the visual timer quiet and exposes one stable warning announcement', () => {
    const html = renderToStaticMarkup(
      createElement(ForceCycleRing, { question: warningQuestion(), variant: 'capsule' }),
    )
    expect(html).toContain('role="timer"')
    expect(html).toContain('aria-live="off"')
    expect(html).toContain('role="status"')
    expect(html).toContain('剩餘 3 分鐘，之後將開啟強制詢問')
  })
})

describe('capsule geometry contract', () => {
  it('keeps the capsule at the documented 44px height', () => {
    const html = renderToStaticMarkup(
      createElement(CapsuleBar, {
        questions: [warningQuestion()],
        onOpen: () => undefined,
      }),
    )
    expect(html).toContain('--capsule-height:44px')
  })
})
