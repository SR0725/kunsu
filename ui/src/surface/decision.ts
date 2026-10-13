// 決策欄的狀態與鍵盤（BDD decision-surface AC-DS-04/05/06/07）。
import type { AnswerPayload } from '../ipc'
import type { Question } from '../types'
import { isTerminal } from '../types'

export interface DecisionDraft {
  selected: string | null
  note: string
}

const isConfirmOnly = (q: Question) => q.kind === 'confirm' && q.options.length === 0
const isTextOnly = (q: Question) => q.kind === 'text' || q.options.length === 0

/** 送出鍵可不可按：選擇題要選項或文字；文字題要文字；確認題（無選項）永遠可按。 */
export function canSend(q: Question, draft: DecisionDraft): boolean {
  if (isConfirmOnly(q)) return true
  const hasText = draft.note.trim() !== ''
  if (isTextOnly(q)) return hasText
  return draft.selected !== null || hasText
}

/** 組答案 payload：選項＋補充 → choice（含 note）；只有文字 → free_text；確認題 → choice confirm。 */
export function buildPayload(q: Question, draft: DecisionDraft, editedParams?: string): AnswerPayload | null {
  const note = draft.note.trim()
  // 所見即所答（AC-EC-05）：帶上作答當下看到的草稿版本與內容雜湊；任一不符 server 會拒收。
  const seen = q.draft ? { revision: q.draft.revision, content_hash: q.draft.content_hash } : {}
  if (isConfirmOnly(q)) {
    return { question_id: q.id, type: 'choice', choice_id: 'confirm', ...(note ? { note } : {}), ...seen }
  }
  if (draft.selected !== null && q.options.some((o) => o.id === draft.selected)) {
    return {
      question_id: q.id,
      type: 'choice',
      choice_id: draft.selected,
      ...(editedParams ? { edited_params: editedParams } : {}),
      ...(note ? { note } : {}),
      ...seen,
    }
  }
  if (note) return { question_id: q.id, type: 'free_text', free_text: note, ...seen }
  return null
}

/** 點選項：同一個再點一次取消選取。 */
export function toggleSelect(draft: DecisionDraft, optionId: string): DecisionDraft {
  return { ...draft, selected: draft.selected === optionId ? null : optionId }
}

/** 選項下方的「你的決定：…」摘要。 */
export function decisionSummary(q: Question, draft: DecisionDraft): string {
  if (isTextOnly(q) && !isConfirmOnly(q)) return draft.note.trim() ? '' : '寫完按 Enter 送出；Shift+Enter 換行'
  const option = q.options.find((o) => o.id === draft.selected)
  const hasNote = draft.note.trim() !== ''
  if (option) return `你的決定：${option.label}${hasNote ? '＋補充說明' : ''}`
  if (hasNote) return '你的決定：自己寫的回答'
  if (isConfirmOnly(q)) return ''
  return `選一個選項（按 1–${Math.min(4, q.options.length)}），或寫下你的做法`
}

export type SurfaceKey =
  | { type: 'select'; index: number }
  | { type: 'submit' }
  | { type: 'next' }

/** 焦點所在的元素：選項（radio）、主要動作、文字輸入、其他按鈕或連結、都不是。 */
export type FocusKind = 'option' | 'submit' | 'text' | 'control' | 'none'

export function focusKind(target: { tag: string; role?: string | null; submit?: boolean } | null): FocusKind {
  if (!target) return 'none'
  if (target.role === 'radio') return 'option'
  if (target.submit) return 'submit'
  if (target.tag === 'TEXTAREA' || target.tag === 'INPUT') return 'text'
  if (['BUTTON', 'A', 'SUMMARY', 'SELECT'].includes(target.tag) || target.role === 'button' || target.role === 'link') return 'control'
  return 'none'
}

/**
 * 決定欄的鍵盤對應（AC-DS-05／07）：輸入框內只有 Enter（非 Shift、非組字）送出；1–4 只在輸入框外選取。
 * 焦點在選項上的 Enter 由我們處理（送出目前的明確選取；瀏覽器預設會把它當點擊而改選取）；焦點在其他按鈕或連結上的
 * Enter 交給瀏覽器啟動那個控制項。Esc 不在這裡：面板從掛載起統一處理（`escapeAction`）。
 */
export function surfaceKey(input: {
  key: string
  shift: boolean
  composing: boolean
  inText: boolean
  answered: boolean
  focus?: FocusKind
}): SurfaceKey | null {
  if (input.key === 'Enter') {
    if (input.composing) return null
    if (input.focus === 'control') return null
    if (input.answered) return { type: 'next' }
    if (input.inText && input.shift) return null
    return { type: 'submit' }
  }
  if (!input.inText && !input.answered && /^[1-4]$/.test(input.key)) {
    return { type: 'select', index: Number(input.key) - 1 }
  }
  return null
}

