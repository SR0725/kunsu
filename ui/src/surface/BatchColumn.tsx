// 題組（PRD §25、BDD batch-review、DESIGN「Batch review」）：一題放一批同類項目，在同一個面板看完、改完、決定完。
// 左欄跟著選中的項目顯示原文；右欄是一鍵按鈕、項目卡清單、總結列與送出。純邏輯在 ./batch。
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { ReactNode, Ref, RefObject, TextareaHTMLAttributes } from 'react'
import type { AnswerPayload, KunsuIpc } from '../ipc'
import type { BatchItem, ItemAction, Question } from '../types'
import { countdown, defaultDeclaration, fmtHm } from '../format'
import { conversationLink, deliverySettled, handoffFromDelivery, startReceiptPolling } from '../delivery'
import { RichContent } from '../components/RichContent'
import { useOpenLink } from './openLink'
import { ClosedElsewhere } from './DecisionColumn'
import { closedStatus, focusKind, isTextEntry, rejectedByRay, rejectionLine, reviewPhase } from './decision'
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
  rebaseDrafts,
  receiptPlan,
  redoAllBlocker,
  redoAllPayload,
  revertText,
  roundOf,
  sendBlocker,
  STATUS_LABEL,
} from './batch'
import type { ItemDraft } from './batch'
import { BookOpen, Check, Globe, Camera, MessageCircle, MessagesSquare, AtSign } from 'lucide-react'
import { sourceGroups, sourceOf, toggleReviewed, draftStorageKey, serializeDrafts, restoreDrafts } from './assistantDrafts'
import './batch.css'

export interface BatchReviewProps {
  ipc: KunsuIpc
  question: Question
  /** 左欄最上面的「為什麼問你」（由面板提供）。 */
  why: ReactNode
  /** 左欄最下面的題目證據（畫布、內文、證據行；由面板提供）。 */
  evidence: ReactNode
  isLast: boolean
  /** 只有目前顯示的題目接收鍵盤。 */
  active: boolean
  /** 這題在面板外被結案（AI 取消、過期…）：就地唯讀，不給作答（AC-DS-16）。 */
  closedElsewhere?: boolean
  onUpdated: (question: Question) => void
  onNext: () => void
  /** 作答送出中（同決策欄）：蓋板據此不在作答結果回來前收起（AC-DS-09、AC-OP-07(c)）。 */
  onSending?: (sending: boolean) => void
}

type Phase = 'deciding' | 'receipt' | 'closed'

const ACTION_KEY: Record<ItemAction, string> = { approve: 'A', skip: 'X', revise: 'R' }

