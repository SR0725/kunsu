// QA 題組第 2 輪（BDD batch-review AC-BR-03／04／07／09；DESIGN「Batch review」）：第 1 輪修正（merge d3a0d2d）之後的
// 獨立 QA。這裡放本輪在瀏覽器（vite mock，`?view=overlay`）實測成立的情境，寫成 node 可重現的回歸保護。
// 本輪題組相關的缺口：主視窗的作答 flow 不跟著最新清單（重新註冊不 rebase、送出帶舊雜湊被拒收）——與一般題同一個根因，
// 測試在 `qaDecisionRound3.test.tsx`「AC-DS-16（與 AC-BR-09）」；開新對話走 stdin 的驗證在
// `crates/kunsu-server/tests/qa_batch_round2.rs`（假 claude／codex 腳本記錄 argv 與 stdin）。
import { describe, expect, it } from 'vitest'
import type { BatchItem, Question } from '../types'
import {
  batchCounts,
  batchKey,
  batchSummary,
  buildBatchPayload,
  chooseAction,
  editNote,
  editText,
  initDrafts,
  itemStatus,
  rebaseDrafts,
  redoAllBlocker,
  redoAllPayload,
  revertText,
  sendBlocker,
} from './batch'
import { escapeAction } from './decision'

const item = (n: number, extra: Partial<BatchItem> = {}): BatchItem => ({
  id: `i${n}`,
  title: `第 ${n} 封`,
  context_md: `來信 ${n}`,
  proposal: `AI 第 ${n} 封`,
  editable: true,
  default: 'approve',
  content_hash: `sha256:v1-${n}`,
  ...extra,
})

const ITEMS: BatchItem[] = [
  item(1),
  item(2),
  item(3),
  item(4),
  item(5),
  item(6, { proposal: '', default: 'skip', default_reason: '促銷信' }),
]

const question = (items: BatchItem[]): Question =>
  ({
    id: 'mail-qa-r2',
    title: '今天 6 封信',
    question_line: '今天 6 封信的回覆',
    body_md: '',
    kind: 'batch',
    tier: 't1',
    requested_tier: 't1',
    evidence: [],
    options: [],
    items,
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
    draft: { revision: 1, content_hash: 'sha256:d1', registered_at: '2026-10-09T00:00:00Z' },
  }) as Question

describe('AC-BR-09 面板開著時重新註冊（瀏覽器實測情境）', () => {
  // 使用者：改了第 2、5 封的文字、第 3 封不處理、第 4 封請 AI 改「先道歉」。AI 重新註冊：第 1、2 封的草稿換了，第 5 封只有
  // 原文多了補充（草稿沒變、雜湊變了）。
  let drafts = initDrafts(ITEMS)
  drafts = editText(drafts, 1, '使用者改過的第 2 封')
  drafts = editText(drafts, 4, '使用者改過的第 5 封')
  drafts = chooseAction(drafts, 2, 'skip')
  drafts = chooseAction(drafts, 3, 'revise')
  drafts = editNote(drafts, 3, '先道歉')
  const next: BatchItem[] = ITEMS.map((it, i) =>
    i === 0 || i === 1
      ? { ...it, proposal: `AI 新版第 ${i + 1} 封`, content_hash: `sha256:v2-${i + 1}` }
      : i === 4
        ? { ...it, context_md: `${it.context_md}（補充）`, content_hash: 'sha256:v2-5' }
        : it,
  )
  const rebased = rebaseDrafts(ITEMS, next, drafts)

  it('使用者沒改過的換成新版本並標「AI 更新了這一項」；改過的保留文字也標更新；其他決定與指示原樣保留', () => {
    expect(rebased.map((d) => [d.action, d.text, d.note, Boolean(d.updated)])).toEqual([
      ['approve', 'AI 新版第 1 封', '', true],
      ['approve', '使用者改過的第 2 封', '', true],
      ['skip', 'AI 第 3 封', '', false],
      ['revise', 'AI 第 4 封', '先道歉', false],
      ['approve', '使用者改過的第 5 封', '', true], // 2026-10-10：原文改變也必須提醒，避免舊審閱註記誤導
      ['skip', '', '', false],
    ])
    expect(itemStatus(next[1], rebased[1])).toBe('edited')
    expect(revertText(next, rebased, 1)[1].text).toBe('AI 新版第 2 封')
    expect(batchSummary(next, batchCounts(next, rebased))).toBe('採用 3 項 · 其中 2 項你改過 · 不處理 2 · 請 AI 改 1')
  })

  it('送出帶的是新版本的雜湊與使用者確認的文字（不會用舊雜湊被拒收）', () => {
    const payload = buildBatchPayload(question(next), rebased)
    if (payload.type !== 'batch') throw new Error('應是題組載荷')
    expect(payload.items.map((d) => [d.item_id, d.action, d.content_hash, 'text' in d ? d.text : undefined])).toEqual([
      ['i1', 'approve', 'sha256:v2-1', 'AI 新版第 1 封'],
      ['i2', 'approve', 'sha256:v2-2', '使用者改過的第 2 封'],
      ['i3', 'skip', 'sha256:v1-3', undefined],
      ['i4', 'revise', 'sha256:v1-4', undefined],
      ['i5', 'approve', 'sha256:v2-5', '使用者改過的第 5 封'],
      ['i6', 'skip', 'sha256:v1-6', undefined],
    ])
  })
})

