// 題組面板的結構（BDD batch-review AC-BR-02／03／07／08）：list 語意、狀態有文字、只核可項目沒有輸入框、
// 左欄顯示選中項目的原文、回執逐項列出決定。
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { BatchItem, Question } from '../types'
import { BatchReview } from './BatchColumn'

const ipc = { openDeepLink: async () => {}, getQuestion: async () => null } as unknown as KunsuIpc

const ITEMS: BatchItem[] = [
  { id: 'i1', title: '陳同學', subtitle: '第 5 章打不開', context_md: '老師好，**第 5 章**影片一直轉圈圈。', proposal: '陳同學你好，已經修好了。', editable: true, default: 'approve', content_hash: 'sha256:1' },
  { id: 'i2', title: '某軟體廠商', context_md: '升級企業方案享 8 折。', proposal: '（不回）', editable: true, default: 'skip', default_reason: '促銷信', content_hash: 'sha256:2' },
  { id: 'i3', title: '張同學退款', subtitle: 'Sample Pro · NT$4,800', context_md: '購買 3 天、進度 8%。', proposal: '建議全額退款 NT$4,800', editable: false, default: 'approve', content_hash: 'sha256:3' },
]

const question = (extra: Partial<Question> = {}): Question =>
  ({
    id: 'mail-1009',
    title: '今天 3 件事',
    question_line: '今天 3 件事的回覆',
    body_md: '',
    kind: 'batch',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [],
    allow_free_text: true,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'continue' },
    source: { agent: 'claude', cwd: '/w' },
    state: 'pending',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-09',
    last_seen: '2026-10-09T01:00:00Z',
    created_at: '2026-10-09T01:00:00Z',
    next_transition_at: '2026-10-09T01:12:00Z',
    items: ITEMS,
    ...extra,
  }) as Question

const render = (q: Question) =>
  renderToStaticMarkup(<BatchReview ipc={ipc} question={q} why={null} evidence={null} isLast active={false} onUpdated={() => {}} onNext={() => {}} />)

const count = (html: string, needle: string) => html.split(needle).length - 1

describe('題組面板', () => {
  it('項目清單是 list，每項一張卡；決定鈕是一組有狀態的按鈕', () => {
    const html = render(question())
    expect(count(html, 'role="list"')).toBe(1)
    expect(count(html, 'role="listitem"')).toBe(3)
    expect(count(html, 'role="group"')).toBe(3)
    expect(html).toContain('aria-label="第 1 項怎麼處理"')
    // 第 2 項 AI 建議不處理：預設按下「不處理」，狀態用文字
    expect(count(html, 'aria-pressed="true"')).toBe(3)
    expect(html).toMatch(/data-action="skip"[^]*不處理/)
    expect(html).toContain('AI 建議不處理：促銷信')
  })

  it('草稿欄裡的 Esc 只離開輸入框：輸入區標成 data-escape="local"，面板不會因此稍後再問', () => {
    const html = render(question())
    expect(count(html, 'data-escape="local"')).toBe(2)
    expect(html).toMatch(/data-escape="local"[^>]*>\s*<textarea/)
  })

  it('只核可的項目沒有輸入框，只有建議與「照建議」', () => {
    const html = render(question())
    expect(count(html, '<textarea')).toBe(2)
    expect(html).toContain('建議全額退款 NT$4,800')
    expect(html).toContain('照建議')
  })

  it('左欄顯示選中項目（預設第 1 項）的原文，Markdown 經過 sanitize 渲染', () => {
    const html = render(question())
    expect(html).toContain('原文 · 第 1 項（共 3 項）')
    expect(html).toContain('<strong>第 5 章</strong>')
  })

  it('總結列與兩個一鍵按鈕；送出鍵可按', () => {
    const html = render(question())
    expect(html).toContain('採用 2 項 · 不處理 1')
    expect(html).toContain('全部照 AI 建議')
    expect(html).toContain('全部退回，請 AI 重寫…')
    expect(html).toMatch(/<button[^>]*data-surface-submit="true"(?![^>]*disabled)[^>]*>送出決定/)
  })

  it('第 2 輪的題組標明第幾輪', () => {
    expect(render(question({ id: 'mail-1009-r2' }))).toContain('第 2 輪')
  })

  it('已送出：回執逐項列出決定與 AI 接下來會做什麼', () => {
    const html = render(
      question({
        state: 'answered',
        answer: {
          answer_id: 'a1',
          channel: 'ui',
          answered_at: '2026-10-09T01:05:00Z',
          items: [
            { item_id: 'i1', action: 'approve', text: '陳同學你好，修好了！' },
            { item_id: 'i2', action: 'revise', note: '還是回一下，婉拒就好' },
            { item_id: 'i3', action: 'approve' },
          ],
          delivery: { state: 'resumed', updated_at: '2026-10-09T01:06:00Z' },
        },
      }),
    )
    expect(html).toContain('每一項的決定')
    expect(html).toContain('你改過')
    expect(html).toContain('請 AI 改')
    expect(html).toContain('還是回一下，婉拒就好')
    expect(html).toContain('AI 會用你確認的文字逐項送出 2 項，一字不改')
    expect(html).toContain('請 AI 改的 1 項改好後會以「第 2 輪」再送來')
    expect(html).toContain('已寫入決策帳本')
  })
})
