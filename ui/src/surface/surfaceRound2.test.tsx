// 決策面板 QA 第 2 輪修正（2026-10-09，作者核准；BDD decision-surface 場景 7／13／16、overlay-presentation 場景 6／7）。
// 測試環境沒有 DOM：純函數與靜態渲染；焦點、真的收起動畫與寬度改變另做瀏覽器 QA。
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { BatchItem, Question } from '../types'
import { DecisionColumn } from './DecisionColumn'
import { DecisionSurface } from './DecisionSurface'
import { BatchReview } from './BatchColumn'
import { closedElsewhereLine, escapeAction, holdsReceipt, openOutside, runLater } from './decision'
import { mergeQuestionSession } from '../components/overlaySession'
import batchCss from './batch.css?raw'

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

const answered = (channel: NonNullable<Question['answer']>['channel'], partial: Partial<Question> = {}) =>
  q({ state: 'answered', answer: { choice_id: 'a', channel, answered_at: '2026-10-09T00:05:00Z' } as Question['answer'], ...partial })

describe('蓋板 session（PRD §5.8、AC-OP-07）', () => {
  it('session 開著時才變成強制、非 internal 的題接到尾端；沒強制或 internal 的不插進來', () => {
    const current = [q({ id: 'q1', state: 'forced' }), q({ id: 'q2' })]
    const latest = [
      q({ id: 'q1', state: 'forced' }),
      q({ id: 'q2' }),
      q({ id: 'q3', state: 'forced' }),
      q({ id: 'q4', state: 'warned' }),
      q({ id: 'q5', state: 'forced', internal: true }),
    ]
    expect(mergeQuestionSession(current, latest).map((item) => item.id)).toEqual(['q1', 'q2', 'q3'])
  })

  it('被其他通道結案的題以最新狀態更新（終態），不會留在可作答的舊狀態', () => {
    const current = [q({ id: 'q1', state: 'forced' }), q({ id: 'q2', state: 'forced' })]
    const merged = mergeQuestionSession(current, [answered('discord', { id: 'q1' }), q({ id: 'q2', state: 'forced' })])
    expect(merged.map((item) => [item.id, item.state])).toEqual([
      ['q1', 'answered'],
      ['q2', 'forced'],
    ])
  })

  it('全新的 session 不帶進已結案的題', () => {
    expect(mergeQuestionSession([], [answered('ui', { id: 'q1' }), q({ id: 'q2' })]).map((item) => item.id)).toEqual(['q2'])
  })

  it('「保留回執」只在面板正顯示這次在面板裡送出的回執時成立', () => {
    const here = new Set(['q1'])
    expect(holdsReceipt(answered('ui', { id: 'q1' }), here)).toBe(true)
    expect(holdsReceipt(answered('discord', { id: 'q2' }), here)).toBe(false)
    expect(holdsReceipt(q({ id: 'q1' }), here)).toBe(false)
    expect(holdsReceipt(undefined, here)).toBe(false)
  })
})

describe('稍後再問（AC-OP-06）', () => {
  it('dismissal 失敗：不收起、不通知原生，回傳看得懂的錯誤（可以再試）', async () => {
    const calls: string[] = []
    const result = await runLater({
      ids: ['q1', 'q2'],
      dismiss: async (id) => {
        calls.push(`dismiss ${id}`)
        if (id === 'q2') throw new Error('資料庫鎖住了')
      },
      ack: () => calls.push('ack'),
      close: async () => {
        calls.push('close')
      },
    })
    expect(result).toEqual({ ok: false, message: '稍後再問沒有成功：資料庫鎖住了。再按一次 Esc 或「稍後再問」重試。' })
    expect(calls).not.toContain('ack')
    expect(calls).not.toContain('close')
  })

  it('全部 dismissal 成功才通知原生，然後收起（等收起完成才回傳）', async () => {
    const calls: string[] = []
    const result = await runLater({
      ids: ['q1', 'q2'],
      dismiss: async (id) => void calls.push(`dismiss ${id}`),
      ack: () => calls.push('ack'),
      close: () =>
        new Promise<void>((resolve) =>
          setTimeout(() => {
            calls.push('closed')
            resolve()
          }, 0),
        ),
    })
    expect(result).toEqual({ ok: true })
    expect(calls).toEqual(['dismiss q1', 'dismiss q2', 'ack', 'closed'])
  })

  it('焦點在行內區塊的按鈕上時 Esc＝稍後再問；只有焦點真的在輸入框裡才只收起編輯', () => {
    expect(escapeAction({ stage: 'open', composing: false, local: true })).toEqual({ later: true, ack: true })
    expect(escapeAction({ stage: 'open', composing: false, local: true, field: false })).toEqual({ later: true, ack: true })
    expect(escapeAction({ stage: 'open', composing: false, local: true, field: true })).toEqual({ later: false, ack: true })
    // 不在行內區塊的輸入框（例如補充說明）：照舊是稍後再問。
    expect(escapeAction({ stage: 'open', composing: false, local: false, field: true })).toEqual({ later: true, ack: true })
  })
})