/**
 * Esc（AC-DS-01、AC-OP-06）：面板從掛載那一刻起就處理，不等張開動畫。`ack` 表示網頁會處理這個 Esc、要通知原生看門狗
 * （稍後再問要等 dismissal 成功才通知，見 `runLater`）；收起中不處理也不通知，留給原生 2 秒後備。輸入法組字中的 Esc
 * 只取消組字；焦點真的在行內區塊（`data-escape="local"`）的輸入框裡（`field`）時只收起編輯。行內區塊裡的按鈕（還原
 * AI 版本、取消、確認退回…）不是輸入框，Esc 照樣是稍後再問（2026-10-09 QA：舊版這些按鈕上 Esc 完全沒反應）。
 */
export function escapeAction(input: {
  stage: 'opening' | 'open' | 'closing'
  composing: boolean
  local: boolean
  field?: boolean
}): {
  later: boolean
  ack: boolean
} {
  if (input.stage === 'closing') return { later: false, ack: false }
  if (input.composing || (input.local && input.field)) return { later: false, ack: true }
  return { later: true, ack: true }
}

/** 焦點是不是在可以打字的地方（textarea、input、contenteditable）。 */
export function isTextEntry(el: { tagName?: string; isContentEditable?: boolean } | null | undefined): boolean {
  if (!el) return false
  return el.tagName === 'TEXTAREA' || el.tagName === 'INPUT' || el.isContentEditable === true
}

export type LaterResult = { ok: true } | { ok: false; message: string }

/**
 * 「稍後再問」這次已記過 dismissal 的題（面板持有、跨重試保存，AC-OP-06）：id → 記完之後的週期編號（讀不到、或 server 回
 * 「已在終態」時是 null）。
 */
export type LaterDone = Map<string, number | null>

/** server 回「題目已在終態，不接受此操作」（`ServerError::Terminal`）：那一題已經不用再問，稍後再問視同成功。 */
export function isTerminalError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  return message.includes('已在終態')
}

function cycleOf(result: unknown): number | null {
  const cycle = (result as { cycle_number?: unknown } | null | undefined)?.cycle_number
  return typeof cycle === 'number' && Number.isFinite(cycle) ? cycle : null
}

/**
 * 稍後再問（AC-OP-06）：逐題 dismissal；任何一題失敗就停下來回傳看得懂的錯誤——不通知原生、不收起，使用者可以再試
 * （Esc 的話原生 2 秒後備照樣完成，不會卡住）。全部成功才通知原生（`ack`），再收起並等收起完成。
 * `done` 記住這次已成功的題：重試時跳過，只做還沒成功的；「已在終態」視同成功（2026-10-09 QA 第 3 輪：舊版重試會把
 * 已成功的目前題目再記一次）。
 */
export async function runLater(input: {
  ids: string[]
  dismiss: (id: string) => Promise<unknown>
  ack?: () => void
  close: () => Promise<void> | void
  done?: LaterDone
}): Promise<LaterResult> {
  for (const id of input.ids) {
    if (input.done?.has(id)) continue
    try {
      const result = await input.dismiss(id)
      input.done?.set(id, cycleOf(result))
    } catch (error) {
      if (isTerminalError(error)) {
        input.done?.set(id, null)
        continue
      }
      const reason = error instanceof Error ? error.message : String(error)
      return { ok: false, message: `稍後再問沒有成功：${reason}。再按一次 Esc 或「稍後再問」重試。` }
    }
  }
  input.ack?.()
  await input.close()
  return { ok: true }
}

/** 稍後再問的範圍（AC-OP-06）：目前這題（還沒結案才算）＋session 裡所有強制題（含中途加入的），去重。 */
export function laterTargets(current: Question | undefined, session: Question[]): string[] {
  const ids = [...(current && !isTerminal(current.state) ? [current.id] : []), ...session.filter((item) => item.state === 'forced').map((item) => item.id)]
  return [...new Set(ids)]
}

/**
 * 記過之後在同一個 session 裡又到期的題（又是強制、週期編號 ≥ 記錄的）從 `done` 拿掉：那是新的一次強制，稍後再問照樣記。
 * 清單還沒更新（讀到的是記之前的資料，週期編號比較小）或還沒到期的照樣跳過。與原生帳本（`DismissLedger`）同一條規則。
 * 回傳新的 Map，不改原本的。
 */
export function forgetDueAgain(done: LaterDone, session: Question[]): LaterDone {
  const next: LaterDone = new Map(done)
  for (const item of session) {
    const after = done.get(item.id)
    if (after === undefined || after === null) continue
    if (item.state === 'forced' && (item.cycle_number ?? 0) >= after) next.delete(item.id)
  }
  return next
}

