// AC-DS-04/05/06/07：送出可用性、payload、選取與鍵盤。
import { describe, expect, it } from 'vitest'
import {
  buildPayload,
  canSend,
  decisionSummary,
  escapeAction,
  focusKind,
  optionTabStop,
  primaryLabel,
  rovingTarget,
  surfaceKey,
  toggleSelect,
  wrapFocus,
} from './decision'
import type { Question } from '../types'

const opts = [
  { id: 'restart', label: '重啟，先測 3 天', recommended: true },
  { id: 'close', label: '正式收攤' },
]
function q(partial: Partial<Question> = {}): Question {
  return {
    id: 'q', title: 't', question_line: '要重啟嗎？', body_md: '', kind: 'choice', tier: 't1',
    requested_tier: 't1', evidence: [], options: opts, allow_free_text: true,
    default_action: { on_expire: 'noop_log' }, wake_on_answer: { mode: 'none' },
    source: { agent: 'claude', cwd: '/w' }, state: 'pending', snooze_count: 0, forced_today_used: false,
    first_proposed: '2026-10-08', last_seen: 'x', created_at: 'x', next_transition_at: 'x',
    ...partial,
  } as Question
}

describe('canSend', () => {
  it('choice needs an option or text; text questions need text; confirm is always ready', () => {
    expect(canSend(q(), { selected: null, note: '' })).toBe(false)
    expect(canSend(q(), { selected: null, note: '   ' })).toBe(false)
    expect(canSend(q(), { selected: 'close', note: '' })).toBe(true)
    expect(canSend(q(), { selected: null, note: '其他做法' })).toBe(true)
    expect(canSend(q({ kind: 'text', options: [] }), { selected: null, note: '' })).toBe(false)
    expect(canSend(q({ kind: 'text', options: [] }), { selected: null, note: '日記' })).toBe(true)
    expect(canSend(q({ kind: 'confirm', options: [] }), { selected: null, note: '' })).toBe(true)
  })
})

describe('buildPayload', () => {
  it('option + note travel together; text alone is free_text; confirm sends confirm', () => {
    expect(buildPayload(q(), { selected: 'restart', note: ' 預算先 600 ' })).toEqual({
      question_id: 'q', type: 'choice', choice_id: 'restart', note: '預算先 600',
    })
    expect(buildPayload(q(), { selected: 'restart', note: '' })).toEqual({
      question_id: 'q', type: 'choice', choice_id: 'restart',
    })
    expect(buildPayload(q(), { selected: null, note: '都不要' })).toEqual({
      question_id: 'q', type: 'free_text', free_text: '都不要',
    })
    expect(buildPayload(q({ kind: 'confirm', options: [] }), { selected: null, note: '' })).toEqual({
      question_id: 'q', type: 'choice', choice_id: 'confirm',
    })
    expect(buildPayload(q(), { selected: null, note: '' })).toBeNull()
  })

  it('帶上作答當下看到的草稿版本與內容雜湊（AC-EC-05）', () => {
    const draft = { revision: 3, content_hash: 'sha256:x', registered_at: '2026-10-08T00:00:00Z' }
    expect(buildPayload(q({ draft }), { selected: 'restart', note: '' })).toEqual({
      question_id: 'q', type: 'choice', choice_id: 'restart', revision: 3, content_hash: 'sha256:x',
    })
    expect(buildPayload(q({ draft }), { selected: null, note: '都不要' })).toEqual({
      question_id: 'q', type: 'free_text', free_text: '都不要', revision: 3, content_hash: 'sha256:x',
    })
  })

  it('carries edited money params only with a chosen option', () => {
    expect(buildPayload(q(), { selected: 'restart', note: '' }, 'adset 1 500 → 450')).toEqual({
      question_id: 'q', type: 'choice', choice_id: 'restart', edited_params: 'adset 1 500 → 450',
    })
  })
})

describe('selection and summary', () => {
  it('clicking the same option again clears it', () => {
    const a = toggleSelect({ selected: null, note: '' }, 'close')
    expect(a.selected).toBe('close')
    expect(toggleSelect(a, 'close').selected).toBeNull()
    expect(toggleSelect(a, 'restart').selected).toBe('restart')
  })

  it('summarises the pending decision in words', () => {
    expect(decisionSummary(q(), { selected: 'restart', note: '' })).toBe('你的決定：重啟，先測 3 天')
    expect(decisionSummary(q(), { selected: 'restart', note: 'x' })).toBe('你的決定：重啟，先測 3 天＋補充說明')
    expect(decisionSummary(q(), { selected: null, note: 'x' })).toBe('你的決定：自己寫的回答')
    expect(decisionSummary(q(), { selected: null, note: '' })).toBe('選一個選項（按 1–2），或寫下你的做法')
  })
})

