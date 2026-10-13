// 題組的純邏輯（BDD batch-review AC-BR-02/03/04/05/07/08）：狀態轉換、總結、送出條件、載荷、鍵盤、回執。
import { describe, expect, it } from 'vitest'
import type { BatchItem, Question } from '../types'
import {
  approveAll,
  approveAllLabel,
  batchCounts,
  batchKey,
  batchSummary,
  buildBatchPayload,
  chooseAction,
  decisionStatus,
  editNote,
  editText,
  excerpt,
  initDrafts,
  itemStatus,
  receiptPlan,
  rebaseDrafts,
  redoAllBlocker,
  redoAllPayload,
  revertText,
  roundOf,
  sendBlocker,
  STATUS_LABEL,
} from './batch'

const item = (id: string, extra: Partial<BatchItem> = {}): BatchItem => ({
  id,
  title: `寄件人 ${id}`,
  context_md: `來信 ${id}`,
  proposal: `草稿 ${id}`,
  editable: true,
  default: 'approve',
  content_hash: `sha256:${id}`,
  ...extra,
})

const ITEMS: BatchItem[] = [
  item('i1'),
  item('i2'),
  // 2026-10-09 作者：AI 建議不處理的項目給空草稿（不准「（不回）」這類佔位文字）。
  item('i3', { default: 'skip', default_reason: '促銷信', proposal: '' }),
  item('i4', { editable: false, proposal: '建議全額退款 NT$4,800' }),
]

