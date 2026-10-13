// 決策面板 QA 第 3 輪修正（2026-10-09，作者核准；BDD decision-surface 場景 9／10／13／16／17、overlay-presentation 場景 6／7、
// batch-review 場景 9）。測試環境沒有 DOM：純函數、事件順序模擬與靜態渲染；真的時序（effect、焦點）另做瀏覽器 QA。
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { BatchItem, Question } from '../types'
import { DecisionColumn } from './DecisionColumn'
import { BatchReview } from './BatchColumn'
import {
  closedElsewhereLine,
  closedStatus,
  dotLabel,
  forgetDueAgain,
  isTerminalError,
  laterTargets,
  reviewPhase,
  runLater,
} from './decision'
import { OVERLAY_CLOSE_GRACE_MS, overlayShellAction, overlayShellMounted } from '../overlayShell'
import type { OverlayShellAction } from '../overlayShell'
import { flowFeed } from '../components/overlaySession'
import indexCss from '../index.css?raw'

const ipc = new Proxy({} as Record<string, unknown>, {
  get: (_t, key: string) => (key.startsWith('subscribe') ? () => () => {} : async () => null),
}) as unknown as KunsuIpc

function q(partial: Partial<Question> = {}): Question {
  return {
    id: 'q1',
    title: '素材 B 要不要加碼',
    question_line: '要不要加碼 B？',
    body_md: '',
    kind: 'choice',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [
      { id: 'a', label: '加碼 20%', recommended: true },
      { id: 'b', label: '維持' },
    ],
    allow_free_text: true,
    default_action: { on_expire: 'noop_log' },
    wake_on_answer: { mode: 'continue' },
    source: { agent: 'claude', cwd: '/w' },
    state: 'pending',
    snooze_count: 0,
    forced_today_used: false,
    first_proposed: '2026-10-09',
    last_seen: '2026-10-09T00:00:00Z',
    created_at: '2026-10-09T00:00:00Z',
    next_transition_at: '2026-10-09T00:00:00Z',
    ...partial,
  } as Question
}

const answer = (channel: NonNullable<Question['answer']>['channel'], extra: Partial<NonNullable<Question['answer']>> = {}) =>
  ({ choice_id: 'a', channel, answered_at: '2026-10-09T00:05:00Z', ...extra }) as Question['answer']

function browser() {
  vi.stubGlobal('window', { innerWidth: 1440, innerHeight: 900, matchMedia: () => ({ matches: false }) })
  vi.stubGlobal('document', { documentElement: { dataset: {} }, body: { style: {} } })
  vi.stubGlobal('localStorage', { getItem: () => null })
}

