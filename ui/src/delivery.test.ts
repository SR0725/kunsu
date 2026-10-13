// 回答延續原對話的回執（BDD conversation-continuity，AC-CC-13）。
import { describe, expect, it, vi } from 'vitest'
import { conversationLink, handoffFromDelivery, isConversationLink } from './delivery'
import type { AgentReport, Delivery, Question } from './types'

function q(partial: Partial<Question> & { answer?: Question['answer'] }): Question {
  return {
    id: 'q1',
    title: 't',
    question_line: '要重啟嗎？',
    body_md: '',
    kind: 'choice',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [],
    allow_free_text: true,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'continue' },
    source: { agent: 'claude', cwd: '/w', host: 'claude_desktop', conversation_id: 'c1', host_session_id: 'local_abc-1' },
    state: 'answered',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-08',
    last_seen: '2026-10-08T01:00:00Z',
    created_at: '2026-10-08T01:00:00Z',
    next_transition_at: '2026-10-08T01:00:00Z',
    ...partial,
  } as Question
}

const answered = (delivery?: Partial<Delivery>, report?: AgentReport) => ({
  answer_id: 'a1',
  channel: 'ui' as const,
  answered_at: '2026-10-08T01:01:00Z',
  ...(delivery ? { delivery: delivery as Delivery } : {}),
  ...(report ? { report } : {}),
})

describe('handoffFromDelivery', () => {
  it.each([
    ['live_waiter', '已交給等待中的原對話', true],
    ['resumed', '已在原對話追加處理', true],
    ['picked_up', '原對話已自行取走答案', true],
    ['held', '原對話正開著，等它空下來就接續', false],
    ['pending', '等待中的原對話將取走答案', false],
  ])('%s → %s', (state, text, done) => {
    const h = handoffFromDelivery(q({ answer: answered({ state: state as Delivery['state'], updated_at: 'x' }) }))
    expect(h?.text).toBe(text)
    expect(h?.done).toBe(done)
    expect(h?.tone).not.toBe('danger')
  })

  it('fresh shows the reason; dead is a visible failure', () => {
    const fresh = handoffFromDelivery(q({ answer: answered({ state: 'fresh', reason: '原對話不存在', updated_at: 'x' }) }))
    expect(fresh?.text).toBe('已開新對話處理（原對話不存在）')
    const dead = handoffFromDelivery(q({ answer: answered({ state: 'dead', reason: 'boom', updated_at: 'x' }) }))
    expect(dead?.tone).toBe('danger')
  })

  it('no delivery record → caller keeps the legacy handoff text', () => {
    expect(handoffFromDelivery(q({ answer: answered() }))).toBeNull()
  })
})

describe('conversationLink', () => {
  it('prefers the server link, falls back to the source identity, rejects anything else', () => {
    expect(conversationLink(q({ answer: answered({ state: 'resumed', conversation_link: 'claude://code/continue?session=local_zz', updated_at: 'x' }) }))).toBe(
      'claude://code/continue?session=local_zz',
    )
    expect(conversationLink(q({ answer: answered() }))).toBe('claude://code/continue?session=local_abc-1')
    expect(
      conversationLink(q({ source: { agent: 'codex', cwd: '/w', host: 'codex_app', conversation_id: '01a1_t' }, answer: answered() })),
    ).toBe('codex://threads/01a1_t')
    expect(conversationLink(q({ source: { agent: 'claude', cwd: '/w', host: 'claude_cli', conversation_id: 'c' }, answer: answered() }))).toBeNull()
    expect(
      conversationLink(q({ answer: answered({ state: 'resumed', conversation_link: 'file:///etc/passwd', updated_at: 'x' }) })),
    ).toBe('claude://code/continue?session=local_abc-1')
  })

  it('isConversationLink mirrors the shell allowlist', () => {
    expect(isConversationLink('codex://threads/a-b_c')).toBe(true)
    expect(isConversationLink('codex://threads/../x')).toBe(false)
    expect(isConversationLink('claude://code/continue?session=local_a&b=1')).toBe(false)
  })
})

// QA 第二輪 G：追加中（resuming）要看得到，輪詢不能把它當成已定案，也不能兩分鐘就停。
describe('resuming 與輪詢', () => {
  it('追加中顯示進行中並持續輪詢', async () => {
    const { deliverySettled, receiptPollDelay } = await import('./delivery')
    const resuming = q({ answer: answered({ state: 'resuming' }) })
    expect(handoffFromDelivery(resuming)).toEqual({ text: '正在原對話追加處理…', done: false, pulse: true, tone: 'muted' })
    expect(deliverySettled(resuming)).toBe(false)
    expect(deliverySettled(q({ answer: answered({ state: 'held' }) }))).toBe(false)
    expect(deliverySettled(q({ answer: answered({ state: 'pending' }) }))).toBe(false)
    // QA 第三輪 #11：已交出去但 AI 還沒回報 → 回執繼續等 AI 回饋。
    expect(deliverySettled(q({ answer: answered({ state: 'resumed' }) }))).toBe(false)
    expect(deliverySettled(q({ answer: answered({ state: 'fresh' }) }))).toBe(false)
    expect(deliverySettled(q({ answer: answered({ state: 'live_waiter' }) }))).toBe(false)
    expect(deliverySettled(q({ answer: answered({ state: 'no_continuation' }) }))).toBe(true)
    expect(deliverySettled(q({ answer: answered({ state: 'dead' }) }))).toBe(true)
    expect(deliverySettled(q({ answer: answered({ state: 'held' }, { status: 'done', summary_md: 'ok', reported_at: 'x' }) }))).toBe(true)
    expect(receiptPollDelay(0)).toBe(2000)
    expect(receiptPollDelay(3 * 60_000)).toBe(10_000)
    expect(receiptPollDelay(31 * 60_000)).toBeNull()
  })
})