/** 題組的兩欄：左欄原文、右欄決定（或送出後的回執）。 */
export function BatchReview({ ipc, question, why, evidence, isLast, active, closedElsewhere = false, onUpdated, onNext, onSending }: BatchReviewProps) {
  const [q, setQ] = useState(question)
  useEffect(() => setQ(question), [question])
  const items = useMemo(() => q.items ?? [], [q.items])
  // 一打開就已結案（回看）的題組一律唯讀：已回答／使用者推掉的是回執，過期、被取代、AI 取消的是狀態畫面（AC-DS-17）。
  const [phase, setPhase] = useState<Phase>(() => {
    const initial = reviewPhase(q)
    return initial === 'deciding' || initial === 'closed' ? initial : 'receipt'
  })
  const [drafts, setDrafts] = useState<ItemDraft[]>(() => {
    try { return restoreDrafts(localStorage.getItem(draftStorageKey(question.id)), question.items ?? []) }
    catch { return initDrafts(question.items ?? []) }
  })
  const [storageError, setStorageError] = useState(false)
  useEffect(() => {
    try {
      if (phase === 'deciding' && !closedElsewhere) localStorage.setItem(draftStorageKey(q.id), serializeDrafts(items, drafts))
      else localStorage.removeItem(draftStorageKey(q.id))
      setStorageError(false)
    } catch { setStorageError(true) }
  }, [q.id, items, drafts, phase, closedElsewhere])
  const replyBatch = items.length > 0 && items.every(item => item.editable)
  const groups = useMemo(() => sourceGroups(items), [items])
  // 項目內容換了（面板開著時草稿重新註冊，AC-BR-09）→ 不清掉使用者的修改：AI 草稿沒變的項目原樣保留，變了的標
  // 「AI 更新了這一項」。送出時帶的是新版本的雜湊。
  const signature = items.map((item) => item.content_hash ?? item.id).join('|')
  const [draftBase, setDraftBase] = useState({ signature, items })
  if (draftBase.signature !== signature) {
    setDraftBase({ signature, items })
    setDrafts((previous) => rebaseDrafts(draftBase.items, items, previous))
  }
  const [selected, setSelected] = useState(0)
  const [redoOpen, setRedoOpen] = useState(false)
  const [redoText, setRedoText] = useState('')
  const [rejecting, setRejecting] = useState(false)
  const [rejectReason, setRejectReason] = useState('')
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const heads = useRef<(HTMLButtonElement | null)[]>([])
  const fields = useRef<(HTMLTextAreaElement | null)[]>([])
  const notes = useRef<(HTMLInputElement | null)[]>([])
  const redoRef = useRef<HTMLTextAreaElement>(null)
  const pendingNoteFocus = useRef<number | null>(null)
  const openLink = useOpenLink(ipc)

  const counts = useMemo(() => batchCounts(items, drafts), [items, drafts])
  const blocker = useMemo(() => sendBlocker(items, drafts), [items, drafts])
  const ready = blocker === null && !sending && items.length > 0
  const redoBlocker = redoAllBlocker(redoText)
  const current = items[Math.min(selected, Math.max(0, items.length - 1))]
  // 被其他通道結案（送出中的以送出結果為準，AC-DS-16）。
  const closedView = closedElsewhere && !sending
  const nextRef = useRef<HTMLButtonElement>(null)

  // 開題時焦點落在第一項（鍵盤可直接 J／K、A／X／R）；送出後（回執）或唯讀時落在「下一題／完成」，焦點不掉到 body
  // （AC-DS-07、AC-BR-08）。
  useEffect(() => {
    if (!active) return
    const id = window.setTimeout(() => {
      if (phase !== 'deciding' || closedView) nextRef.current?.focus({ preventScroll: true })
      else heads.current[0]?.focus({ preventScroll: true })
    }, 60)
    return () => window.clearTimeout(id)
  }, [active, phase, closedView])

  // 選「請 AI 改」後把焦點放進指示欄（渲染後才有輸入框）。
  useEffect(() => {
    const index = pendingNoteFocus.current
    if (index === null) return
    pendingNoteFocus.current = null
    reveal(notes.current[index])
  })

  const select = useCallback(
    (index: number, focusHead = false) => {
      const next = Math.max(0, Math.min(items.length - 1, index))
      setSelected(next)
      if (focusHead) {
        const head = heads.current[next]
        head?.focus({ preventScroll: true })
        head?.closest('li')?.scrollIntoView({ block: 'nearest' })
      }
    },
    [items.length],
  )

  const decide = useCallback((index: number, action: ItemAction) => {
    setSelected(index)
    setDrafts((previous) => {
      const next = chooseAction(previous, index, action)
      if (next[index]?.action === 'revise') pendingNoteFocus.current = index
      return next
    })
  }, [])

  // 回執期間追蹤交付狀態與 AI 回饋（沿用決策欄的輪詢；只依題目與階段重啟）。
  const latestQ = useRef(q)
  const latestOnUpdated = useRef(onUpdated)
  useEffect(() => {
    latestQ.current = q
    latestOnUpdated.current = onUpdated
  })
  const questionId = q.id
  useEffect(() => {
    if (phase !== 'receipt' || deliverySettled(latestQ.current)) return
    return startReceiptPolling({
      fetch: () => ipc.getQuestion(questionId),
      onUpdate: (updated) => {
        setQ(updated)
        latestOnUpdated.current(updated)
      },
    })
  }, [phase, questionId, ipc])

  const submit = useCallback(
    async (payload: AnswerPayload) => {
      setSending(true)
      setError(null)
      onSending?.(true)
      try {
        const updated = await ipc.answerQuestion(payload)
        setQ(updated)
        onUpdated(updated)
        setPhase('receipt')
      } catch (err) {
        // 失敗不得誤顯成功：保留每一項的草稿與決定，可重試。
        setError(`沒有送出成功：${err instanceof Error ? err.message : String(err)}。你的修改都還在，可以再送一次。`)
      } finally {
        setSending(false)
        onSending?.(false)
      }
    },
    [ipc, onSending, onUpdated],
  )

  const send = useCallback(() => {
    if (!ready) return
    void submit(buildBatchPayload(q, drafts))
  }, [drafts, q, ready, submit])

  const sendRedoAll = useCallback(() => {
    const payload = redoAllPayload(q, redoText)
    if (!payload) {
      redoRef.current?.focus()
      return
    }
    void submit(payload)
  }, [q, redoText, submit])

  const sendReject = useCallback(() => {
    void submit({ question_id: q.id, type: 'reject', reason: rejectReason.trim() || undefined })
  }, [q.id, rejectReason, submit])

  // 鍵盤（AC-BR-07）：只有目前顯示的題目接收。
  useEffect(() => {
    if (!active) return
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null
      const inText = isTextEntry(target)
      const key = batchKey({
        key: event.key,
        meta: event.metaKey,
        ctrl: event.ctrlKey,
        composing: event.isComposing || event.keyCode === 229,
        inText,
        answered: phase !== 'deciding' || closedView,
        // 回執裡焦點在連結或其他按鈕上，Enter 只啟動那個控制項（同決策欄的 focusKind）。
        focus: focusKind(
          target?.tagName ? { tag: target.tagName, role: target.getAttribute('role'), submit: Boolean(target.dataset?.surfaceSubmit) } : null,
        ),
      })
      if (!key) return
      switch (key.type) {
        case 'next':
          event.preventDefault()
          onNext()
          return
        case 'blur':
          event.preventDefault()
          target?.blur()
          heads.current[selected]?.focus({ preventScroll: true })
          return
        case 'submit':
          event.preventDefault()
          // 在「全部退回」的指示欄裡按 ⌘Enter＝送出全部退回，不是逐項送出。
          if (target?.dataset.batchRedo) sendRedoAll()
          else send()
          return
        case 'move':
          event.preventDefault()
          select(selected + key.delta, true)
          return
        case 'edit': {
          const field = fields.current[selected]
          if (!field) return
          event.preventDefault()
          reveal(field)
          field.setSelectionRange(field.value.length, field.value.length)
          return
        }
        case 'decide':
          event.preventDefault()
          decide(selected, key.action)
          return
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, closedView, decide, onNext, phase, select, selected, send, sendRedoAll])

  const round = roundOf(q.id)

  if (closedView || phase === 'closed') {
    return (
      <>
        <div className="surface-evidence" aria-label="脈絡與證據">
          {why}
          {evidence}
        </div>
        <ClosedElsewhere ipc={ipc} question={q} withWhy={false} isLast={isLast} onNext={onNext} nextRef={nextRef} review={!closedView} />
      </>
    )
  }

  return (
    <>
      <div className="surface-evidence" aria-label="脈絡與證據">
        {why}
        {current && (
          <section className="surface-card batch-original" aria-label="選中項目的原文">
            <div className="surface-sec-label">
              {items.every((item) => !item.editable) ? '申請內容' : '原文'} · 第 {items.indexOf(current) + 1} 項（共 {items.length} 項）
            </div>
            <div className="batch-original__head">
              <b>{current.title}</b>
              {current.subtitle && <span>{current.subtitle}</span>}
            </div>
            <RichContent className="surface-md batch-original__body" onOpenLink={openLink}>
              {current.context_md}
            </RichContent>
            {phase === 'deciding' && <p className="batch-original__hint">跟著右邊選中的項目切換；按 J／K 換項目。</p>}
          </section>
        )}
        {evidence}
      </div>
      {phase === 'receipt' ? (
        <BatchReceipt ipc={ipc} question={q} isLast={isLast} onNext={onNext} onSelect={(i) => select(i)} nextRef={nextRef} />
      ) : (
        <div className="surface-decision batch-decision">
          <div className="surface-ask-label">
            {replyBatch ? '助理已備好回覆' : '要你決定 · 題組'}{round > 1 ? ` · 第 ${round} 輪` : ''}
          </div>
          <h2 className="surface-ask">
            <RichContent inline onOpenLink={openLink}>
              {q.question_line}
            </RichContent>
          </h2>
          <div className="surface-conseq">
            <span>
              <b>{replyBatch ? '發送方式：' : '不決的後果：'}</b>
              {replyBatch ? '你按下全部送出後，助理才會依目前文字發送。確認註記可省略。' : defaultDeclaration(q).replace(/^不答 → /, '')}
            </span>
            {q.deadline_at && (
              <span>
                <b>期限：</b>
                {countdown(q.deadline_at, new Date())}
              </span>
            )}
          </div>

          {replyBatch && <nav className="batch-sources" aria-label="回覆來源分類">
            {groups.map(group => <button type="button" key={group.id} onClick={() => select(group.firstIndex, true)}>
              <SourceIcon id={group.id} /><span>{group.label}</span><b>{group.count}</b>
            </button>)}
          </nav>}
          {storageError && <p role="alert" className="batch-blocker">本機暫存失敗；請保持此面板開啟，你目前的修改仍在。</p>}
          <div className="batch-bulk">
            <button type="button" className="batch-bulk__btn" onClick={() => setDrafts(approveAll)} disabled={sending}>
              {approveAllLabel(items)}
            </button>
            <button
              type="button"
              className="batch-bulk__btn"
              data-ghost="true"
              aria-expanded={redoOpen}
              disabled={sending}
              onClick={() => {
                setRedoOpen((open) => !open)
                window.setTimeout(() => redoRef.current?.focus(), 0)
              }}
            >
              全部退回，請 AI 重寫…
            </button>
            <span className="batch-bulk__hint">{replyBatch ? "看過即可全部送出，不必逐則確認" : "逐項改完再送，或一鍵處理全部"}</span>
          </div>
          {redoOpen && (
            <div className="batch-redo" data-escape="local">
              <label htmlFor={`redo-${q.id}`}>跟 AI 說要怎麼改（套用到所有項目，一次送出）</label>
              <AutoGrowTextarea
                id={`redo-${q.id}`}
                data-batch-redo="true"
                ref={redoRef}
                value={redoText}
                aria-invalid={redoBlocker !== null}
                aria-describedby={redoBlocker ? `redo-${q.id}-error` : undefined}
                placeholder="例如：語氣再正式一點，每封結尾附上課程頁連結，不要用驚嘆號"
                onChange={(event) => setRedoText(event.target.value)}
              />
              {redoBlocker && (
                <p id={`redo-${q.id}-error`} className="batch-blocker" role="status">
                  {redoBlocker}
                </p>
              )}
              <div className="batch-redo__row">
                <button type="button" className="batch-bulk__btn" data-ghost="true" onClick={() => setRedoOpen(false)}>
                  取消
                </button>
                <button
                  type="button"
                  className="batch-bulk__btn"
                  data-primary="true"
                  disabled={sending || redoText.trim() === '' || redoBlocker !== null}
                  onClick={sendRedoAll}
                >
                  全部退回重寫
                </button>
              </div>
            </div>
          )}

          <ol className="batch-list" role="list" aria-label={`${items.length} 個項目`}>
            {items.map((item, i) => (
              <BatchCard
                key={item.id}
                item={item}
                index={i}
                draft={drafts[i]}
                selected={i === selected}
                headRef={(el) => {
                  heads.current[i] = el
                }}
                fieldRef={(el) => {
                  fields.current[i] = el
                }}
                noteRef={(el) => {
                  notes.current[i] = el
                }}
                disabled={sending}
                onReviewed={() => setDrafts(previous => toggleReviewed(previous, i))}
                onSelect={() => select(i)}
                onDecide={(action) => decide(i, action)}
                onText={(text) => setDrafts((previous) => editText(previous, i, text))}
                onNote={(note) => setDrafts((previous) => editNote(previous, i, note))}
                onRevert={() => setDrafts((previous) => revertText(items, previous, i))}
                onOpenLink={openLink}
              />
            ))}
          </ol>

          {error && (
            <div role="alert" className="surface-alert">
              {error}
            </div>
          )}
          {rejecting && (
            <div className="surface-reject" data-escape="local">
              <input
                autoFocus
                value={rejectReason}
                placeholder="為什麼這題不該問你？（可留白）"
                aria-label="退回原因"
                onChange={(event) => setRejectReason(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter' && !event.nativeEvent.isComposing) {
                    event.preventDefault()
                    event.stopPropagation()
                    sendReject()
                  }
                  if (event.key === 'Escape' && !event.nativeEvent.isComposing) setRejecting(false)
                }}
              />
              <button type="button" onClick={sendReject} disabled={sending}>
                確認退回
              </button>
            </div>
          )}

          <footer className="surface-foot batch-foot">
            <button type="button" className="surface-reject-btn" onClick={() => setRejecting((value) => !value)}>
              這題不該問我
            </button>
            <div className="batch-foot__status" aria-live="polite">
              <span className="batch-summary tabular">{batchSummary(items, counts)}</span>
              {blocker && <span className="batch-blocker">{blocker}</span>}
            </div>
            <button
              type="button"
              className="surface-send"
              data-surface-submit="true"
              disabled={!ready}
              onClick={send}
              title={blocker ?? undefined}
              aria-keyshortcuts="Meta+Enter"
            >
              {sending ? '送出中…' : replyBatch ? (counts.approve > 0 ? `全部送出 ${counts.approve} 則` : '送出修改指示') : '送出決定'} <kbd aria-hidden="true">⌘⏎</kbd>
            </button>
          </footer>
        </div>
      )}
    </>
  )
}

