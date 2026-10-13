// QA 題組第 1 輪（BDD batch-review AC-BR-02..08；DESIGN「Batch review」「Decision surface」「Routines view」）。
// 每個測試的斷言寫的是規格期望；失敗訊息寫實際行為。標「應綠」的是確認現行實作成立的對照組。
// 只能在瀏覽器看到的行為（焦點、textarea 高度）另記在 QA 報告，這裡只放能在 node 重現的部分。
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'
import type { KunsuIpc } from '../ipc'
import type { BatchItem, Question } from '../types'
import { answerLine, routineHistory } from '../routines/routines'
import { BatchReview } from './BatchColumn'
import { batchKey, buildBatchPayload, chooseAction, initDrafts, redoAllPayload, sendBlocker } from './batch'
import { escapeAction } from './decision'

const ipc = { openDeepLink: async () => {}, getQuestion: async () => null } as unknown as KunsuIpc

const ITEMS: BatchItem[] = [
  { id: 'i1', title: '陳同學', context_md: '老師好，第 5 章影片一直轉圈圈。', proposal: '陳同學你好，已經修好了。', editable: true, default: 'approve', content_hash: 'sha256:1' },
  { id: 'i2', title: '某品牌窗口', context_md: '想邀請您開箱冷萃新品。', proposal: '您好，謝謝邀請！', editable: true, default: 'approve', content_hash: 'sha256:2' },
  // 舊的 skill 草稿與 server 測試用「（不回）」當 AI 建議不處理項目的 proposal。2026-10-09 作者的新規則：不准佔位文字，
  // AI 建議不處理的可編輯項目給空草稿或真的回覆（plugins/kunsu/skills/kunsu/references/batch.md），所以這裡改成空草稿。
  { id: 'i3', title: '某軟體廠商', context_md: '升級企業方案享 8 折。', proposal: '', editable: true, default: 'skip', default_reason: '促銷信', content_hash: 'sha256:3' },
  { id: 'i4', title: '張同學退款', context_md: '購買 3 天、進度 8%。', proposal: '建議全額退款 NT$4,800', editable: false, default: 'approve', content_hash: 'sha256:4' },
]

