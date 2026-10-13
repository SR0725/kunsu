// 督導頁渲染（AC-RC-05、06、08）：按鈕語意、來源圖示可讀名稱、逾時預設、回饋 sanitizer、沒有回覆框。
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { RoutinesView } from './RoutinesView'

const NOW = Date.parse('2026-10-08T12:00:00Z')
const hoursAgo = (h: number) => new Date(NOW - h * 3_600_000).toISOString()
const ipc = { openDeepLink: async () => {} } as unknown as KunsuIpc

function run(id: string, hours: number, partial: Partial<Question> = {}): Question {
  return {
    id,
    title: 't',
    question_line: `題目 ${id}`,
    body_md: '',
    kind: 'choice',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [{ id: 'y', label: '有做到' }, { id: 'n', label: '沒做到' }],
    allow_free_text: true,
    default_action: { on_expire: 'pick_option', option_id: 'y' },
    wake_on_answer: { mode: 'continue' },
    source: { agent: 'codex', cwd: '/w', routine_id: 'goals', routine_name: '每週目標檢討', trigger: 'scheduled', expected_every_hours: 168 },
    state: 'pending',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-08',
    last_seen: hoursAgo(hours),
    created_at: hoursAgo(hours),
    next_transition_at: hoursAgo(hours),
    ...partial,
  } as Question
}

const runs = [
  run('rt-goals-2', 2, {
    state: 'answered',
    answer: {
      answer_id: 'a2',
      choice_id: 'n',
      free_text: '<b>粗</b> https://example.com/' + 'x'.repeat(300),
      channel: 'ui',
      answered_at: hoursAgo(1),
      report: { status: 'done', summary_md: '已更新 **goals**<script>alert(1)</script><img src=x onerror="alert(2)">', reported_at: hoursAgo(0.5) },
    },
  }),
  run('rt-goals-1', 170, { state: 'expired', answer: { answer_id: 'a1', choice_id: 'y', channel: 'auto_policy', answered_at: hoursAgo(160) } }),
]

describe('督導頁列表', () => {
  it('每列是清單裡的按鈕，來源圖示有可讀名稱', () => {
    const html = renderToStaticMarkup(<RoutinesView ipc={ipc} questions={runs} nowMs={NOW} />)
    // 2026-10-09 主視窗改 dub.co 語言（AC-AS-06）：列表是表格列（k-table／k-row），不再測舊的 question-list class 名稱。
    expect(html).toMatch(/<ul[^>]*aria-label="排程"/)
    expect(html).toMatch(/<li[^>]*><button type="button" class="k-row/)
    expect(html).toMatch(/class="k-pill"[^>]*data-tone="[a-z]+"[^>]*>.*?<svg[^>]*aria-hidden="true"/)
    expect(html).not.toMatch(/<button[^>]*role=/)
    expect(html).toMatch(/<svg[^>]*role="img"[^>]*aria-label="Codex"/)
  })
})

describe('督導頁歷史', () => {
  const html = renderToStaticMarkup(<RoutinesView ipc={ipc} questions={runs} nowMs={NOW} initialRoutine="goals" />)

  it('標題可接收焦點；逾時預設不算回答；每次 check-in 是一張卡片', () => {
    expect(html).toMatch(/<h1[^>]*tabindex="-1"/)
    expect(html.match(/<li[^>]*class="k-card routine-entry"/g)).toHaveLength(2)
    expect(html).toContain('沒有回答（逾時依預設：有做到）')
    expect(html).not.toContain('>有做到<')
  })

  it('回答是純文字、回饋經 sanitizer、沒有回覆框', () => {
    expect(html).toContain('沒做到（&lt;b&gt;粗&lt;/b&gt;')
    expect(html).toContain('<strong>goals</strong>')
    expect(html).not.toMatch(/<script|onerror/)
    expect(html).not.toMatch(/<textarea|<input|<form|contenteditable/)
  })
})