/** 「保留回執」（AC-OP-07）：面板正顯示的是這次 session 在面板裡送出的那張回執。 */
export function holdsReceipt(q: Question | undefined, answeredHere: ReadonlySet<string>): boolean {
  return q !== undefined && isTerminal(q.state) && answeredHere.has(q.id)
}

/** 被其他通道結案的題就地唯讀時，說明是怎麼結案的（AC-DS-16）。 */
export function closedElsewhereLine(q: Question): string {
  if (q.state === 'cancelled') return q.rejected ? '你在另一個 Kunsu 視窗退回了這題' : '出題的 AI 已取消這題'
  if (q.state === 'superseded') return '出題的 AI 已改出新版本'
  if (q.state === 'expired') return '已過期'
  switch (q.answer?.channel) {
    case 'discord':
      return '已由 Discord 回答'
    case 'cli':
      return '已由終端機（CLI）回答'
    case 'kongko':
      return '已由 Kongko 發佈確認結案'
    case 'auto_policy':
      return '已依預先授權的規則自動處理'
    case 'ui':
      return '已在另一個 Kunsu 視窗回答'
    case 'mobile':
      return '已在手機上回答'
    default:
      return '已結案'
  }
}

/** 使用者用「這題不該問我」推掉的題（server 記 `rejected`）；沒有 `rejected` 的 cancelled 是出題的 AI 取消的。 */
export const rejectedByRay = (q: Question) => q.state === 'cancelled' && Boolean(q.rejected)

/** 「你退回了這題：原因」。 */
export function rejectionLine(q: Question): string {
  const reason = q.rejected?.reason?.trim()
  return reason ? `你退回了這題：${reason}` : '你退回了這題'
}

/**
 * 決策欄一打開時的階段（AC-DS-10、AC-DS-17）：未結案照常作答；已回答（或使用者自己推掉）是回執、執行失敗的是失敗畫面；
 * 過期、被取代、出題的 AI 取消的是唯讀狀態畫面——回看已結案的題一律沒有作答控制。
 */
export function reviewPhase(q: Question): 'deciding' | 'receipt' | 'failed' | 'closed' {
  if (!isTerminal(q.state)) return 'deciding'
  if (q.state === 'answered' || q.state === 'routed') return q.execution_blocked ? 'failed' : 'receipt'
  if (rejectedByRay(q)) return 'receipt'
  return 'closed'
}

export type MoneyAction = 'reconcile' | 'reauthorize'

/**
 * 這張錢題回執可以做什麼（純函數；CONTRACT §9、AC-MA-18／19／20）：一律依伺服器算的 `money_state`——結果不明 → 人工對帳、可重新授權
 * → 重新授權、還在跑或已定案 → 沒有動作。沒有 `money_state`（舊資料、讀取失敗）才退回看執行紀錄開頭。
 */
export function moneyActionsFor(q: Question): MoneyAction | null {
  const a = q.answer
  if (!q.money?.is_money_op || !a) return null
  if (q.money_state) {
    if (q.money_state === 'unknown') return 'reconcile'
    if (q.money_state === 'reauthorizable') return 'reauthorize'
    return null
  }
  const r = a.execution_result?.trimStart() ?? ''
  if (r.startsWith('EXEC_UNKNOWN')) return 'reconcile'
  const executable =
    a.channel === 'ui' && q.gate !== 'red' && q.options.some((o) => o.id === a.choice_id && o.recommended === true)
  if (executable && (r.startsWith('EXEC_BLOCKED') || r.startsWith('RECONCILED_NOT_EXECUTED'))) return 'reauthorize'
  return null
}

/**
 * 執行失敗畫面的標題與說明（F2，CONTRACT §9）：結果不明時標題「結果不明」、請使用者先核對再對帳；可以重新授權才寫「修好原因後可以在這裡
 * 重新授權」；其他（不動錢的題、已定案）只說答案仍有效。
 */
export function failureCopy(q: Question): { label: string; note: string } {
  const action = moneyActionsFor(q)
  if (action === 'reconcile') {
    return { label: '結果不明', note: 'executor 已用掉授權但沒有回報結果，錢可能已經動過，請先到廣告後台核對再對帳。' }
  }
  const base = '你的答案仍然有效並已寫入決策帳本；失敗原因已留痕。'
  return { label: '執行沒有完成', note: action === 'reauthorize' ? `${base}修好原因後可以在這裡重新授權。` : base }
}

/**
 * 作答當下收到執行失敗事件後，依重新讀到的題目決定畫面（F3）：結果不明 → 回執＋對帳鈕（不停在「執行沒有完成」）；錢鏈列還沒跑完 →
 * 再等一下；其他（被擋、讀不到）→ 失敗畫面。
 */