describe('surfaceKey', () => {
  const base = { shift: false, composing: false, inText: false, answered: false }
  it('numbers select only outside text fields', () => {
    expect(surfaceKey({ ...base, key: '2' })).toEqual({ type: 'select', index: 1 })
    expect(surfaceKey({ ...base, key: '2', inText: true })).toBeNull()
    expect(surfaceKey({ ...base, key: '5' })).toBeNull()
  })
  it('Enter submits, Shift+Enter and IME composition do not', () => {
    expect(surfaceKey({ ...base, key: 'Enter' })).toEqual({ type: 'submit' })
    expect(surfaceKey({ ...base, key: 'Enter', inText: true })).toEqual({ type: 'submit' })
    expect(surfaceKey({ ...base, key: 'Enter', inText: true, shift: true })).toBeNull()
    expect(surfaceKey({ ...base, key: 'Enter', inText: true, composing: true })).toBeNull()
  })
  it('Esc is not a column key (the surface owns it, see escapeAction); Enter after answering goes next', () => {
    expect(surfaceKey({ ...base, key: 'Escape', inText: true })).toBeNull()
    expect(surfaceKey({ ...base, key: 'Escape' })).toBeNull()
    expect(surfaceKey({ ...base, key: 'Enter', answered: true })).toEqual({ type: 'next' })
  })
})

// QA 第 1 輪 #1、#10（AC-DS-05／07）：Enter 依焦點所在的控制項決定；瀏覽器會把按鈕上的 Enter 當點擊。
describe('Enter 依焦點所在控制項', () => {
  const base = { shift: false, composing: false, inText: false, answered: false }
  it('焦點在選項上：攔下瀏覽器預設、送出目前的明確選取（不是改選焦點所在的選項）', () => {
    expect(surfaceKey({ ...base, key: 'Enter', focus: 'option' })).toEqual({ type: 'submit' })
  })
  it('焦點在送出鍵上：送出；回執裡的主按鈕：下一題', () => {
    expect(surfaceKey({ ...base, key: 'Enter', focus: 'submit' })).toEqual({ type: 'submit' })
    expect(surfaceKey({ ...base, key: 'Enter', focus: 'submit', answered: true })).toEqual({ type: 'next' })
  })
  it('焦點在其他按鈕或連結上：只啟動那個控制項——作答中不送出、回執裡不跳題', () => {
    expect(surfaceKey({ ...base, key: 'Enter', focus: 'control' })).toBeNull()
    expect(surfaceKey({ ...base, key: 'Enter', focus: 'control', answered: true })).toBeNull()
  })
  it('focusKind 把焦點元素分成選項、送出鍵、文字輸入、其他控制項', () => {
    expect(focusKind({ tag: 'BUTTON', role: 'radio' })).toBe('option')
    expect(focusKind({ tag: 'BUTTON', submit: true })).toBe('submit')
    expect(focusKind({ tag: 'BUTTON' })).toBe('control')
    expect(focusKind({ tag: 'A' })).toBe('control')
    expect(focusKind({ tag: 'SUMMARY' })).toBe('control')
    expect(focusKind({ tag: 'TEXTAREA' })).toBe('text')
    expect(focusKind({ tag: 'INPUT' })).toBe('text')
    expect(focusKind({ tag: 'BODY' })).toBe('none')
    expect(focusKind(null)).toBe('none')
  })
})

// QA 第 1 輪 #4（AC-DS-01、AC-OP-06）：Esc 從面板掛載就有效；只有網頁真的處理了 Esc 才通知原生已接手。
describe('escapeAction', () => {
  const base = { composing: false, local: false }
  it('張開動畫中與開啟後：稍後再問，並通知原生', () => {
    expect(escapeAction({ ...base, stage: 'opening' })).toEqual({ later: true, ack: true })
    expect(escapeAction({ ...base, stage: 'open' })).toEqual({ later: true, ack: true })
  })
  it('輸入法組字中只取消組字；行內編輯中只收起編輯——網頁都處理了', () => {
    expect(escapeAction({ ...base, stage: 'open', composing: true })).toEqual({ later: false, ack: true })
    // 行內編輯＝焦點真的在行內區塊的輸入框裡（2026-10-09 QA：行內區塊裡的按鈕上 Esc 是稍後再問）。
    expect(escapeAction({ ...base, stage: 'open', local: true, field: true })).toEqual({ later: false, ack: true })
  })
  it('收起中：不處理也不通知，留給原生後備', () => {
    expect(escapeAction({ ...base, stage: 'closing' })).toEqual({ later: false, ack: false })
  })
})