const question = (extra: Partial<Question> = {}): Question =>
  ({
    id: 'mail-1009',
    title: '今天 4 件事',
    question_line: '今天 4 件事的回覆',
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

const answered = (extra: Partial<Question> = {}): Question =>
  question({
    state: 'answered',
    answer: {
      answer_id: 'a1',
      channel: 'ui',
      answered_at: '2026-10-09T01:05:00Z',
      items: [
        { item_id: 'i1', action: 'approve', text: '陳同學你好，修好了！', content_hash: 'sha256:1' },
        { item_id: 'i2', action: 'approve', text: '您好，謝謝邀請！', content_hash: 'sha256:2' },
        { item_id: 'i3', action: 'skip', content_hash: 'sha256:3' },
        { item_id: 'i4', action: 'approve', content_hash: 'sha256:4' },
      ],
    },
    ...extra,
  })

const render = (q: Question) =>
  renderToStaticMarkup(<BatchReview ipc={ipc} question={q} why={null} evidence={null} isLast active={false} onUpdated={() => {}} onNext={() => {}} />)

describe('AC-BR-07：Esc', () => {
  // BDD 場景 7：「Esc 離開輸入框（不在輸入框時 Esc＝稍後再問）」；CLAUDE.md §6／DESIGN「Strict, never stuck」：Esc 在任何狀態下有效。
  // 「還原 AI 版本」（.batch-draft 內）、全部退回的「取消」「全部退回重寫」（.batch-redo 內）、「確認退回」（.surface-reject 內）
  // 都是按鈕、不是輸入框，但都在 data-escape="local" 區塊裡：面板把它當行內編輯不稍後再問，題組又只在 INPUT／TEXTAREA 上處理 Esc。
  it('焦點在 data-escape="local" 區塊裡的按鈕上時，Esc 要嘛稍後再問、要嘛離開該區塊（不能什麼都不做）', () => {
    const surface = escapeAction({ stage: 'open', composing: false, local: true })
    const batch = batchKey({ key: 'Escape', meta: false, ctrl: false, composing: false, inText: false, answered: false })
    expect(
      surface.later || batch !== null,
      `實際：面板 later=${surface.later}（但 ack=${surface.ack}，原生 2 秒後備也不會接手），題組 batchKey=${JSON.stringify(batch)}——這個 Esc 沒有任何效果`,
    ).toBe(true)
  })

  it('應綠：輸入框裡的 Esc 只離開輸入框；輸入框外、非 local 區塊的 Esc 交給面板稍後再問', () => {
    expect(batchKey({ key: 'Escape', meta: false, ctrl: false, composing: false, inText: true, answered: false })).toEqual({ type: 'blur' })
    expect(escapeAction({ stage: 'open', composing: false, local: false })).toEqual({ later: true, ack: true })
  })
})

describe('AC-BR-03／05：AI 建議不處理的項目改成採用（2026-10-09 作者核准的規則）', () => {
  // 舊版：DESIGN「原因寫在草稿裡」、skill 範例 proposal 是「（不回）」→ 使用者改成採用而沒動草稿，agent 會把「（不回）」一字不改寄出。
  // 使用者的規則：不准佔位文字（AI 給空草稿或真的回覆），可編輯項目採用時最終文字不能是空的——面板停用送出並說明，server 也拒收。
  it('預設不處理的項目沒改草稿就改成採用，送出前要攔下或提醒（不能把佔位文字當成要寄出的文字）', () => {
    const drafts = chooseAction(initDrafts(ITEMS), 2, 'approve')
    const payload = buildBatchPayload(question(), drafts)
    const sent = payload.type === 'batch' ? payload.items[2] : undefined
    expect(
      sendBlocker(ITEMS, drafts),
      `實際：沒有任何攔阻，送出載荷第 3 項是 ${JSON.stringify(sent)}`,
    ).toBe('第 3 項沒有內容，先寫好再採用')
  })

  it('只有空白也算沒有內容；寫了真的內容就可以送', () => {
    const blank = chooseAction(initDrafts(ITEMS), 2, 'approve').map((d, i) => (i === 2 ? { ...d, text: '  \n ' } : d))
    expect(sendBlocker(ITEMS, blank)).toBe('第 3 項沒有內容，先寫好再採用')
    const written = blank.map((d, i) => (i === 2 ? { ...d, text: '謝謝，暫時不需要。' } : d))
    expect(sendBlocker(ITEMS, written)).toBeNull()
  })
})

describe('AC-BR-04：全部退回的指示長度', () => {
  // CONTRACT §14：請 AI 改的指示與整體指示都 ≤2,000 字。全部退回時 UI 沒有檢查，「全部退回重寫」可按；
  // server 拒收時錯誤寫「第 1 項的指示超過 2,000 字」，但使用者寫的是一段整體指示，看不懂是哪一項。
  it('超過 2,000 字的全部退回指示在送出前就攔下', () => {
    const payload = redoAllPayload(question(), '改'.repeat(2001))
    const items = payload?.type === 'batch' ? payload.items.length : 0
    expect(payload === null, `實際：照樣組出載荷（${items} 項，每項附 2,001 字指示），送出後才被 server 以「第 1 項的指示超過 2,000 字」拒收`).toBe(true)
  })
})

describe('AC-BR-08：回執', () => {
  // DESIGN「Decision surface」：回執裡題目「render as Markdown, exactly as while deciding」。作答中用 RichContent inline，回執用純文字。
  it('回執的題目和作答時一樣渲染 Markdown', () => {
    const line = '今天 **4 件事**的回覆'
    const deciding = render(question({ question_line: line }))
    const receipt = render(answered({ question_line: line }))
    expect(deciding).toContain('<strong>4 件事</strong>')
    const h2 = receipt.match(/<h2 class="surface-ask">.*?<\/h2>/)?.[0]
    expect(receipt.includes('<strong>4 件事</strong>'), `實際：回執標題是 ${h2}（原始字串，看得到 ** 記號）`).toBe(true)
  })

  it('應綠：回執逐項列出決定、總數與後續（逐項送出幾項、封存幾項）', () => {
    const html = render(answered())
    expect(html).toContain('每一項的決定')
    expect(html).toContain('AI 會用你確認的文字逐項送出 3 項，一字不改')
    expect(html).toContain('不處理的 1 項會封存，不回覆')
  })
})

describe('整合：督導頁（DESIGN「Routines view」：每筆紀錄顯示使用者的完整回答）', () => {
  // 題組常由排程產生（skill 範例「09:00 排程收信」；mock 的題組就是 routine「每日收信」）。題組答案沒有 choice_id，
  // 整體指示通常也沒有，answerText 回 null → 狀態「已回應」旁邊寫「你的回答：沒有回答」。
  it('排程題組已逐項作答時，「你的回答」要寫出逐項決定的摘要，不能寫「沒有回答」', () => {
    const q = answered({ source: { agent: 'claude', cwd: '/w', routine_id: 'mail-daily', routine_name: '每日收信', trigger: 'scheduled' } })
    const [entry] = routineHistory([q], 'mail-daily')
    expect(entry.status).toBe('answered')
    expect(answerLine(entry), `實際：狀態「已回應」，你的回答卻是「${answerLine(entry)}」`).not.toBe('沒有回答')
  })
})

describe('應綠：所見即所送的載荷', () => {
  it('採用的文字原樣（含前後空白與空行）、每項帶面板看到的雜湊；只核可項目不帶文字；草稿題帶 revision 與內容雜湊', () => {
    const text = '  陳同學你好：\n\n修好了。  '
    const drafts = initDrafts(ITEMS).map((d, i) => (i === 0 ? { ...d, text } : d))
    const payload = buildBatchPayload(question({ draft: { revision: 3, content_hash: 'sha256:v3' } as Question['draft'] }), drafts)
    expect(payload).toMatchObject({ type: 'batch', revision: 3, content_hash: 'sha256:v3' })
    if (payload.type !== 'batch') throw new Error('要是題組載荷')
    expect(payload.items[0]).toEqual({ item_id: 'i1', action: 'approve', text, content_hash: 'sha256:1' })
    expect(payload.items[3]).toEqual({ item_id: 'i4', action: 'approve', content_hash: 'sha256:4' })
    expect(payload.items.every((d) => d.content_hash)).toBe(true)
  })

  it('單按 Enter 不送出；⌘／Ctrl+Enter 送出；輸入法組字中一律不處理', () => {
    const k = (key: string, extra: Partial<Parameters<typeof batchKey>[0]> = {}) =>
      batchKey({ key, meta: false, ctrl: false, composing: false, inText: false, answered: false, ...extra })
    expect(k('Enter')).toBeNull()
    expect(k('Enter', { inText: true })).toBeNull()
    expect(k('Enter', { meta: true, inText: true })).toEqual({ type: 'submit' })
    expect(k('Enter', { ctrl: true })).toEqual({ type: 'submit' })
    expect(k('Enter', { meta: true, composing: true })).toBeNull()
    expect(k('a', { inText: true })).toBeNull()
  })
})