function BatchCard({
  item,
  index,
  draft,
  selected,
  headRef,
  fieldRef,
  noteRef,
  onSelect,
  onDecide,
  onText,
  onNote,
  onRevert,
  onOpenLink,
  onReviewed,
  disabled,
}: {
  item: BatchItem
  index: number
  draft: ItemDraft
  selected: boolean
  headRef: (el: HTMLButtonElement | null) => void
  fieldRef: (el: HTMLTextAreaElement | null) => void
  noteRef: (el: HTMLInputElement | null) => void
  onSelect: () => void
  onDecide: (action: ItemAction) => void
  onText: (text: string) => void
  onNote: (note: string) => void
  onRevert: () => void
  onOpenLink: (href: string) => void
  onReviewed: () => void
  disabled: boolean
}) {
  const n = index + 1
  const status = itemStatus(item, draft)
  const edited = item.editable && draft.text !== item.proposal
  // AI 建議不處理、給空草稿的項目（不准佔位文字，2026-10-09 作者）：草稿框提示要先寫好才能採用。
  const emptyDraft = item.editable && item.proposal.trim() === ''
  const actions: { action: ItemAction; label: string }[] = [
    { action: 'approve', label: item.editable ? '採用' : '照建議' },
    { action: 'skip', label: '不處理' },
    { action: 'revise', label: '請 AI 改' },
  ]
  return (
    <li role="listitem" className="batch-card" data-action={draft.action} data-selected={selected ? 'true' : 'false'} onFocus={onSelect}>
      <button type="button" ref={headRef} className="batch-card__head" aria-current={selected ? 'true' : undefined} onClick={onSelect}>
        <span className="batch-card__num tabular">{item.editable ? <SourceIcon id={sourceOf(item).id} /> : n}</span>
        <span className="batch-card__title">{item.title}</span>
        {item.subtitle && <span className="batch-card__sub">{item.subtitle}</span>}
        <span className="batch-status" data-s={status}>
          {item.editable && status === "approve" ? "待送出" : STATUS_LABEL[status]}
        </span>
      </button>
      <div className="batch-card__excerpt">{excerpt(item.context_md)}</div>
      {selected && (
        // 窄螢幕（單欄）時左欄的原文在上方看不到：改在選中的卡片裡顯示（寬螢幕由 CSS 隱藏）。
        <RichContent className="batch-card__context" onOpenLink={onOpenLink}>
          {item.context_md}
        </RichContent>
      )}
      {item.default === 'skip' && item.default_reason && <div className="batch-card__reason">AI 建議不處理：{item.default_reason}</div>}
      {item.editable ? (
        <div className="batch-draft" data-escape="local">
          <AutoGrowTextarea
            ref={fieldRef}
            disabled={disabled}
            value={draft.text}
            aria-label={`第 ${n} 項的草稿`}
            placeholder={emptyDraft ? 'AI 建議不處理；要回覆就先寫好內容再採用' : undefined}
            onChange={(event) => onText(event.target.value)}
          />
          <div className="batch-draft__meta">
            <span>
              {draft.updated && <span className="batch-updated">AI 更新了這一項 · </span>}
              {edited ? (
                <>
                  已修改 ·{' '}
                  <button type="button" className="surface-link" onClick={onRevert}>
                    還原 AI 版本
                  </button>
                </>
              ) : (
                'AI 草稿'
              )}
            </span>
            <span className="tabular">{[...draft.text].length.toLocaleString('en-US')} 字</span>
          </div>
        </div>
      ) : (
        <div className="batch-proposal">
          <span className="batch-proposal__label">{draft.updated ? 'AI 建議 · AI 更新了這一項' : 'AI 建議'}</span>
          {item.proposal}
        </div>
      )}
      <div className="batch-acts" role="group" aria-label={`第 ${n} 項怎麼處理`}>
        {item.editable && <button type="button" className="batch-reviewed" aria-pressed={!!draft.reviewed}
          disabled={disabled} onClick={onReviewed} aria-label={`第 ${n} 項${draft.reviewed ? '撤銷確認' : '確認已看過'}`}>
          <Check size={13} aria-hidden="true" />{draft.reviewed ? '已確認' : '確認'}
        </button>}
        {actions.map(({ action, label }) => (
          <button
            key={action}
            type="button"
            disabled={disabled}
            aria-pressed={draft.action === action}
            aria-keyshortcuts={ACTION_KEY[action]}
            onClick={() => onDecide(action)}
          >
            {label}
            <kbd aria-hidden="true">{ACTION_KEY[action]}</kbd>
          </button>
        ))}
      </div>
      {draft.action === 'revise' && (
        <div className="batch-note" data-escape="local">
          <input
            ref={noteRef}
            disabled={disabled}
            value={draft.note}
            aria-label={`第 ${n} 項要怎麼改`}
            aria-invalid={draft.note.trim() === ''}
            placeholder="跟 AI 說這項要怎麼改（例如：先道歉再說明）"
            onChange={(event) => onNote(event.target.value)}
          />
        </div>
      )}
    </li>
  )
}