describe('往外開（AC-DS-13）', () => {
  it('讓開失敗（稍後再問被拒）就不打開目標', async () => {
    const open = vi.fn()
    openOutside({ inOverlay: true, stepAside: async () => false, open })('https://example.com')
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(open).not.toHaveBeenCalled()
  })

  it('主視窗不讓開，直接打開', () => {
    const open = vi.fn()
    const stepAside = vi.fn(async () => true)
    openOutside({ inOverlay: false, stepAside, open })('https://example.com')
    expect(stepAside).not.toHaveBeenCalled()
    expect(open).toHaveBeenCalledWith('https://example.com')
  })
})

describe('被其他通道結案的題就地唯讀（AC-DS-16）', () => {
  it('說明是怎麼結案的', () => {
    expect(closedElsewhereLine(answered('discord'))).toBe('已由 Discord 回答')
    expect(closedElsewhereLine(answered('cli'))).toBe('已由終端機（CLI）回答')
    expect(closedElsewhereLine(answered('mobile'))).toBe('已在手機上回答')
    expect(closedElsewhereLine(answered('kongko'))).toBe('已由 Kongko 發佈確認結案')
    expect(closedElsewhereLine(answered('ui'))).toBe('已在另一個 Kunsu 視窗回答')
    expect(closedElsewhereLine(answered('auto_policy'))).toBe('已依預先授權的規則自動處理')
    expect(closedElsewhereLine(q({ state: 'cancelled' }))).toBe('出題的 AI 已取消這題')
    expect(closedElsewhereLine(q({ state: 'superseded' }))).toBe('出題的 AI 已改出新版本')
    expect(closedElsewhereLine(q({ state: 'expired' }))).toBe('已過期')
  })

  it('一般題：唯讀畫面，沒有選項、輸入框與送出，只有完成', () => {
    const html = renderToStaticMarkup(
      <DecisionColumn ipc={ipc} question={answered('discord')} closedElsewhere withWhy={false} isLast active={false} onUpdated={() => {}} onNext={() => {}} />,
    )
    expect(html).toContain('已在其他地方結案')
    expect(html).toContain('已由 Discord 回答')
    expect(html).not.toContain('role="radio"')
    expect(html).not.toContain('<textarea')
    expect(html).not.toContain('這題不該問我')
    expect(html).toMatch(/data-surface-submit="true"[^>]*>完成/)
  })

  it('題組：唯讀畫面，沒有項目卡與送出決定', () => {
    const items: BatchItem[] = [
      { id: 'i1', title: '陳同學', context_md: '老師好', proposal: '你好', editable: true, default: 'approve', content_hash: 'sha256:1' },
      { id: 'i2', title: '王先生', context_md: '發票', proposal: '收到', editable: true, default: 'approve', content_hash: 'sha256:2' },
    ]
    const batch = q({ id: 'mail-1', kind: 'batch', options: [], items, state: 'cancelled' })
    const html = renderToStaticMarkup(
      <BatchReview ipc={ipc} question={batch} closedElsewhere why={null} evidence={null} isLast={false} active={false} onUpdated={() => {}} onNext={() => {}} />,
    )
    expect(html).toContain('已在其他地方結案')
    expect(html).toContain('出題的 AI 已取消這題')
    expect(html).not.toContain('<textarea')
    expect(html).not.toContain('送出決定')
    expect(html).toMatch(/data-surface-submit="true"[^>]*>下一題/)
  })
})

describe('切題淡入（AC-DS-10）', () => {
  afterEach(() => vi.unstubAllGlobals())
  it('第一次張開不帶切題淡入（張開有自己的依序淡入；切題才帶 data-enter）', () => {
    vi.stubGlobal('window', { innerWidth: 1440, innerHeight: 900, matchMedia: () => ({ matches: false }) })
    vi.stubGlobal('document', { documentElement: { dataset: {} }, body: { style: {} } })
    vi.stubGlobal('localStorage', { getItem: () => null })
    const html = renderToStaticMarkup(<DecisionSurface ipc={ipc} questions={[q()]} onExit={() => {}} />)
    expect(html).toMatch(/class="surface-body"/)
    expect(html).not.toContain('data-enter="true"')
  })
})

describe('題組卡片標題（AC-BR-02）', () => {
  it('標題換行顯示完整內容，不用省略號截掉', () => {
    const flat = batchCss.replace(/\/\*[\s\S]*?\*\//g, '')
    const bodies = [...flat.matchAll(/([^{}]+)\{([^{}]*)\}/g)]
      .filter(([, selector]) => selector.split(',').some((part) => part.trim() === '.batch-card__title'))
      .map(([, , body]) => body)
      .join(';')
    expect(bodies).not.toMatch(/text-overflow:\s*ellipsis/)
    expect(bodies).not.toMatch(/white-space:\s*nowrap/)
    expect(bodies).toMatch(/overflow-wrap:\s*anywhere/)
  })
})