// QA 第 1 輪 #10（AC-DS-07）：面板是對話框，Tab 不會跑到面板後面。
describe('wrapFocus', () => {
  it('最後一個往後回到第一個、第一個往前到最後一個；焦點在面板外時拉回面板', () => {
    expect(wrapFocus(4, 3, false)).toBe(0)
    expect(wrapFocus(4, 0, true)).toBe(3)
    expect(wrapFocus(4, -1, false)).toBe(0)
    expect(wrapFocus(4, -1, true)).toBe(3)
  })
  it('中間的交給瀏覽器；沒有可聚焦元素不處理', () => {
    expect(wrapFocus(4, 1, false)).toBeNull()
    expect(wrapFocus(4, 2, true)).toBeNull()
    expect(wrapFocus(0, -1, false)).toBeNull()
  })
})

// QA 第 1 輪 #8（AC-DS-04 表，與 CONTRACT §9 一致）。
describe('primaryLabel', () => {
  const pick = (selected: string | null) => ({ selected, note: '' })
  const money = q({
    money: { is_money_op: true },
    options: [
      { id: 'go', label: '加碼到 600', recommended: true },
      { id: 'hold', label: '維持' },
    ],
  })
  const label = (question: Question, selected: string | null, extra: { sending?: boolean; locked?: boolean } = {}) =>
    primaryLabel({ question, draft: pick(selected), sending: extra.sending ?? false, locked: extra.locked ?? false })
  it('選擇題「送出決定」、文字題「送出回答」、確認題「確認」', () => {
    expect(label(q(), null)).toBe('送出決定')
    expect(label(q(), 'restart')).toBe('送出決定')
    expect(label(q({ kind: 'text', options: [] }), null)).toBe('送出回答')
    expect(label(q({ kind: 'choice', options: [] }), null)).toBe('送出回答')
    expect(label(q({ kind: 'confirm', options: [] }), null)).toBe('確認')
  })
  it('黃燈錢題選了執行型選項「批准並執行」；安全選項仍是「送出決定」', () => {
    expect(label(money, 'go')).toBe('批准並執行')
    expect(label(money, 'hold')).toBe('送出決定')
    expect(label(money, null)).toBe('送出決定')
  })
  it('錢題一鍵被鎖時選了選項「拍板此選項（不執行）」；送出中「送出中…」', () => {
    expect(label(money, 'go', { locked: true })).toBe('拍板此選項（不執行）')
    expect(label(money, 'hold', { locked: true })).toBe('拍板此選項（不執行）')
    expect(label(q(), 'restart', { sending: true })).toBe('送出中…')
  })
})

// AC-DS-07：選項是一個 Tab 停點（radiogroup 慣例），上下鍵在選項間移動焦點；順序：選項 → 補充說明 → 這題不該問我 → 送出。
describe('roving tabindex', () => {
  const opts = [{ id: 'a' }, { id: 'b' }, { id: 'c' }]
  it('停點在已選的選項，沒選就是第一個', () => {
    expect(optionTabStop(opts, null)).toBe(0)
    expect(optionTabStop(opts, 'c')).toBe(2)
    expect(optionTabStop(opts, 'zzz')).toBe(0)
  })
  it('方向鍵循環移動，Home／End 到頭尾，其他鍵不處理', () => {
    expect(rovingTarget(0, 'ArrowDown', 3)).toBe(1)
    expect(rovingTarget(2, 'ArrowDown', 3)).toBe(0)
    expect(rovingTarget(0, 'ArrowUp', 3)).toBe(2)
    expect(rovingTarget(1, 'ArrowRight', 3)).toBe(2)
    expect(rovingTarget(1, 'ArrowLeft', 3)).toBe(0)
    expect(rovingTarget(1, 'Home', 3)).toBe(0)
    expect(rovingTarget(1, 'End', 3)).toBe(2)
    expect(rovingTarget(1, 'Enter', 3)).toBeNull()
  })
})

// AC-DS-13：蓋板在所有視窗之上，往外開之前要先讓開（未答＝稍後再問、已答＝關閉），目標才不會開在蓋板後面。
describe('openOutside', () => {
  it('蓋板裡：先讓開再打開（讓開完成之後才打開，QA 第 2 輪）', async () => {
    const { openOutside } = await import('./decision')
    const calls: string[] = []
    openOutside({
      inOverlay: true,
      stepAside: async () => {
        calls.push('aside')
      },
      open: (href) => calls.push(`open ${href}`),
    })('claude://code/continue?session=local_x')
    expect(calls).toEqual(['aside'])
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(calls).toEqual(['aside', 'open claude://code/continue?session=local_x'])
  })

  it('主視窗：直接打開', async () => {
    const { openOutside } = await import('./decision')
    const calls: string[] = []
    openOutside({
      inOverlay: false,
      stepAside: () => {
        calls.push('aside')
      },
      open: (href) => calls.push(`open ${href}`),
    })('https://example.com')
    expect(calls).toEqual(['open https://example.com'])
  })
})