describe('AC-BR-03／04 送出前擋下（瀏覽器實測：第 9 封空草稿按 A、整體指示 2,001 字）', () => {
  it('AI 建議不處理、草稿是空的項目被改成採用：送出停用並寫「第 N 項沒有內容，先寫好再採用」；寫了內容就可以送', () => {
    const approved = chooseAction(initDrafts(ITEMS), 5, 'approve')
    expect(sendBlocker(ITEMS, approved)).toBe('第 6 項沒有內容，先寫好再採用')
    expect(sendBlocker(ITEMS, editText(approved, 5, '   '))).toBe('第 6 項沒有內容，先寫好再採用')
    expect(sendBlocker(ITEMS, editText(approved, 5, '謝謝通知，暫時不需要。'))).toBeNull()
  })

  it('整體指示剛好 2,000 字可以送；2,001 字擋下且不組載荷', () => {
    const q = question(ITEMS)
    expect(redoAllBlocker('請'.repeat(2000))).toBeNull()
    expect(redoAllPayload(q, '請'.repeat(2000))).not.toBeNull()
    expect(redoAllBlocker('請'.repeat(2001))).toBe('整體指示超過 2,000 字（2,001 字），先縮短再送出')
    expect(redoAllPayload(q, '請'.repeat(2001))).toBeNull()
  })
})

describe('AC-BR-07 鍵盤（瀏覽器實測：⌘Enter、E、Esc 在草稿裡、Esc 在「還原 AI 版本」上）', () => {
  const key = (k: string, extra: Partial<Parameters<typeof batchKey>[0]> = {}) =>
    batchKey({ key: k, meta: false, ctrl: false, composing: false, inText: false, answered: false, ...extra })

  it('⌘Enter 送出（輸入框裡也可以）；單按 Enter 不送出；E 進入草稿', () => {
    expect(key('Enter', { meta: true })).toEqual({ type: 'submit' })
    expect(key('Enter', { meta: true, inText: true })).toEqual({ type: 'submit' })
    expect(key('Enter')).toBeNull()
    expect(key('e')).toEqual({ type: 'edit' })
  })

  it('Esc 在草稿框裡＝離開輸入框（面板不稍後再問）；在行內區塊的按鈕上＝稍後再問（題組不處理，交給面板）', () => {
    expect(key('Escape', { inText: true })).toEqual({ type: 'blur' })
    expect(escapeAction({ stage: 'open', composing: false, local: true, field: true })).toEqual({ later: false, ack: true })
    expect(key('Escape')).toBeNull()
    expect(escapeAction({ stage: 'open', composing: false, local: true, field: false })).toEqual({ later: true, ack: true })
  })

  it('回執：焦點在項目列（按鈕）上 Enter 只啟動那一列，不跳下一題；焦點在「下一題」上 Enter 前往下一題', () => {
    expect(key('Enter', { answered: true, focus: 'control' })).toBeNull()
    expect(key('Enter', { answered: true, focus: 'submit' })).toEqual({ type: 'next' })
  })
})