const question = (extra: Partial<Question> = {}): Question =>
  ({
    id: 'mail-1009',
    title: '今天 4 封信',
    question_line: '今天 4 封信的回覆',
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

describe('初始狀態（AC-BR-02／03）', () => {
  it('每項預設採用、AI 建議不處理的預設不處理；草稿是 AI 版本', () => {
    const drafts = initDrafts(ITEMS)
    expect(drafts.map((d) => d.action)).toEqual(['approve', 'approve', 'skip', 'approve'])
    expect(drafts[0]).toEqual({ action: 'approve', text: '草稿 i1', note: '' })
    expect(ITEMS.map((it, i) => STATUS_LABEL[itemStatus(it, drafts[i])])).toEqual(['採用', '採用', '不處理', '採用'])
  })
})

describe('逐項決定（AC-BR-03）', () => {
  it('改過的項目標「你改過」，還原回 AI 版本', () => {
    let drafts = editText(initDrafts(ITEMS), 0, '我自己改的')
    expect(itemStatus(ITEMS[0], drafts[0])).toBe('edited')
    expect(STATUS_LABEL.edited).toBe('你改過')
    drafts = revertText(ITEMS, drafts, 0)
    expect(drafts[0].text).toBe('草稿 i1')
    expect(itemStatus(ITEMS[0], drafts[0])).toBe('approve')
  })

  it('在不處理的項目上改草稿 → 改成採用；請 AI 改的項目改草稿不改決定', () => {
    let drafts = editText(initDrafts(ITEMS), 2, '還是回一下好了')
    expect(drafts[2].action).toBe('approve')
    drafts = chooseAction(drafts, 1, 'revise')
    drafts = editText(drafts, 1, '改一點')
    expect(drafts[1].action).toBe('revise')
  })

  it('再按一次同一個「不處理／請 AI 改」回到採用；切換不會清掉已改的文字與指示', () => {
    let drafts = editText(initDrafts(ITEMS), 0, '改過')
    drafts = chooseAction(drafts, 0, 'skip')
    expect(drafts[0].action).toBe('skip')
    drafts = chooseAction(drafts, 0, 'skip')
    expect(drafts[0].action).toBe('approve')
    expect(drafts[0].text).toBe('改過')
    drafts = editNote(chooseAction(drafts, 1, 'revise'), 1, '先道歉')
    drafts = chooseAction(drafts, 1, 'approve')
    expect(drafts[1].note).toBe('先道歉')
    expect(chooseAction(drafts, 1, 'approve')[1].action).toBe('approve')
  })

  it('不改傳入的陣列（React 狀態不可變）', () => {
    const drafts = initDrafts(ITEMS)
    chooseAction(drafts, 0, 'skip')
    editText(drafts, 1, 'x')
    expect(drafts[0].action).toBe('approve')
    expect(drafts[1].text).toBe('草稿 i2')
  })
})

describe('總結列（AC-BR-02）', () => {
  it('採用 N 項 · 其中 M 項你改過 · 不處理 K · 請 AI 改 L', () => {
    let drafts = editText(initDrafts(ITEMS), 0, '改過')
    drafts = editNote(chooseAction(drafts, 1, 'revise'), 1, '短一點')
    const counts = batchCounts(ITEMS, drafts)
    expect(counts).toEqual({ approve: 2, edited: 1, skip: 1, revise: 1 })
    expect(batchSummary(ITEMS, counts)).toBe('採用 2 項 · 其中 1 項你改過 · 不處理 1 · 請 AI 改 1')
  })

  it('沒有的類別不寫；只核可的題組寫「照建議」', () => {
    const plain = ITEMS.slice(0, 2)
    expect(batchSummary(plain, batchCounts(plain, initDrafts(plain)))).toBe('採用 2 項')
    const approveOnly = [item('r1', { editable: false }), item('r2', { editable: false, default: 'skip', default_reason: '不符規則' })]
    expect(batchSummary(approveOnly, batchCounts(approveOnly, initDrafts(approveOnly)))).toBe('照建議 1 項 · 不處理 1')
    expect(approveAllLabel(approveOnly)).toBe('全部照 AI 建議')
    expect(approveAllLabel(ITEMS)).toBe('全部照 AI 建議')
    expect(approveAllLabel(plain)).toBe('全部採用 AI 草稿')
  })
})

describe('送出條件（AC-BR-03）', () => {
  it('請 AI 改沒寫指示 → 停用並說明是第幾項', () => {
    const drafts = chooseAction(initDrafts(ITEMS), 1, 'revise')
    expect(sendBlocker(ITEMS, drafts)).toBe('第 2 項選了「請 AI 改」，要寫一句怎麼改')
    expect(sendBlocker(ITEMS, editNote(drafts, 1, '   '))).not.toBeNull()
    expect(sendBlocker(ITEMS, editNote(drafts, 1, '先道歉'))).toBeNull()
  })

  it('採用的草稿是空的、或超過 5,000 字 → 停用', () => {
    // 2026-10-09 作者核准的說法：「第 N 項沒有內容，先寫好再採用」（server 拒收時同一句）。
    expect(sendBlocker(ITEMS, editText(initDrafts(ITEMS), 0, '  \n'))).toBe('第 1 項沒有內容，先寫好再採用')
    expect(sendBlocker(ITEMS, chooseAction(initDrafts(ITEMS), 2, 'approve'))).toBe('第 3 項沒有內容，先寫好再採用')
    expect(sendBlocker(ITEMS, editText(initDrafts(ITEMS), 1, '字'.repeat(5001)))).toBe('第 2 項的草稿超過 5,000 字')
    expect(sendBlocker(ITEMS, editText(initDrafts(ITEMS), 1, '字'.repeat(5000)))).toBeNull()
    expect(sendBlocker(ITEMS, chooseAction(editText(initDrafts(ITEMS), 0, ''), 0, 'skip'))).toBeNull()
  })
})

describe('一鍵處理全部（AC-BR-04）', () => {
  it('全部採用：非不處理的都改採用、保留改過的文字、清掉請 AI 改的指示', () => {
    let drafts = editText(initDrafts(ITEMS), 0, '改過')
    drafts = editNote(chooseAction(drafts, 1, 'revise'), 1, '短一點')
    const all = approveAll(drafts)
    expect(all.map((d) => d.action)).toEqual(['approve', 'approve', 'skip', 'approve'])
    expect(all[0].text).toBe('改過')
    expect(all.every((d) => d.note === '')).toBe(true)
  })

  it('全部退回：每項都是請 AI 改，附同一段指示；沒寫指示不能送', () => {
    expect(redoAllPayload(question(), '   ')).toBeNull()
    const payload = redoAllPayload(question(), '  語氣正式一點  ')
    expect(payload).toEqual({
      question_id: 'mail-1009',
      type: 'batch',
      instruction: '語氣正式一點',
      items: ITEMS.map((it) => ({ item_id: it.id, action: 'revise', note: '語氣正式一點', content_hash: it.content_hash })),
    })
  })
})

describe('送出載荷（AC-BR-05：所見即所送）', () => {
  it('逐項帶決定、採用的可編輯項目帶最後文字（原樣，不修剪）、每項帶雜湊', () => {
    let drafts = editText(initDrafts(ITEMS), 0, '  改過的文字\n')
    drafts = editNote(chooseAction(drafts, 1, 'revise'), 1, ' 先道歉 ')
    const payload = buildBatchPayload(question(), drafts, '')
    expect(payload).toEqual({
      question_id: 'mail-1009',
      type: 'batch',
      items: [
        { item_id: 'i1', action: 'approve', text: '  改過的文字\n', content_hash: 'sha256:i1' },
        { item_id: 'i2', action: 'revise', note: '先道歉', content_hash: 'sha256:i2' },
        { item_id: 'i3', action: 'skip', content_hash: 'sha256:i3' },
        { item_id: 'i4', action: 'approve', content_hash: 'sha256:i4' },
      ],
    })
  })

  it('草稿題帶作答當下的版本與內容雜湊；有整體指示就帶上', () => {
    const q = question({ draft: { revision: 3, content_hash: 'sha256:v3', registered_at: '2026-10-09T01:00:00Z' } })
    const payload = buildBatchPayload(q, initDrafts(ITEMS), ' 全部早上寄 ')
    expect(payload).toMatchObject({ revision: 3, content_hash: 'sha256:v3', instruction: '全部早上寄' })
  })
})

describe('鍵盤（AC-BR-07）', () => {
  const base = { meta: false, ctrl: false, composing: false, inText: false, answered: false }
  it('J／K、↑／↓ 換項目；E 編輯；A／X／R 決定', () => {
    expect(batchKey({ ...base, key: 'j' })).toEqual({ type: 'move', delta: 1 })
    expect(batchKey({ ...base, key: 'ArrowDown' })).toEqual({ type: 'move', delta: 1 })
    expect(batchKey({ ...base, key: 'K' })).toEqual({ type: 'move', delta: -1 })
    expect(batchKey({ ...base, key: 'ArrowUp' })).toEqual({ type: 'move', delta: -1 })
    expect(batchKey({ ...base, key: 'e' })).toEqual({ type: 'edit' })
    expect(batchKey({ ...base, key: 'a' })).toEqual({ type: 'decide', action: 'approve' })
    expect(batchKey({ ...base, key: 'x' })).toEqual({ type: 'decide', action: 'skip' })
    expect(batchKey({ ...base, key: 'r' })).toEqual({ type: 'decide', action: 'revise' })
  })

  it('⌘Enter 送出（輸入框內也可以）；單按 Enter 不送出', () => {
    expect(batchKey({ ...base, key: 'Enter', meta: true })).toEqual({ type: 'submit' })
    expect(batchKey({ ...base, key: 'Enter', ctrl: true, inText: true })).toEqual({ type: 'submit' })
    expect(batchKey({ ...base, key: 'Enter' })).toBeNull()
    expect(batchKey({ ...base, key: 'Enter', meta: true, composing: true })).toBeNull()
  })

  it('輸入框內打字不觸發快捷鍵；Esc 離開輸入框；不在輸入框時的 Esc 交給面板（稍後再問只觸發一次）', () => {
    expect(batchKey({ ...base, key: 'a', inText: true })).toBeNull()
    expect(batchKey({ ...base, key: 'j', inText: true })).toBeNull()
    expect(batchKey({ ...base, key: 'Escape', inText: true })).toEqual({ type: 'blur' })
    expect(batchKey({ ...base, key: 'Escape' })).toBeNull()
    expect(batchKey({ ...base, key: 'Escape', answered: true })).toBeNull()
  })

  it('⌘A 等系統快捷鍵不攔；送出後 Enter＝下一題', () => {
    expect(batchKey({ ...base, key: 'a', meta: true })).toBeNull()
    expect(batchKey({ ...base, key: 'Enter', answered: true })).toEqual({ type: 'next' })
    expect(batchKey({ ...base, key: 'a', answered: true })).toBeNull()
  })
})

describe('卡片與回執（AC-BR-02／08）', () => {
  it('一行摘要取原文第一段的純文字', () => {
    expect(excerpt('## 主旨\n\n老師好，**第 5 章**的[影片](https://x)打不開')).toBe('主旨')
    expect(excerpt('\n\n> 老師好，**第 5 章**的[影片](https://x)打不開')).toBe('老師好，第 5 章的影片打不開')
    expect(excerpt('字'.repeat(200), 20)).toBe('字'.repeat(19) + '…')
  })

  it('第幾輪看題目 id 的 -rN', () => {
    expect(roundOf('mail-1009')).toBe(1)
    expect(roundOf('mail-1009-r2')).toBe(2)
    expect(roundOf('mail-1009-r12')).toBe(12)
    expect(roundOf('r2')).toBe(1)
  })

  it('回執說明 AI 接下來會做什麼', () => {
    const answered = question({
      state: 'answered',
      answer: {
        channel: 'ui',
        answered_at: '2026-10-09T01:05:00Z',
        items: [
          { item_id: 'i1', action: 'approve', text: '改過' },
          { item_id: 'i2', action: 'revise', note: '短一點' },
          { item_id: 'i3', action: 'skip' },
          { item_id: 'i4', action: 'approve' },
        ],
      },
    })
    expect(receiptPlan(answered)).toEqual([
      'AI 會用你確認的文字逐項送出 2 項，一字不改',
      '請 AI 改的 1 項改好後會以「第 2 輪」再送來',
      '不處理的 1 項會封存，不回覆',
    ])
    const decisions = answered.answer!.items!
    expect(decisions.map((d) => STATUS_LABEL[decisionStatus(ITEMS.find((it) => it.id === d.item_id), d)])).toEqual([
      '你改過',
      '請 AI 改',
      '不處理',
      '採用',
    ])
  })

  it('全部退回的回執', () => {
    const answered = question({
      id: 'mail-1009-r2',
      state: 'answered',
      answer: {
        channel: 'ui',
        answered_at: '2026-10-09T01:05:00Z',
        free_text: '語氣正式一點',
        items: ITEMS.map((it) => ({ item_id: it.id, action: 'revise' as const, note: '語氣正式一點' })),
      },
    })
    expect(receiptPlan(answered)).toEqual(['全部退回，AI 會依你的指示重寫：「語氣正式一點」', '改好後會以「第 3 輪」再送來給你看'])
  })
})

// ---- QA 第 1 輪（2026-10-09，作者核准）-------------------------------------------------------------

describe('全部退回的整體指示長度（AC-BR-04）', () => {
  it('超過 2,000 字先擋：說明是整體指示、不組載荷；剛好 2,000 字可以送', () => {
    expect(redoAllBlocker('改'.repeat(2001))).toBe('整體指示超過 2,000 字（2,001 字），先縮短再送出')
    expect(redoAllPayload(question(), '改'.repeat(2001))).toBeNull()
    expect(redoAllBlocker(`  ${'改'.repeat(2000)}  `)).toBeNull()
    expect(redoAllPayload(question(), '改'.repeat(2000))).not.toBeNull()
    expect(redoAllBlocker('')).toBeNull()
  })
})

describe('面板開著時草稿重新註冊（AC-BR-09）', () => {
  const v2 = (extra: Partial<BatchItem>[] = []) => ITEMS.map((it, i) => ({ ...it, content_hash: `${it.content_hash}-v2`, ...(extra[i] ?? {}) }))

  it('AI 草稿沒變的項目保留使用者的文字、決定與指示', () => {
    let drafts = editText(initDrafts(ITEMS), 0, '我改過的')
    drafts = editNote(chooseAction(drafts, 1, 'revise'), 1, '短一點')
    const next = rebaseDrafts(ITEMS, v2(), drafts)
    expect(next[0]).toMatchObject({ action: 'approve', text: '我改過的' })
    expect(next[1]).toMatchObject({ action: 'revise', note: '短一點' })
    expect(next.some((d) => d.updated)).toBe(false)
  })

  it('AI 草稿變了：使用者改過的文字保留並標「AI 更新了這一項」，還原 AI 版本＝新版本；沒改過的直接換成新版本', () => {
    const drafts = editText(initDrafts(ITEMS), 0, '我改過的')
    const items = v2([{ proposal: '新草稿 i1' }, { proposal: '新草稿 i2' }])
    const next = rebaseDrafts(ITEMS, items, drafts)
    expect(next[0]).toMatchObject({ action: 'approve', text: '我改過的', updated: true })
    expect(next[1]).toMatchObject({ action: 'approve', text: '新草稿 i2', updated: true })
    expect(revertText(items, next, 0)[0].text).toBe('新草稿 i1')
    expect(itemStatus(items[1], next[1])).toBe('approve')
  })

  it('新加入的項目照預設；被拿掉的項目消失；使用者動過的決定不被新預設蓋掉，沒動過的跟著新預設', () => {
    const drafts = chooseAction(initDrafts(ITEMS), 0, 'skip')
    const items: BatchItem[] = [
      { ...ITEMS[0], proposal: '新草稿 i1', default: 'approve' },
      { ...ITEMS[1], proposal: '新草稿 i2', default: 'skip', default_reason: '其實不用回' },
      item('i5'),
    ]
    const next = rebaseDrafts(ITEMS, items, drafts)
    expect(next).toHaveLength(3)
    expect(next[0].action).toBe('skip')
    expect(next[1].action).toBe('skip')
    expect(next[2]).toEqual({ action: 'approve', text: '草稿 i5', note: '' })
  })
})

describe('回執裡的 Enter（AC-BR-08、AC-DS-07）', () => {
  const base = { meta: false, ctrl: false, composing: false, inText: false, answered: true }
  it('焦點在回執主按鈕或沒有焦點時 Enter＝下一題；焦點在連結或其他按鈕上只啟動那個控制項', () => {
    expect(batchKey({ ...base, key: 'Enter', focus: 'submit' })).toEqual({ type: 'next' })
    expect(batchKey({ ...base, key: 'Enter', focus: 'none' })).toEqual({ type: 'next' })
    expect(batchKey({ ...base, key: 'Enter', focus: 'control' })).toBeNull()
  })
})
