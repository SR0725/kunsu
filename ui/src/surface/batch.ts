// 題組的純邏輯（PRD §25、BDD batch-review、CONTRACT §14）：逐項狀態、總結、送出條件、載荷、鍵盤與回執。
// 元件（BatchColumn.tsx）只負責呈現與焦點；所有決定都經過這裡，方便測試。
import type { AnswerPayload } from '../ipc'
import type { BatchItem, ItemAction, ItemDecision, Question } from '../types'
import type { FocusKind } from './decision'

/** 與 server 相同的上限（kunsu_core::batch）。 */
export const MAX_TEXT = 5000
export const MAX_NOTE = 2000

/** 一項在面板上的草稿狀態：決定、目前的文字、請 AI 改的指示。 */
export interface ItemDraft {
  action: ItemAction
  text: string
  note: string
  /** 面板開著時 AI 重新註冊、這一項的草稿變了（卡片寫「AI 更新了這一項」，AC-BR-09）。 */
  updated?: boolean
  /** Local review bookmark only; never execution authorization. */
  reviewed?: boolean
}

export type ItemStatus = 'approve' | 'edited' | 'skip' | 'revise'

/** 狀態一律有文字（不只靠顏色）。 */
export const STATUS_LABEL: Record<ItemStatus, string> = {
  approve: '採用',
  edited: '你改過',
  skip: '不處理',
  revise: '請 AI 改',
}

const defaultAction = (item: BatchItem): ItemAction => (item.default === 'skip' ? 'skip' : 'approve')

/** 開題：每項預設採用；AI 建議不處理的預設不處理；文字是 AI 的版本。 */
export function initDrafts(items: BatchItem[]): ItemDraft[] {
  return items.map((item) => ({ action: defaultAction(item), text: item.proposal, note: '' }))
}

/**
 * 面板開著時草稿重新註冊（AC-BR-09）：不靜默清掉使用者的修改。AI 草稿（proposal）沒變的項目原樣保留；變了的項目標
 * `updated`——使用者改過的文字保留（「還原 AI 版本」還原成新版本），沒改過的換成新版本；使用者沒動過的決定跟著新預設。
 * 新加入的項目照預設，被拿掉的項目消失。
 */
export function rebaseDrafts(oldItems: BatchItem[], newItems: BatchItem[], drafts: ItemDraft[]): ItemDraft[] {
  return newItems.map((item) => {
    const index = oldItems.findIndex((old) => old.id === item.id)
    const old = oldItems[index]
    const draft = drafts[index]
    if (!old || !draft) return { action: defaultAction(item), text: item.proposal, note: '' }
    if (old.proposal === item.proposal && old.default === item.default && old.context_md === item.context_md && old.subtitle === item.subtitle && old.title === item.title && old.editable === item.editable && old.default_reason === item.default_reason) return draft
    const edited = old.editable && draft.text !== old.proposal
    const untouched = draft.action === defaultAction(old)
    return {
      ...draft,
      action: untouched ? defaultAction(item) : draft.action,
      text: edited ? draft.text : item.proposal,
      updated: true,
      reviewed: false,
    }
  })
}

export function itemStatus(item: BatchItem, draft: ItemDraft): ItemStatus {
  if (draft.action === 'skip') return 'skip'
  if (draft.action === 'revise') return 'revise'
  return item.editable && draft.text !== item.proposal ? 'edited' : 'approve'
}

const update = (drafts: ItemDraft[], index: number, patch: Partial<ItemDraft>): ItemDraft[] =>
  drafts.map((d, i) => (i === index ? { ...d, ...patch } : d))

/** 按決定鈕：同一個「不處理／請 AI 改」再按一次回到採用；切換不清掉文字與指示。 */
export function chooseAction(drafts: ItemDraft[], index: number, action: ItemAction): ItemDraft[] {
  const current = drafts[index]
  if (!current) return drafts
  const next: ItemAction = current.action === action && action !== 'approve' ? 'approve' : action
  return update(drafts, index, { action: next, ...(current.reviewed !== undefined ? {reviewed: false} : {}) })
}

/** 改草稿：在「不處理」的項目上動手改＝要回這一項，改成採用。 */
export function editText(drafts: ItemDraft[], index: number, text: string): ItemDraft[] {
  const current = drafts[index]
  if (!current) return drafts
  return update(drafts, index, { text, ...(current.reviewed !== undefined ? {reviewed: false} : {}), action: current.action === 'skip' ? 'approve' : current.action })
}

export function editNote(drafts: ItemDraft[], index: number, note: string): ItemDraft[] {
  return drafts[index] ? update(drafts, index, { note }) : drafts
}

/** 還原 AI 版本。 */
export function revertText(items: BatchItem[], drafts: ItemDraft[], index: number): ItemDraft[] {
  const item = items[index]
  return item ? update(drafts, index, { text: item.proposal, ...(drafts[index]?.reviewed !== undefined ? {reviewed:false} : {}) }) : drafts
}

