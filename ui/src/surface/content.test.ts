// AC-DS-02/03/08：為什麼問你、證據與版型判斷。
import { describe, expect, it } from 'vitest'
import { evidenceMarkdown, hasEvidence, rawSource, whyContent } from './content'
import type { Question } from '../types'

function q(partial: Partial<Question>): Question {
  return {
    id: 'q', title: 'Acme 停投 18 天', question_line: '要重啟嗎？', body_md: '', kind: 'choice', tier: 't1',
    requested_tier: 't1', evidence: [], options: [], allow_free_text: true,
    default_action: { on_expire: 'noop_log' }, wake_on_answer: { mode: 'none' },
    source: { agent: 'claude', cwd: '/w' }, state: 'pending', snooze_count: 0, forced_today_used: false,
    first_proposed: '2026-10-08', last_seen: 'x', created_at: 'x', next_transition_at: 'x',
    ...partial,
  } as Question
}

describe('whyContent', () => {
  it('turns a list whose items start with times into a timeline, last “現在” highlighted', () => {
    const w = whyContent(q({
      why_md: '每日復盤發現停投 18 天。\n\n1. 09:30 讀取 Meta 後台\n2. 09:31 發現名單成本上升 38%\n3. 現在 等你決定',
    }))
    expect(w.fromWhy).toBe(true)
    expect(w.blocks[0]).toEqual({ kind: 'text', markdown: '每日復盤發現停投 18 天。' })
    expect(w.blocks[1]).toEqual({
      kind: 'timeline',
      items: [
        { time: '09:30', text: '讀取 Meta 後台', now: false },
        { time: '09:31', text: '發現名單成本上升 38%', now: false },
        { time: '現在', text: '等你決定', now: true },
      ],
    })
  })

  it('keeps ordinary lists and quotes as markdown; highlights the last item when no “現在”', () => {
    const w = whyContent(q({ why_md: '- 一般清單\n- 第二項\n\n> 昨天的 AI 回饋\n\n- 08:00 起床\n- 09:00 開會' }))
    expect(w.blocks[0]).toEqual({ kind: 'text', markdown: '- 一般清單\n- 第二項' })
    expect(w.blocks[1]).toEqual({ kind: 'text', markdown: '> 昨天的 AI 回饋' })
    const tl = w.blocks[2]
    expect(tl.kind).toBe('timeline')
    if (tl.kind === 'timeline') expect(tl.items.map((i) => i.now)).toEqual([false, true])
  })

  it('old questions fall back to title + first body paragraph, and evidence keeps the rest', () => {
    const old = q({ body_md: '連三天零成交。\n\n細節一\n\n細節二' })
    const w = whyContent(old)
    expect(w.fromWhy).toBe(false)
    expect(w.blocks).toEqual([{ kind: 'text', markdown: '**Acme 停投 18 天**\n\n連三天零成交。' }])
    expect(evidenceMarkdown(old)).toBe('細節一\n\n細節二')
    expect(evidenceMarkdown(q({ why_md: '脈絡', body_md: '全部證據' }))).toBe('全部證據')
  })
})

describe('hasEvidence', () => {
  it('two columns only when there is something to look at', () => {
    expect(hasEvidence(q({ why_md: '脈絡', body_md: '' }))).toBe(false)
    expect(hasEvidence(q({ why_md: '脈絡', body_md: '| a | b |\n|---|---|\n| 1 | 2 |' }))).toBe(true)
    expect(hasEvidence(q({ why_md: '脈絡', evidence: [{ text: 'ROAS 3.2', source: 'Meta' }] }))).toBe(true)
    expect(hasEvidence(q({ body_md: '只有一段，會被拿去當脈絡' }))).toBe(false)
  })

  it('有畫布就算有證據（AC-EC-07 版型）', () => {
    const draft = { revision: 1, content_hash: 'sha256:x', canvas: 'tsx' as const, registered_at: '2026-10-08T00:00:00Z' }
    expect(hasEvidence(q({ why_md: '脈絡', body_md: '', draft }))).toBe(true)
    expect(hasEvidence(q({ why_md: '脈絡', body_md: '', draft: { ...draft, canvas: undefined } }))).toBe(false)
  })
})

describe('rawSource', () => {
  it('reports exactly what the agent sent and its size', () => {
    const r = rawSource(q({ why_md: '脈絡', body_md: '證據\n第二行' }))
    expect(r.text).toBe('脈絡\n\n證據\n第二行')
    expect(r.lines).toBe(4)
    expect(r.chars).toBe(r.text.length)
  })
})