function SourceIcon({id}:{id:string}) {
  const Icon = ({course:BookOpen, facebook:Globe, instagram:Camera, threads:AtSign, line:MessageCircle} as Record<string,typeof BookOpen>)[id] ?? MessagesSquare
  return <Icon size={15} aria-hidden="true" />
}

/** 把焦點放進輸入框並捲到看得見的位置（scroll-margin 讓它不被底部的送出列蓋住）。 */
function reveal(el: HTMLElement | null | undefined) {
  if (!el) return
  el.focus({ preventScroll: true })
  el.scrollIntoView({ block: 'nearest' })
}

/** 依內容量出高度（不出現內部捲軸）。 */
function fitHeight(el: HTMLTextAreaElement) {
  el.style.height = 'auto'
  el.style.height = `${el.scrollHeight + 2}px`
}

/**
 * 草稿自動長高（不出現內部捲軸或水平捲動）。值改變與寬度改變（面板或視窗變窄、單欄／兩欄切換）都重算——
 * 超出的部分被藏起來卻照樣送出是不行的（AC-BR-07）。
 */
function AutoGrowTextarea({ ref, value, ...rest }: TextareaHTMLAttributes<HTMLTextAreaElement> & { ref?: Ref<HTMLTextAreaElement>; value: string }) {
  const inner = useRef<HTMLTextAreaElement | null>(null)
  useLayoutEffect(() => {
    const el = inner.current
    if (el) fitHeight(el)
  }, [value])
  useEffect(() => {
    const el = inner.current
    if (!el || typeof ResizeObserver === 'undefined') return
    let width = el.getBoundingClientRect().width
    const observer = new ResizeObserver((entries) => {
      const next = entries[0]?.contentRect.width ?? width
      // 只看寬度：高度是我們自己改的，看高度會循環。
      if (Math.abs(next - width) < 0.5) return
      width = next
      fitHeight(el)
    })
    observer.observe(el)
    return () => observer.disconnect()
  }, [])
  return (
    <textarea
      {...rest}
      value={value}
      rows={2}
      ref={(el) => {
        inner.current = el
        if (typeof ref === 'function') ref(el)
        else if (ref) (ref as RefObject<HTMLTextAreaElement | null>).current = el
      }}
    />
  )
}