/** 全部採用：不處理的維持不處理，其他都採用（保留改過的文字），清掉請 AI 改的指示。 */
export function approveAll(drafts: ItemDraft[]): ItemDraft[] {
  return drafts.map((d) => ({ ...d, action: d.action === 'skip' ? 'skip' : 'approve', note: '' }))
}

export interface BatchCounts {
  /** 採用（含你改過）。 */
  approve: number
  edited: number
  skip: number
  revise: number
}

export function batchCounts(items: BatchItem[], drafts: ItemDraft[]): BatchCounts {
  const counts: BatchCounts = { approve: 0, edited: 0, skip: 0, revise: 0 }
  items.forEach((item, i) => {
    const status = itemStatus(item, drafts[i])
    if (status === 'skip') counts.skip++
    else if (status === 'revise') counts.revise++
    else {
      counts.approve++
      if (status === 'edited') counts.edited++
    }
  })
  return counts
}

const approveOnly = (items: BatchItem[]) => items.every((item) => !item.editable)

/** 總結列：「採用 N 項 · 其中 M 項你改過 · 不處理 K · 請 AI 改 L」；只核可的題組寫「照建議」。 */
export function batchSummary(items: BatchItem[], counts: BatchCounts): string {
  const parts = [`${approveOnly(items) ? '照建議' : '採用'} ${counts.approve} 項`]
  if (counts.edited) parts.push(`其中 ${counts.edited} 項你改過`)
  if (counts.skip) parts.push(`不處理 ${counts.skip}`)
  if (counts.revise) parts.push(`請 AI 改 ${counts.revise}`)
  return parts.join(' · ')
}

/** 「全部採用」鈕的字：全是草稿才叫「AI 草稿」，有只核可的項目就叫「AI 建議」。 */
export function approveAllLabel(items: BatchItem[]): string {
  return items.every((item) => item.editable) ? '全部採用 AI 草稿' : '全部照 AI 建議'
}

/** 送出鍵停用的原因（第一個問題）；可以送出時回 null。 */
export function sendBlocker(items: BatchItem[], drafts: ItemDraft[]): string | null {
  for (let i = 0; i < items.length; i++) {
    const item = items[i]
    const d = drafts[i]
    if (!d) continue
    const n = i + 1
    if (d.action === 'revise') {
      if (d.note.trim() === '') return `第 ${n} 項選了「請 AI 改」，要寫一句怎麼改`
      if ([...d.note.trim()].length > MAX_NOTE) return `第 ${n} 項的指示超過 2,000 字`
    }
    if (d.action === 'approve' && item.editable) {
      // 可編輯項目採用時最終文字不能是空的（2026-10-09 作者；server 拒收時同一句）。
      if (d.text.trim() === '') return `第 ${n} 項沒有內容，先寫好再採用`
      if ([...d.text].length > MAX_TEXT) return `第 ${n} 項的草稿超過 5,000 字`
    }
  }
  return null
}

/** 作答當下看到的草稿版本（草稿題才有）。 */
const seenVersion = (q: Question) => (q.draft ? { revision: q.draft.revision, content_hash: q.draft.content_hash } : {})
const hashOf = (item: BatchItem) => (item.content_hash ? { content_hash: item.content_hash } : {})

/**
 * 送出載荷（所見即所送）：每項帶決定與面板顯示時的雜湊；採用的可編輯項目帶最後文字（原樣，不修剪）；
 * 只核可的項目不帶文字；請 AI 改帶指示。
 */
export function buildBatchPayload(q: Question, drafts: ItemDraft[], instruction?: string): AnswerPayload {
  const items = q.items ?? []
  const overall = instruction?.trim()
  return {
    question_id: q.id,
    type: 'batch',
    items: items.map((item, i): ItemDecision => {
      const d = drafts[i] ?? { action: 'approve', text: item.proposal, note: '' }
      if (d.action === 'approve') return { item_id: item.id, action: 'approve', ...(item.editable ? { text: d.text } : {}), ...hashOf(item) }
      if (d.action === 'revise') return { item_id: item.id, action: 'revise', note: d.note.trim(), ...hashOf(item) }
      return { item_id: item.id, action: 'skip', ...hashOf(item) }
    }),
    ...(overall ? { instruction: overall } : {}),
    ...seenVersion(q),
  }
}

/** 全部退回的整體指示為什麼不能送（超過 2,000 字）；可以送或還沒寫時回 null（AC-BR-04）。 */
export function redoAllBlocker(instruction: string): string | null {
  const length = [...instruction.trim()].length
  if (length <= MAX_NOTE) return null
  return `整體指示超過 2,000 字（${length.toLocaleString('en-US')} 字），先縮短再送出`
}