const text = (html: string) => html.replace(/<[^>]*>/g, '')
const flatCss = indexCss.replace(/\/\*[\s\S]*?\*\//g, '')
/** index.css 裡選擇器（逗號分隔的其中一個）恰好是 `selector` 的規則內容，全部串起來。 */
const rule = (selector: string) =>
  [...flatCss.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
    .filter(([, sel]) => sel.split(',').some((part) => part.trim() === selector))
    .map(([, , body]) => body)
    .join(';')

function column(question: Question) {
  return renderToStaticMarkup(
    <DecisionColumn ipc={ipc} question={question} withWhy={false} isLast active={false} onUpdated={() => {}} onNext={() => {}} />,
  )
}

afterEach(() => {
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

// =====================================================================================================

describe('AC-DS-09／AC-OP-07(c) 蓋板的收起判斷不早於這次作答的結果', () => {
  const base = { loaded: true, hasTransition: true }

  it('作答進行中（送出到結果回來）一律不收起，即使清單已經空了', () => {
    expect(overlayShellAction({ ...base, questionCount: 0, holdingReceipt: false, answering: true })).toBe('render')
    expect(overlayShellAction({ ...base, questionCount: 0, holdingReceipt: false, answering: false })).toBe('close')
  })

  it('收起緩衝期間面板照樣掛著；只有等待（沒有轉場、清單還沒載入）時不掛', () => {
    expect(overlayShellMounted('close')).toBe(true)
    expect(overlayShellMounted('render')).toBe(true)
    expect(overlayShellMounted('wait')).toBe(false)
    expect(OVERLAY_CLOSE_GRACE_MS).toBe(300)
  })

  /** 照 OverlayShell 的 effect 模擬：判斷是 close 時排一次收起計時，判斷一改變就取消。 */
  function shell() {
    const state = { ...base, questionCount: 1, holdingReceipt: false, answering: false }
    let timer: ReturnType<typeof setTimeout> | undefined
    const seen: OverlayShellAction[] = []
    const result = { exited: false, unmounted: false }
    const apply = (patch: Partial<typeof state>) => {
      Object.assign(state, patch)
      const action = overlayShellAction(state)
      seen.push(action)
      if (!overlayShellMounted(action)) result.unmounted = true
      if (action === 'close') {
        timer ??= setTimeout(() => {
          result.exited = true
        }, OVERLAY_CLOSE_GRACE_MS)
      } else if (timer !== undefined) {
        clearTimeout(timer)
        timer = undefined
      }
    }
    return { apply, seen, result }
  }

  it('瀏覽器重現的順序：送出 → 清單先變空 → 結果回來 → 面板回報回執：面板從沒被卸載，也不收起', () => {
    vi.useFakeTimers()
    const s = shell()
    s.apply({ answering: true }) // 決策欄在呼叫 answer_question 前同步回報
    s.apply({ questionCount: 0 }) // server 先發 questions_changed
    vi.advanceTimersByTime(50)
    s.apply({ answering: false }) // 作答結果回來（同一批更新裡 session 已寫進答案）
    vi.advanceTimersByTime(16)
    s.apply({ holdingReceipt: true }) // 面板的 effect 回報正在顯示這次送出的回執
    vi.advanceTimersByTime(5_000)
    expect(s.result.unmounted, `判斷序列：${s.seen.join(' → ')}`).toBe(false)
    expect(s.result.exited, `判斷序列：${s.seen.join(' → ')}`).toBe(false)
  })

  it('作答結果比清單變更晚 2 秒才回來（真實 server 交付較慢）也不會收起', () => {
    vi.useFakeTimers()
    const s = shell()
    s.apply({ answering: true })
    s.apply({ questionCount: 0 })
    vi.advanceTimersByTime(2_000)
    s.apply({ answering: false, holdingReceipt: true })
    vi.advanceTimersByTime(5_000)
    expect(s.result.exited).toBe(false)
  })

  it('回歸：題目全由其他通道結案、沒有回執要保留時，300 ms 後收起', () => {
    vi.useFakeTimers()
    const s = shell()
    s.apply({ questionCount: 0 })
    vi.advanceTimersByTime(OVERLAY_CLOSE_GRACE_MS - 1)
    expect(s.result.exited).toBe(false)
    vi.advanceTimersByTime(1)
    expect(s.result.exited).toBe(true)
  })
})

describe('AC-DS-16／AC-BR-09 主視窗的作答流程跟著最新清單', () => {
  const session = [q({ id: 'a' }), q({ id: 'b' })]
  const latest = [
    q({ id: 'a', state: 'answered', answer: answer('discord') }),
    q({ id: 'b' }),
    q({ id: 'c', state: 'forced' }),
    q({ id: 'old', state: 'expired' }),
  ]

  it('一般 flow 用完整清單（同蓋板：就地更新、接上新強制題）', () => {
    expect(flowFeed(latest, session, false)).toBe(latest)
  })

  it('回看已結案的題只跟著那幾題的最新資料，不把強制題接進來', () => {
    const review = [q({ id: 'old', state: 'expired' })]
    expect(flowFeed(latest, review, true).map((item) => item.id)).toEqual(['old'])
  })
})

describe('AC-DS-13 選項裡的連結不包在選項按鈕裡', () => {
  it('選項卡片：radio 按鈕裡沒有連結；標題與說明在按鈕外、以 aria-labelledby／aria-describedby 連回去', () => {
    browser()
    const html = column(
      q({
        state: 'forced',
        options: [
          { id: 'a', label: '照 [提案](https://docs.example.com/p) 執行', detail: '見 [試算表](https://sheets.example.com/s)', recommended: true },
          { id: 'b', label: '維持' },
        ],
      }),
    )
    const buttons = [...html.matchAll(/<button\b[^>]*>([\s\S]*?)<\/button>/g)]
    for (const [, inner] of buttons) expect(inner).not.toMatch(/<a\b/)
    const radio = html.match(/<button\b[^>]*role="radio"[^>]*>/)?.[0] ?? ''
    const labelledby = radio.match(/aria-labelledby="([^"]+)"/)?.[1]
    const describedby = radio.match(/aria-describedby="([^"]+)"/)?.[1]
    expect(labelledby, radio).toBeTruthy()
    expect(describedby, radio).toBeTruthy()
    expect(html).toContain(`id="${labelledby}"`)
    expect(html).toContain(`id="${describedby}"`)
    expect(html).toMatch(/<a\b[^>]*href="https:\/\/docs\.example\.com\/p"/)
    expect(html).toMatch(/<a\b[^>]*href="https:\/\/sheets\.example\.com\/s"/)
  })

  it('選取狀態仍同步在 radio 的 aria-checked，卡片以 data-checked 呈現白框', () => {
    browser()
    const html = column(q({ state: 'forced' }))
    expect(html.match(/role="radio"/g)).toHaveLength(2)
    expect(html).toMatch(/role="radio"[^>]*aria-checked="false"|aria-checked="false"[^>]*role="radio"/)
    expect(html).toContain('data-checked="false"')
    expect(rule('.surface-opt[data-checked="true"]')).toMatch(/border-color/)
    // radio 按鈕鋪滿卡片；疊在上面的標題與說明讓點擊穿過去，只有其中的連結接收點擊。
    expect(rule('.surface-opt__hit')).toMatch(/position:\s*absolute/)
    expect(rule('.surface-opt__hit')).toMatch(/inset:\s*0/)
    expect(rule('.surface-opt__label')).toMatch(/pointer-events:\s*none/)
    expect(rule('.surface-opt__detail')).toMatch(/pointer-events:\s*none/)
    expect(rule('.surface-opt__label a')).toMatch(/pointer-events:\s*auto/)
    expect(rule('.surface-opt__detail a')).toMatch(/pointer-events:\s*auto/)
  })
})

describe('AC-DS-17 回看已結案的題：唯讀、狀態依實際終態', () => {
  it('reviewPhase：已回答與使用者推掉的是回執；過期、被取代、AI 取消的是唯讀狀態；未結案照常作答', () => {
    expect(reviewPhase(q())).toBe('deciding')
    expect(reviewPhase(q({ state: 'forced' }))).toBe('deciding')
    expect(reviewPhase(q({ state: 'answered', answer: answer('ui') }))).toBe('receipt')
    expect(reviewPhase(q({ state: 'routed', answer: answer('kongko') }))).toBe('receipt')
    expect(reviewPhase(q({ state: 'answered', answer: answer('ui'), execution_blocked: 'prepare 失敗' }))).toBe('failed')
    expect(reviewPhase(q({ state: 'cancelled', rejected: { at: '2026-10-09T00:03:00Z', reason: '不該問我' } }))).toBe('receipt')
    expect(reviewPhase(q({ state: 'cancelled' }))).toBe('closed')
    expect(reviewPhase(q({ state: 'expired' }))).toBe('closed')
    expect(reviewPhase(q({ state: 'superseded' }))).toBe('closed')
  })

  it('closedStatus：已回答／已過期（沒有回答、逾時依預設）／AI 取消了這題／已退回重擬（你退回了這題：原因）／被新題取代', () => {
    expect(closedStatus(q({ state: 'answered', answer: answer('discord') }))).toEqual({ label: '已回答', line: '已由 Discord 回答' })
    expect(closedStatus(q({ state: 'expired' }))).toEqual({ label: '已過期', line: '沒有回答' })
    expect(
      closedStatus(
        q({ state: 'expired', default_action: { on_expire: 'pick_option', option_id: 'b' }, answer: answer('auto_policy', { choice_id: 'b' }) }),
      ),
    ).toEqual({ label: '已過期', line: '沒有回答（逾時依預設：維持）' })
    expect(closedStatus(q({ state: 'cancelled' }))).toEqual({ label: 'AI 取消了這題', line: '出題的 AI 已取消這題' })
    expect(closedStatus(q({ state: 'cancelled', rejected: { at: '2026-10-09T00:03:00Z', reason: '不該問我' } }))).toEqual({
      label: '已退回重擬',
      line: '你退回了這題：不該問我',
    })
    expect(closedStatus(q({ state: 'cancelled', rejected: { at: '2026-10-09T00:03:00Z' } }))).toEqual({ label: '已退回重擬', line: '你退回了這題' })
    expect(closedStatus(q({ state: 'superseded' }))).toEqual({ label: '被新題取代', line: '出題的 AI 已改出新版本' })
  })

  it('被其他通道結案的說明分得出使用者在別的視窗推掉與 AI 取消', () => {
    expect(closedElsewhereLine(q({ state: 'cancelled', rejected: { at: '2026-10-09T00:03:00Z' } }))).toBe('你在另一個 Kunsu 視窗退回了這題')
    expect(closedElsewhereLine(q({ state: 'cancelled' }))).toBe('出題的 AI 已取消這題')
  })

  it('一般題：過期（逾時依預設）、被取代的回看畫面是唯讀狀態，沒有選項、回答框與送出', () => {
    browser()
    for (const question of [
      q({ state: 'expired', default_action: { on_expire: 'pick_option', option_id: 'b' }, answer: answer('auto_policy', { choice_id: 'b' }) }),
      q({ state: 'superseded' }),
    ]) {
      const html = column(question)
      expect(html).not.toContain('role="radio"')
      expect(html).not.toContain('<textarea')
      expect(html).not.toContain('這題不該問我')
      expect(text(html)).not.toMatch(/要你決定|要你回應|送出決定|送出回答|已在其他地方結案/)
      expect(html).toMatch(/data-surface-submit="true"[^>]*>完成/)
    }
    expect(text(column(q({ state: 'superseded' })))).toContain('被新題取代')
    expect(text(column(q({ state: 'expired', default_action: { on_expire: 'pick_option', option_id: 'b' }, answer: answer('auto_policy', { choice_id: 'b' }) })))).toContain(
      '沒有回答（逾時依預設：維持）',
    )
  })

  it('使用者推掉的題回看時是回執，寫明你退回了這題與原因', () => {
    browser()
    const html = text(column(q({ state: 'cancelled', rejected: { at: '2026-10-09T00:03:00Z', reason: '不該問我' } })))
    expect(html).toContain('已退回重擬')
    expect(html).toContain('你退回了這題：不該問我')
  })

  const items: BatchItem[] = [
    { id: 'i1', title: '陳同學', context_md: '老師好', proposal: '你好', editable: true, default: 'approve', content_hash: 'sha256:1' },
    { id: 'i2', title: '王先生', context_md: '發票', proposal: '收到', editable: true, default: 'approve', content_hash: 'sha256:2' },
  ]
  const batch = (partial: Partial<Question>) => q({ id: 'mail-1', kind: 'batch', options: [], items, ...partial })
  const review = (question: Question) =>
    renderToStaticMarkup(<BatchReview ipc={ipc} question={question} why={null} evidence={null} isLast active={false} onUpdated={() => {}} onNext={() => {}} />)

  it('題組：過期的回看畫面唯讀（沒有草稿框與送出決定），AI 取消的不寫成退回重擬', () => {
    browser()
    const expired = review(batch({ state: 'expired' }))
    expect(expired).not.toContain('<textarea')
    expect(text(expired)).not.toMatch(/送出決定|要你決定|已在其他地方結案/)
    expect(text(expired)).toContain('已過期')
    const cancelled = text(review(batch({ state: 'cancelled' })))
    expect(cancelled).not.toContain('退回重擬')
    expect(cancelled).toContain('AI 取消了這題')
  })

  it('題組：使用者推掉的回看時是回執，寫已退回重擬與原因', () => {
    browser()
    const html = text(review(batch({ state: 'cancelled', rejected: { at: '2026-10-09T00:03:00Z', reason: '這批不用回' } })))
    expect(html).toContain('已退回重擬')
    expect(html).toContain('你退回了這題：這批不用回')
  })
})

describe('AC-DS-10／AC-DS-16 進度點的無障礙名稱依實際終態', () => {
  it('未結案、在面板結案、被其他通道結案各有各的寫法', () => {
    expect(dotLabel(0, q(), false)).toBe('第 1 題')
    expect(dotLabel(1, q({ state: 'answered', answer: answer('ui') }), false)).toBe('第 2 題（已回答）')
    expect(dotLabel(1, q({ state: 'cancelled', rejected: { at: '2026-10-09T00:03:00Z' } }), false)).toBe('第 2 題（已退回重擬）')
    expect(dotLabel(2, q({ state: 'answered', answer: answer('discord') }), true)).toBe('第 3 題（已結案：已由 Discord 回答）')
    expect(dotLabel(2, q({ state: 'cancelled' }), true)).toBe('第 3 題（已結案：出題的 AI 已取消這題）')
    expect(dotLabel(2, q({ state: 'expired' }), true)).toBe('第 3 題（已結案：已過期）')
  })
})

describe('AC-OP-06 稍後再問重試：每題只記一次', () => {
  it('範圍：目前題目（未結案）＋session 的強制題，去重；已結案的目前題目不算', () => {
    const session = [q({ id: 'a' }), q({ id: 'b', state: 'forced' }), q({ id: 'c', state: 'forced' })]
    expect(laterTargets(session[0], session)).toEqual(['a', 'b', 'c'])
    expect(laterTargets(session[1], session)).toEqual(['b', 'c'])
    expect(laterTargets(q({ id: 'a', state: 'answered' }), session)).toEqual(['b', 'c'])
  })

  it('server 回「已在終態」視同成功：記進 done、繼續下一題、最後收起', async () => {
    const calls: string[] = []
    const done = new Map<string, number | null>()
    const events: string[] = []
    const result = await runLater({
      ids: ['a', 'b'],
      done,
      dismiss: async (id) => {
        calls.push(id)
        if (id === 'a') throw new Error('題目 a 已在終態 Answered，不接受此操作')
        return { cycle_number: 3 }
      },
      ack: () => events.push('ack'),
      close: () => void events.push('close'),
    })
    expect(result.ok).toBe(true)
    expect(calls).toEqual(['a', 'b'])
    expect([...done.entries()]).toEqual([
      ['a', null],
      ['b', 3],
    ])
    expect(events).toEqual(['ack', 'close'])
    expect(isTerminalError(new Error('題目 a 已在終態 Answered，不接受此操作'))).toBe(true)
    expect(isTerminalError('題目 a 已在終態 Expired，不接受此操作')).toBe(true)
    expect(isTerminalError(new Error('資料庫暫時鎖住了'))).toBe(false)
  })

  it('done 記住成功的週期編號：之後同一個 session 又到期（又是強制、週期編號 ≥ 記錄的）就從 done 拿掉，照樣記', () => {
    const done = new Map<string, number | null>([
      ['stale', 3],
      ['pending', 3],
      ['again', 3],
      ['closed', null],
    ])
    const session = [
      q({ id: 'stale', state: 'forced', cycle_number: 2 }), // 清單還沒更新：讀到的是記之前的資料
      q({ id: 'pending', state: 'pending', cycle_number: 3 }),
      q({ id: 'again', state: 'forced', cycle_number: 3 }), // 15 分鐘後又到期
      q({ id: 'closed', state: 'answered', cycle_number: 1 }),
    ]
    expect([...forgetDueAgain(done, session).keys()]).toEqual(['stale', 'pending', 'closed'])
    expect(done.size, '不改原本的 Map').toBe(4)
  })
})

describe('AC-OP-06 「稍後再問沒有成功」的錯誤不蓋住內容', () => {
  it('錯誤是頁首下方的流內區塊（面板 grid 的第 2 列），不是疊在內容上的絕對定位', () => {
    expect(rule('.surface-alert.surface-later-error')).not.toMatch(/position:\s*absolute/)
    expect(rule('.surface-panel')).toMatch(/grid-template-rows:\s*68px\s+auto\s+minmax\(0,\s*1fr\)/)
    expect(rule('.surface-body')).toMatch(/grid-row:\s*3/)
  })
})