/** 回執（AC-BR-08）：每項的決定與總數、AI 接下來會做什麼、交付狀態與 AI 回饋。 */
function BatchReceipt({
  ipc,
  question,
  isLast,
  onNext,
  onSelect,
  nextRef,
}: {
  ipc: KunsuIpc
  question: Question
  isLast: boolean
  onNext: () => void
  onSelect: (index: number) => void
  nextRef?: Ref<HTMLButtonElement>
}) {
  const a = question.answer
  const items = question.items ?? []
  const cancelled = question.state === 'cancelled'
  const handoff = handoffFromDelivery(question) ?? {
    text: cancelled
      ? rejectedByRay(question)
        ? '已退回出題者重擬'
        : '出題的 AI 已取消這題'
      : question.wake_on_answer.mode === 'none'
        ? '答案已寫入決策帳本，等下一輪排程收割'
        : 'AI 接手中…',
    done: cancelled || question.wake_on_answer.mode === 'none',
    pulse: false,
    tone: 'muted' as const,
  }
  const link = conversationLink(question)
  const openLink = useOpenLink(ipc)
  const plan = cancelled ? [] : receiptPlan(question)
  // 「已退回重擬」只用在使用者自己推掉的題組（有 rejected）；出題的 AI 取消的不寫成使用者退回（AC-DS-17）。
  const label = cancelled ? closedStatus(question).label : a && a.channel !== 'ui' ? '已回答 · 題組' : '已送出 · 題組'
  return (
    <div className="surface-decision batch-decision">
      <div className="surface-ask-label">{label}</div>
      {/* 回執的題目和作答時一樣經 Markdown 渲染（AC-DS-09、AC-BR-08）。 */}
      <h2 className="surface-ask">
        <RichContent inline onOpenLink={openLink}>
          {question.question_line}
        </RichContent>
      </h2>
      {rejectedByRay(question) && (
        <div className="surface-final">
          <div className="surface-final__note">{rejectionLine(question)}</div>
        </div>
      )}
      <div className="surface-receipt" aria-live="polite">
        <div className="surface-rc" data-on="true">
          <span className="surface-rc__ic">✓</span>
          <span>
            已收到<small>{a?.answered_at ? fmtHm(a.answered_at) : ''}</small>
          </span>
        </div>
        <div className="surface-rc" data-on="true">
          <span className="surface-rc__ic">✓</span>
          <span>已寫入決策帳本</span>
        </div>
        {plan.map((line) => (
          <div key={line} className="surface-rc" data-on="true">
            <span className="surface-rc__ic">✓</span>
            <span>{line}</span>
          </div>
        ))}
        <div className="surface-rc" data-on="true" data-tone={handoff.tone}>
          <span className="surface-rc__ic">{handoff.done ? '✓' : handoff.tone === 'danger' ? '✕' : '…'}</span>
          <span>
            {handoff.text}
            {link && (
              <small>
                <button type="button" className="surface-link" onClick={() => openLink(link)}>
                  打開原對話（{link.startsWith('codex://') ? 'Codex App' : 'Claude App'}）↗
                </button>
              </small>
            )}
          </span>
        </div>
      </div>
      {a?.items && a.items.length > 0 && (
        <section className="surface-card batch-receipt-list" aria-label="每一項的決定">
          <div className="surface-sec-label">每一項的決定</div>
          <ol role="list">
            {a.items.map((d) => {
              const index = items.findIndex((item) => item.id === d.item_id)
              const item = items[index]
              const status = decisionStatus(item, d)
              return (
                <li key={d.item_id} role="listitem">
                  <button type="button" className="batch-receipt-row" onClick={() => index >= 0 && onSelect(index)}>
                    <span className="batch-card__num tabular">{index + 1}</span>
                    <span className="batch-card__title">{item?.title ?? d.item_id}</span>
                    <span className="batch-status" data-s={status}>
                      {STATUS_LABEL[status]}
                    </span>
                  </button>
                  {d.action === 'revise' && d.note && d.note !== a.free_text && <div className="batch-receipt-note">{d.note}</div>}
                </li>
              )
            })}
          </ol>
        </section>
      )}
      {a?.report && (
        <section className="surface-feedback" aria-label="AI 回饋">
          <div className="surface-feedback__head">AI 回饋 · {a.report.status === 'done' ? '已完成' : a.report.status === 'failed' ? '失敗' : '被擋下'}</div>
          <RichContent onOpenLink={openLink}>{a.report.summary_md}</RichContent>
        </section>
      )}
      <footer className="surface-foot batch-foot">
        <span className="surface-muted">{isLast ? '全部處理完了' : '可以前往下一題'}</span>
        <button ref={nextRef} type="button" className="surface-send" data-surface-submit="true" onClick={onNext}>
          {isLast ? '完成' : '下一題'} <kbd>⏎</kbd>
        </button>
      </footer>
    </div>
  )
}