/** 全部退回：每項都是請 AI 改、附同一段指示，一次送出；沒寫指示或超過 2,000 字不能送。 */
export function redoAllPayload(q: Question, instruction: string): AnswerPayload | null {
  const note = instruction.trim()
  if (note === '' || redoAllBlocker(note) !== null) return null
  return {
    question_id: q.id,
    type: 'batch',
    items: (q.items ?? []).map((item) => ({ item_id: item.id, action: 'revise', note, ...hashOf(item) })),
    instruction: note,
    ...seenVersion(q),
  }
}

export type BatchKey =
  | { type: 'move'; delta: 1 | -1 }
  | { type: 'edit' }
  | { type: 'decide'; action: ItemAction }
  | { type: 'submit' }
  | { type: 'blur' }
  | { type: 'next' }

/**
 * 鍵盤（AC-BR-07）：J／K、↑／↓ 換項目；E 進入草稿；A／X／R 決定；⌘Enter 送出（輸入框內也可以）；
 * Esc 在輸入框內＝離開輸入框（輸入區標 data-escape="local"）；不在輸入框時的 Esc 由面板統一處理成稍後再問（這裡不處理，
 * 避免同一個 Esc 稍後再問兩次）。單按 Enter 不送出；輸入法組字中一律不處理。回執裡的 Enter 依焦點（`focus`，同決策欄的
 * `focusKind`）：焦點在連結或其他按鈕上交給瀏覽器啟動那個控制項，不跳題（AC-BR-08、AC-DS-07）。
 */
export function batchKey(input: {
  key: string
  meta: boolean
  ctrl: boolean
  composing: boolean
  inText: boolean
  answered: boolean
  focus?: FocusKind
}): BatchKey | null {
  if (input.composing) return null
  if (input.key === 'Escape') return input.inText ? { type: 'blur' } : null
  if (input.answered) return input.key === 'Enter' && !input.inText && input.focus !== 'control' ? { type: 'next' } : null
  if (input.key === 'Enter') return input.meta || input.ctrl ? { type: 'submit' } : null
  if (input.inText || input.meta || input.ctrl) return null
  switch (input.key.toLowerCase()) {
    case 'j':
    case 'arrowdown':
      return { type: 'move', delta: 1 }
    case 'k':
    case 'arrowup':
      return { type: 'move', delta: -1 }
    case 'e':
      return { type: 'edit' }
    case 'a':
      return { type: 'decide', action: 'approve' }
    case 'x':
      return { type: 'decide', action: 'skip' }
    case 'r':
      return { type: 'decide', action: 'revise' }
    default:
      return null
  }
}

/** 卡片的一行摘要：原文第一段的純文字（拿掉 Markdown 記號與連結網址）。 */
export function excerpt(md: string, max = 80): string {
  const first = md
    .replace(/\r/g, '')
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line !== '')
  if (!first) return ''
  const plain = first
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/<[^>]+>/g, '')
    .replace(/^(?:#{1,6}\s+|>\s*|[-*+]\s+|\d+[.)]\s+)+/, '')
    .replace(/[*_`~]/g, '')
    .replace(/\s+/g, ' ')
    .trim()
  const chars = [...plain]
  return chars.length > max ? chars.slice(0, max - 1).join('') + '…' : plain
}

/** 第幾輪：題目 id 結尾 `-rN`（N≥2）；其他是第 1 輪。 */
export function roundOf(id: string): number {
  const m = id.match(/^(.+)-r(\d+)$/)
  const n = m ? Number(m[2]) : 1
  return n >= 2 ? n : 1
}

/** 回執裡每項的狀態（從答案判斷）。 */
export function decisionStatus(item: BatchItem | undefined, decision: ItemDecision): ItemStatus {
  if (decision.action === 'skip') return 'skip'
  if (decision.action === 'revise') return 'revise'
  return item?.editable && decision.text !== undefined && decision.text !== item.proposal ? 'edited' : 'approve'
}

/** 回執：AI 接下來會做什麼。 */
export function receiptPlan(q: Question): string[] {
  const decisions = q.answer?.items ?? []
  const next = roundOf(q.id) + 1
  const n = (action: ItemAction) => decisions.filter((d) => d.action === action).length
  const revise = n('revise')
  if (decisions.length > 0 && revise === decisions.length) {
    const instruction = q.answer?.free_text?.trim()
    return [`全部退回，AI 會依你的指示重寫${instruction ? `：「${instruction}」` : ''}`, `改好後會以「第 ${next} 輪」再送來給你看`]
  }
  const lines: string[] = []
  if (n('approve')) lines.push(`AI 會用你確認的文字逐項送出 ${n('approve')} 項，一字不改`)
  if (revise) lines.push(`請 AI 改的 ${revise} 項改好後會以「第 ${next} 輪」再送來`)
  if (n('skip')) lines.push(`不處理的 ${n('skip')} 項會封存，不回覆`)
  return lines
}