// QA 第四輪 #7（AC-CC-13）：起算時間固定在開始輪詢，新資料不重設；前 2 分鐘每 2 秒、之後每 10 秒、30 分鐘停。
describe('startReceiptPolling', () => {
  it('依開始輪詢的時間退避並在 30 分鐘停止', async () => {
    vi.useFakeTimers()
    try {
      vi.setSystemTime(Date.parse('2026-10-08T00:00:00Z'))
      const { startReceiptPolling } = await import('./delivery')
      const start = Date.now()
      const at: number[] = []
      let updates = 0
      const pending = q({ answer: answered({ state: 'resumed' }) })
      startReceiptPolling({
        fetch: async () => {
          at.push(Date.now() - start)
          return { ...pending }
        },
        onUpdate: () => {
          updates += 1
        },
      })
      await vi.advanceTimersByTimeAsync(60 * 60_000)
      expect(at[0]).toBe(1200)
      expect(at[1] - at[0]).toBe(2000)
      const late = at.filter((t) => t > 2 * 60_000 + 2000)
      expect(late.length).toBeGreaterThan(100)
      expect(late.slice(1).every((t, i) => t - late[i] === 10_000)).toBe(true)
      expect(at[at.length - 1]).toBeLessThanOrEqual(30 * 60_000 + 10_000)
      expect(updates).toBe(at.length)
    } finally {
      vi.useRealTimers()
    }
  })

  it('交付定案就停；stop 之後不再輪詢', async () => {
    vi.useFakeTimers()
    try {
      const { startReceiptPolling } = await import('./delivery')
      let n = 0
      const settled = q({ answer: answered({ state: 'resumed' }, { status: 'done', summary_md: 'ok', reported_at: 'x' }) })
      startReceiptPolling({ fetch: async () => (n++, settled), onUpdate: () => {} })
      await vi.advanceTimersByTimeAsync(10 * 60_000)
      expect(n).toBe(1)
      let m = 0
      const stop = startReceiptPolling({ fetch: async () => (m++, q({ answer: answered({ state: 'held' }) })), onUpdate: () => {} })
      await vi.advanceTimersByTimeAsync(5000)
      stop()
      const seen = m
      await vi.advanceTimersByTimeAsync(10 * 60_000)
      expect(m).toBe(seen)
    } finally {
      vi.useRealTimers()
    }
  })
})

// QA 第五輪 N9（AC-CC-13）：讀取失敗（app 忙、IPC 逾時）照退避重試，不會停止、不留未處理的錯誤。
describe('startReceiptPolling 遇錯', () => {
  it('一次失敗後繼續輪詢', async () => {
    vi.useFakeTimers()
    try {
      const { startReceiptPolling } = await import('./delivery')
      let n = 0
      const held = q({ answer: answered({ state: 'held' }) })
      startReceiptPolling({
        fetch: async () => {
          n += 1
          if (n === 2) throw new Error('ipc timeout')
          return held
        },
        onUpdate: () => {},
      })
      await vi.advanceTimersByTimeAsync(60_000)
      expect(n).toBeGreaterThan(10)
    } finally {
      vi.useRealTimers()
    }
  })
})

// QA 第五輪 N11（AC-CC-13／14）：錢題回執顯示錢的結果一行；交付失敗另由交付狀態說明。
describe('moneyOutcome', () => {
  it('依錢鏈結果開頭判斷', async () => {
    const { moneyOutcome } = await import('./delivery')
    const money = (execution_result?: string) =>
      q({ money: { is_money_op: true }, answer: { ...answered({ state: 'dead' }), execution_result } } as Partial<Question>)
    expect(moneyOutcome(money('COMMIT_OK: ok；交付失敗：死信'))).toMatchObject({ text: '錢：已執行完成', tone: 'ok' })
    expect(moneyOutcome(money('EXEC_UNKNOWN: executor 已用掉授權但 commit 沒有回報結果'))).toMatchObject({
      text: '錢：結果不明，可能已經動過，請人工對帳',
      tone: 'muted',
    })
    expect(moneyOutcome(money('APPROVAL_SIGNED: x；DRY_RUN commit: y'))).toMatchObject({ text: '錢：乾跑，未實際執行', tone: 'muted' })
    expect(moneyOutcome(money('DRY_RUN(internal): prepare 未執行'))).toMatchObject({ text: '錢：乾跑，未實際執行', tone: 'muted' })
    expect(moneyOutcome(money('EXEC_BLOCKED: 護欄'))).toBeNull()
    expect(moneyOutcome(money('已在原對話追加處理'))).toBeNull()
    expect(moneyOutcome(money(undefined))).toBeNull()
    expect(moneyOutcome(q({ answer: { ...answered({ state: 'resumed' }), execution_result: 'COMMIT_OK' } }))).toBeNull()
  })
})

it('queued Codex delivery remains waiting until the agent reports', () => {
  const question = q({answer: answered({state:'resumed',reason:'已排入原對話，等待 AI 接手'})})
  expect(handoffFromDelivery(question)).toEqual({text:'已排入原對話，等待 AI 接手',done:false,pulse:false,tone:'muted'})
})