export function phaseAfterExecFailure(q: Question | null | undefined): 'receipt' | 'failed' | 'wait' {
  if (!q) return 'failed'
  const r = q.answer?.execution_result?.trimStart() ?? ''
  if (q.money_state === 'unknown' || r.startsWith('EXEC_UNKNOWN')) return 'receipt'
  if (q.money_state === 'running') return 'wait'
  return 'failed'
}

/** 已結案的題的狀態文字（回看，AC-DS-17）：依實際終態，不把 AI 取消說成使用者退回。 */
export function closedStatus(q: Question): { label: string; line: string } {
  switch (q.state) {
    case 'expired': {
      const auto = q.answer?.channel === 'auto_policy' && q.answer.choice_id ? q.options.find((o) => o.id === q.answer?.choice_id)?.label : undefined
      return { label: '已過期', line: auto ? `沒有回答（逾時依預設：${auto}）` : '沒有回答' }
    }
    case 'cancelled':
      return q.rejected ? { label: '已退回重擬', line: rejectionLine(q) } : { label: 'AI 取消了這題', line: '出題的 AI 已取消這題' }
    case 'superseded':
      return { label: '被新題取代', line: '出題的 AI 已改出新版本' }
    default:
      return { label: '已回答', line: closedElsewhereLine(q) }
  }
}

/** 進度點的無障礙名稱（AC-DS-10、AC-DS-16）：依實際終態，被其他通道結案的寫出原因，不一律寫「已回答」。 */
export function dotLabel(index: number, q: Question, closedElsewhere: boolean): string {
  const n = `第 ${index + 1} 題`
  if (!isTerminal(q.state)) return n
  if (closedElsewhere) return `${n}（已結案：${closedElsewhereLine(q)}）`
  return `${n}（${closedStatus(q).label}）`
}

/** 對話框 Tab 循環（AC-DS-07）：回傳要聚焦的索引；null 表示交給瀏覽器。index=-1 表示焦點不在面板內。 */
export function wrapFocus(count: number, index: number, shift: boolean): number | null {
  if (count <= 0) return null
  if (index < 0) return shift ? count - 1 : 0
  if (!shift && index === count - 1) return 0
  if (shift && index === 0) return count - 1
  return null
}

/** 右下主要動作的文字（AC-DS-04 表，與 CONTRACT §9 一致）。`question` 是實際呈現的題目（十秒可決沒過時已鎖選項）。 */
export function primaryLabel(input: { question: Question; draft: DecisionDraft; sending: boolean; locked: boolean }): string {
  const { question: q, draft, sending, locked } = input
  if (sending) return '送出中…'
  if (isConfirmOnly(q)) return '確認'
  if (isTextOnly(q)) return '送出回答'
  const selected = q.options.find((o) => o.id === draft.selected)
  const isMoney = q.money?.is_money_op === true
  if (selected && isMoney && locked) return '拍板此選項（不執行）'
  if (selected && isMoney && selected.recommended) return '批准並執行'
  return '送出決定'
}

/** 選項組的 Tab 停點（radiogroup roving tabindex）：已選的選項，沒選就是第一個。 */
export function optionTabStop(options: { id: string }[], selected: string | null): number {
  const index = options.findIndex((o) => o.id === selected)
  return index >= 0 ? index : 0
}

/** 選項間的方向鍵移動（循環）；不是移動鍵回 null。 */
export function rovingTarget(current: number, key: string, count: number): number | null {
  if (count <= 0) return null
  switch (key) {
    case 'ArrowDown':
    case 'ArrowRight':
      return (current + 1) % count
    case 'ArrowUp':
    case 'ArrowLeft':
      return (current - 1 + count) % count
    case 'Home':
      return 0
    case 'End':
      return count - 1
    default:
      return null
  }
}

/**
 * 往外開（AC-DS-13）：蓋板在所有視窗之上，打開外部 App 或瀏覽器前先讓開（未答＝稍後再問、已答＝關閉），
 * 而且要等讓開**完成**（dismissal 成功、收起動畫跑完、視窗隱藏）才打開，目標才不會開在蓋板後面看不到；讓開失敗
 * （`stepAside` 回 false）就不打開。主視窗是一般視窗，直接打開。
 */
export function openOutside({
  inOverlay,
  stepAside,
  open,
}: {
  inOverlay: boolean
  stepAside: () => Promise<boolean | void> | boolean | void
  open: (href: string) => void
}): (href: string) => void {
  return (href) => {
    if (!inOverlay) {
      open(href)
      return
    }
    void Promise.resolve(stepAside()).then((stepped) => {
      if (stepped !== false) open(href)
    })
  }
}
