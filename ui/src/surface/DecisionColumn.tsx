// 決策欄（BDD decision-surface AC-DS-04/05/06/07/09）：只放這一題的決定。
// 右下角只有主要動作；「這題不該問我」在左下；稍後再問在頁首。
// 錢題保留所見即所簽、改金額紅線重檢、紅燈／升紅只拍板不執行（AC-MA-12）；回執與失敗畫面的錢的後續動作依伺服器算的
// `money_state`（AC-MA-18／19／20）。

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { AnswerPayload, ExecEvent, KunsuIpc } from '../ipc'
import type { Question } from '../types'
import { checkOneClick, lintCard } from '../lint'
import { parseMoneyChange } from '../redline'
import type { RedlineResult } from '../redline'
import { countdown, defaultDeclaration, fmtHm } from '../format'
import { conversationLink, handoffFromDelivery, moneyOutcome, receiptSettled, startReceiptPolling } from '../delivery'
import { MoneyActions } from './MoneyActions'
import { RichContent } from '../components/RichContent'
import {
  buildPayload,
  canSend,
  closedElsewhereLine,
  closedStatus,
  decisionSummary,
  failureCopy,
  focusKind,
  optionTabStop,
  phaseAfterExecFailure,
  primaryLabel,
  rejectedByRay,
  rejectionLine,
  reviewPhase,
  rovingTarget,
  surfaceKey,
  toggleSelect,
} from './decision'
import type { DecisionDraft } from './decision'
import { WhyCard } from './WhyCard'
import { useOpenLink } from './openLink'
import { reducedMotion } from '../motion'

type Phase = 'deciding' | 'receipt' | 'failed' | 'closed'

/** 執行失敗事件後重新讀題：錢鏈列還沒跑完時每 500 ms 再讀，最多 12 次；之後留在回執，由回執輪詢接手（F3）。 */
const FAIL_RECHECK_TRIES = 12
const FAIL_RECHECK_MS = 500
const delay = (ms: number) => new Promise<void>((resolve) => window.setTimeout(resolve, ms))

export interface DecisionColumnProps {
  ipc: KunsuIpc
  question: Question
  /** 單欄版型時把「為什麼問你」放在題目上方。 */
  withWhy: boolean
  isLast: boolean
  /** 只有目前顯示的題目接收鍵盤。 */
  active: boolean
  /** 這題在面板外被結案（Discord、CLI、AI 取消、過期）：就地唯讀，不給作答（AC-DS-16）。 */
  closedElsewhere?: boolean
  onUpdated: (question: Question) => void
  onNext: () => void
  /**
   * 作答送出中（送出到結果回來）：呼叫 answer_question 前同步回報 true，結果寫進 session 後才回報 false。蓋板據此不在作答
   * 結果回來前收起（AC-DS-09、AC-OP-07(c)）。
   */
  onSending?: (sending: boolean) => void
}

export function DecisionColumn({ ipc, question, withWhy, isLast, active, closedElsewhere = false, onUpdated, onNext, onSending }: DecisionColumnProps) {
  const [q, setQ] = useState(question)
  useEffect(() => setQ(question), [question])
  // 一打開就已結案（回看）的題一律唯讀：已回答／使用者推掉的是回執，過期、被取代、AI 取消的是狀態畫面（AC-DS-17）。
  const [phase, setPhase] = useState<Phase>(() => reviewPhase(q))
  const [draft, setDraft] = useState<DecisionDraft>({ selected: null, note: '' })
  const [sending, setSending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [rejecting, setRejecting] = useState(false)
  const [rejectReason, setRejectReason] = useState('')
  const [editing, setEditing] = useState<{ optionId: string; value: string } | null>(null)
  const [redline, setRedline] = useState<RedlineResult | null>(null)
  const [execEvents, setExecEvents] = useState<ExecEvent[]>([])
  const [failReason, setFailReason] = useState<string | null>(q.execution_blocked ?? null)
  const noteRef = useRef<HTMLTextAreaElement>(null)
  const firstOptionRef = useRef<HTMLButtonElement>(null)
  const nextRef = useRef<HTMLButtonElement>(null)
  const unsubRef = useRef<(() => void) | null>(null)
  useEffect(() => () => unsubRef.current?.(), [])
  // 這次作答收到過執行失敗事件：之後的作答結果與回執輪詢不把畫面蓋回「已送出」（F3）。
  const failSeenRef = useRef(false)
  // 題目句、選項標題與說明裡的連結走面板的同一個打開入口（蓋板先讓開，AC-DS-13）。
  const openLink = useOpenLink(ipc)

  const lint = useMemo(() => lintCard(q), [q])
  const execLock = useMemo(() => checkOneClick(q, redline?.breached ?? false), [q, redline])
  const isMoney = q.money?.is_money_op === true
  const isRed = q.gate === 'red'
  const degraded = lint.malformed
  // 十秒可決沒過：鎖選項，只能文字回答、退回或稍後（沿用舊卡語意）。
  const effective = useMemo<Question>(() => (degraded ? { ...q, options: [], kind: 'text' } : q), [degraded, q])
  const ready = canSend(effective, draft) && !sending
  // 確認題（confirm、沒有選項）：主要動作是「確認」，補充說明選填（AC-DS-15）。
  const confirmOnly = effective.kind === 'confirm' && effective.options.length === 0
  const isTextQuestion = !confirmOnly && (effective.kind === 'text' || effective.options.length === 0)
  const focusNote = isTextQuestion || confirmOnly
  // 被其他通道結案（送出中的那一題以送出結果為準）。
  const closedView = closedElsewhere && !sending

  // 開啟時焦點落在第一個選項（文字題落在回答框）；回執或失敗畫面落在「下一題／完成」（送出後焦點不掉到 body，AC-DS-07）。
  useEffect(() => {
    if (!active) return
    const id = window.setTimeout(() => {
      if (phase !== 'deciding' || closedView) nextRef.current?.focus({ preventScroll: true })
      else if (focusNote) noteRef.current?.focus({ preventScroll: true })
      else firstOptionRef.current?.focus({ preventScroll: true })
    }, 60)
    return () => window.clearTimeout(id)
  }, [active, phase, focusNote, closedView])

  // 回執期間追蹤交付狀態與 AI 回饋（回答延續原對話）。只依題目 id 與階段重啟，新資料不重設起算時間（AC-CC-13）。
  const latestQ = useRef(q)
  const latestOnUpdated = useRef(onUpdated)
  useEffect(() => {
    latestQ.current = q
    latestOnUpdated.current = onUpdated
  })
  const questionId = q.id
  // 伺服器回的新題目接回這一欄（錢的後續動作、執行失敗後重新讀題）：按鈕跟著最新的題目（F3）。
  const adopt = useCallback((updated: Question) => {
    setQ(updated)
    latestOnUpdated.current(updated)
  }, [])
  // 錢鏈列還沒跑完（money_state = running）時也輪詢：錢的結果定案後按鈕跟著出現（AC-MA-20）。
  useEffect(() => {
    if (phase !== 'receipt' || receiptSettled(latestQ.current)) return
    return startReceiptPolling({
      fetch: () => ipc.getQuestion(questionId),
      settled: receiptSettled,
      onUpdate: (updated) => {
        adopt(updated)
        // 收到過執行失敗事件、錢鏈列結束後不是結果不明（被擋等）：改走失敗畫面。
        if (failSeenRef.current && updated.execution_blocked && phaseAfterExecFailure(updated) === 'failed') {
          setFailReason(updated.execution_blocked)
          setPhase('failed')
        }
      },
    })
  }, [phase, questionId, ipc, adopt])

  const submit = useCallback(
    async (payload: AnswerPayload) => {
      setSending(true)
      setError(null)
      onSending?.(true)
      if (payload.type === 'choice' && isMoney) {
        setExecEvents([])
        unsubRef.current?.()
        failSeenRef.current = false
        unsubRef.current = ipc.subscribeExecution(q.id, (event) => {
          setExecEvents((previous) => [...previous, event])
          if (event.status !== 'fail') return
          failSeenRef.current = true
          setFailReason(event.line ?? '執行失敗（未附原因）')
          // 重新讀題（F3）：結果不明 → 回執＋對帳鈕；錢鏈列還沒跑完 → 稍後再讀；其他 → 失敗畫面。
          void (async () => {
            for (let attempt = 0; ; attempt++) {
              const updated = await ipc.getQuestion(q.id).catch(() => null)
              const next = phaseAfterExecFailure(updated)
              if (next === 'wait' && attempt < FAIL_RECHECK_TRIES) {
                await delay(FAIL_RECHECK_MS)
                continue
              }
              if (updated) adopt(updated)
              if (next === 'failed') {
                if (updated?.execution_blocked) setFailReason(updated.execution_blocked)
                setPhase('failed')
              } else setPhase('receipt')
              return
            }
          })()
        })
      }
      try {
        const updated = await ipc.answerQuestion(payload)
        setQ(updated)
        onUpdated(updated)
        // 執行失敗事件已先到：畫面交給重新讀題決定，不蓋回「已送出」。
        if (!failSeenRef.current) {
          setPhase(updated.execution_blocked ? 'failed' : 'receipt')
          if (updated.execution_blocked) setFailReason(updated.execution_blocked)
        }
      } catch (err) {
        // 失敗不得誤顯成功：保留草稿，可重試（AC-DS-09）。
        setError(`沒有送出成功：${err instanceof Error ? err.message : String(err)}。草稿已保留，可以再按一次送出。`)
      } finally {
        setSending(false)
        onSending?.(false)
      }
    },
    [adopt, ipc, isMoney, onSending, onUpdated, q.id],
  )

  const send = useCallback(() => {
    if (!ready) return
    const editedParams = q.answer?.edited_params ?? (redline ? (q.params_line ?? undefined) : undefined)
    const payload = buildPayload(effective, draft, editedParams)
    if (payload) void submit(payload)
  }, [draft, effective, q.answer?.edited_params, q.params_line, ready, redline, submit])

  const sendReject = useCallback(() => {
    void submit({ question_id: q.id, type: 'reject', reason: rejectReason.trim() || undefined })
  }, [q.id, rejectReason, submit])

  const applyEdit = useCallback(async () => {
    if (!editing) return
    const value = Number(editing.value.replace(/,/g, ''))
    if (!Number.isFinite(value) || value <= 0) return
    const result = await ipc.editOption(q.id, editing.optionId, value)
    setQ(result.question)
    onUpdated(result.question)
    setRedline(result.redline)
    setEditing(null)
  }, [editing, ipc, onUpdated, q.id])

  // 鍵盤（AC-DS-05／07）：只有目前顯示的題目接收；Esc 由面板統一處理。
  // 焦點在選項上的 Enter 攔下瀏覽器預設（會被當成點擊而改選取），送出目前的明確選取；其他按鈕或連結交給瀏覽器。
  useEffect(() => {
    if (!active) return
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null
      const focus = focusKind(
        target?.tagName ? { tag: target.tagName, role: target.getAttribute('role'), submit: Boolean(target.dataset?.surfaceSubmit) } : null,
      )
      const key = surfaceKey({
        key: event.key,
        shift: event.shiftKey,
        composing: event.isComposing || event.keyCode === 229,
        inText: focus === 'text',
        answered: phase !== 'deciding' || closedView,
        focus,
      })
      if (!key) return
      if (key.type === 'next') {
        event.preventDefault()
        onNext()
        return
      }
      if (key.type === 'submit') {
        event.preventDefault()
        send()
        return
      }
      const option = effective.options[key.index]
      if (option) {
        event.preventDefault()
        setDraft((current) => toggleSelect(current, option.id))
      }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [active, closedView, effective.options, onNext, phase, send])

  if (closedView) {
    return <ClosedElsewhere ipc={ipc} question={q} withWhy={withWhy} isLast={isLast} onNext={onNext} nextRef={nextRef} onUpdated={adopt} />
  }

  if (phase === 'closed') {
    return <ClosedElsewhere ipc={ipc} question={q} withWhy={withWhy} isLast={isLast} onNext={onNext} nextRef={nextRef} onUpdated={adopt} review />
  }

  if (phase === 'failed') {
    // 文字依伺服器的錢的狀態（F2）：結果不明 → 標題「結果不明」、先核對再對帳；可以重新授權才說可以在這裡重新授權。
    const copy = failureCopy(q)
    return (
      <div className="surface-decision">
        {withWhy && <WhyCard ipc={ipc} question={q} />}
        <div className="surface-ask-label">{copy.label}</div>
        <h2 className="surface-ask">
          <RichContent inline onOpenLink={openLink}>
            {q.question_line}
          </RichContent>
        </h2>
        <div role="alert" className="surface-alert">{failReason ?? '執行失敗'}</div>
        <p className="surface-muted">{copy.note}</p>
        <MoneyActions ipc={ipc} question={q} onUpdated={adopt} focusAfter={nextRef} explain={false} />
        <footer className="surface-foot">
          <span />
          <button ref={nextRef} type="button" className="surface-send" data-surface-submit="true" onClick={onNext}>
            {isLast ? '完成' : '下一題'} <kbd>⏎</kbd>
          </button>
        </footer>
      </div>
    )
  }

  if (phase === 'receipt') {
    return (
      <div className="surface-decision">
        {withWhy && <WhyCard ipc={ipc} question={q} />}
        <SurfaceReceipt ipc={ipc} question={q} execEvents={execEvents} isLast={isLast} onNext={onNext} nextRef={nextRef} onAdopt={adopt} />
      </div>
    )
  }

  const consequence = defaultDeclaration(q).replace(/^不答 → /, '')
  const summary = decisionSummary(effective, draft)
  const selectedOption = q.options.find((option) => option.id === draft.selected)
  const primary = primaryLabel({ question: effective, draft, sending, locked: execLock.locked })

  return (
    <div className="surface-decision">
      {withWhy && <WhyCard ipc={ipc} question={q} />}
      <div className="surface-ask-label">{isTextQuestion && !degraded ? '要你回應' : '要你決定'}</div>
      <h2 className="surface-ask">
        <RichContent inline onOpenLink={openLink}>
          {q.question_line}
        </RichContent>
      </h2>
      <div className="surface-conseq">
        <span>
          <b>不決的後果：</b>
          {consequence}
        </span>
        {q.deadline_at && (
          <span>
            <b>期限：</b>
            {countdown(q.deadline_at, new Date())}
          </span>
        )}
      </div>

      {degraded && (
        <div role="alert" className="surface-alert">
          <div className="font-medium">這題沒通過十秒可決檢查，已鎖選項與執行</div>
          <ul>
            {lint.problems.map((problem) => (
              <li key={problem.code + problem.message}>{problem.message}</li>
            ))}
          </ul>
          <div>只能文字回答、退回重擬或稍後再問。</div>
        </div>
      )}

      {(isMoney || q.params_line) && (
        <div className="surface-params" data-breached={redline?.breached ? 'true' : 'false'}>
          <small>所見即所簽</small>
          {q.params_line ?? '（動錢題缺參數行）'}
          {redline?.breached && <div className="surface-params__breach">{redline.reasons.join('；')} → 已升紅並鎖一鍵</div>}
        </div>
      )}

      {!degraded && q.options.length > 0 && (
        <div
          className="surface-opts"
          role="radiogroup"
          aria-label="選項"
          onKeyDown={(event) => {
            const buttons = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="radio"]')]
            const current = buttons.indexOf(document.activeElement as HTMLButtonElement)
            if (current < 0) return
            const target = rovingTarget(current, event.key, buttons.length)
            if (target === null) return
            event.preventDefault()
            event.stopPropagation()
            buttons[target]?.focus()
          }}
        >
          {q.options.map((option, index) => {
            const checked = draft.selected === option.id
            const editable = isMoney && !isRed && parseMoneyChange(option.label) !== null
            const isEditing = editing?.optionId === option.id
            const labelId = `opt-${q.id}-${index}-label`
            const detailId = `opt-${q.id}-${index}-detail`
            return (
              <div key={option.id} className="surface-opt-wrap">
                {/* 可點擊區是一顆鋪滿卡片的 radio；標題與說明疊在上面、在按鈕外，裡面的連結不會被包在按鈕裡（點連結不選取，
                    Tab 走得到，AC-DS-13）。radio 的名稱與描述以 aria-labelledby／aria-describedby 連回標題與說明。 */}
                <div className="surface-opt" data-checked={checked ? 'true' : 'false'}>
                  <button
                    ref={index === 0 ? firstOptionRef : undefined}
                    type="button"
                    role="radio"
                    aria-checked={checked}
                    aria-labelledby={labelId}
                    aria-describedby={option.detail ? detailId : undefined}
                    tabIndex={index === optionTabStop(q.options, draft.selected) ? 0 : -1}
                    className="surface-opt__hit"
                    onClick={() => setDraft((current) => toggleSelect(current, option.id))}
                  />
                  <span className="surface-opt__key tabular" aria-hidden="true">
                    {index + 1}
                  </span>
                  <span id={labelId} className="surface-opt__label">
                    <RichContent inline onOpenLink={openLink}>
                      {option.label}
                    </RichContent>
                    {option.recommended && <span className="surface-opt__reco">AI 建議</span>}
                  </span>
                  {option.detail && (
                    <span id={detailId} className="surface-opt__detail">
                      <RichContent inline onOpenLink={openLink}>
                        {option.detail}
                      </RichContent>
                    </span>
                  )}
                </div>
                {editable && !isEditing && (
                  <button
                    type="button"
                    className="surface-opt__edit"
                    onClick={() => setEditing({ optionId: option.id, value: String(parseMoneyChange(option.label)?.to ?? '') })}
                  >
                    改金額
                  </button>
                )}
                {isEditing && (
                  <span className="surface-opt__editor" data-escape="local">
                    <span>{option.label.split('→')[0]}→</span>
                    <input
                      autoFocus
                      inputMode="numeric"
                      aria-label="新的金額"
                      value={editing.value}
                      onChange={(event) => setEditing({ ...editing, value: event.target.value })}
                      onKeyDown={(event) => {
                        if (event.key === 'Enter') {
                          event.preventDefault()
                          event.stopPropagation()
                          void applyEdit()
                        }
                        if (event.key === 'Escape') {
                          event.preventDefault()
                          event.stopPropagation()
                          setEditing(null)
                        }
                      }}
                    />
                    <button type="button" onClick={() => void applyEdit()}>
                      套用
                    </button>
                  </span>
                )}
              </div>
            )
          })}
        </div>
      )}

      <div className="surface-note" data-primary={isTextQuestion ? 'true' : 'false'}>
        <label htmlFor={`note-${q.id}`}>{isTextQuestion ? '你的回答' : '補充說明（選填）'}</label>
        <textarea
          id={`note-${q.id}`}
          ref={noteRef}
          value={draft.note}
          placeholder={isTextQuestion ? '直接寫下你的回答…' : '例如：照選項做，但預算先減半'}
          onChange={(event) => setDraft((current) => ({ ...current, note: event.target.value }))}
        />
      </div>
      <div className="surface-summary" aria-live="polite">
        {summary}
      </div>
      {execLock.locked && execLock.reason && selectedOption && (
        <div className="surface-lock">{execLock.reason}——拍板只寫入決策帳本，不觸發執行。</div>
      )}
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
              if (event.key === 'Enter') {
                event.preventDefault()
                event.stopPropagation()
                sendReject()
              }
              if (event.key === 'Escape') {
                event.preventDefault()
                event.stopPropagation()
                setRejecting(false)
              }
            }}
          />
          <button type="button" onClick={sendReject} disabled={sending}>
            確認退回
          </button>
        </div>
      )}

      <footer className="surface-foot">
        <button type="button" className="surface-reject-btn" onClick={() => setRejecting((value) => !value)}>
          這題不該問我
        </button>
        <button
          type="button"
          className="surface-send"
          data-surface-submit="true"
          disabled={!ready}
          onClick={send}
          title={execLock.locked && selectedOption ? execLock.reason : undefined}
        >
          {primary} <kbd>⏎</kbd>
        </button>
      </footer>
    </div>
  )
}

/** 回執就地顯示（AC-DS-09）：決定、已收到、已寫入帳本、交付狀態（回答延續原對話）、AI 回饋。 */
function SurfaceReceipt({
  ipc,
  question,
  execEvents,
  isLast,
  onNext,
  nextRef,
  onAdopt,
}: {
  ipc: KunsuIpc
  question: Question
  execEvents: ExecEvent[]
  isLast: boolean
  onNext: () => void
  nextRef: React.RefObject<HTMLButtonElement | null>
  /** 錢的後續動作送出後的新題目。 */
  onAdopt: (question: Question) => void
}) {
  const a = question.answer
  // 確認題（confirm、沒有選項）的答案是 `confirm`，回執寫「確認」（AC-DS-15）。
  const confirmed = a?.choice_id === 'confirm' && question.kind === 'confirm' && question.options.length === 0
  const choiceLabel = question.options.find((item) => item.id === a?.choice_id)?.label ?? (confirmed ? '確認' : undefined)
  const handoff = handoffFromDelivery(question) ?? legacyHandoff(question)
  const money = moneyOutcome(question)
  const link = conversationLink(question)
  const openLink = useOpenLink(ipc)
  const [shown, setShown] = useState(0)
  useEffect(() => {
    if (reducedMotion()) {
      setShown(3)
      return
    }
    const timers = [0, 1, 2].map((i) => window.setTimeout(() => setShown(i + 1), 160 + i * 260))
    return () => timers.forEach(window.clearTimeout)
  }, [])
  const cancelled = question.state === 'cancelled'
  // 「已退回重擬」只用在使用者自己推掉的題（有 rejected）；出題的 AI 取消的不寫成使用者退回（AC-DS-17）。回看別的通道答的題寫
  // 「已回答」。
  const label = cancelled ? closedStatus(question).label : a && a.channel !== 'ui' ? '已回答' : '已送出'
  return (
    <>
      <div className="surface-ask-label">{label}</div>
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
      {!cancelled && (
        <div className="surface-final">
          {choiceLabel && (
            <b>
              <RichContent inline onOpenLink={openLink}>
                {choiceLabel}
              </RichContent>
            </b>
          )}
          {a?.free_text && <div className="surface-final__note">{a.free_text}</div>}
          {!choiceLabel && !a?.free_text && <b>已作答</b>}
        </div>
      )}
      <div className="surface-receipt" aria-live="polite">
        <div className="surface-rc" data-on={shown >= 1}>
          <span className="surface-rc__ic">✓</span>
          <span>
            已收到<small>{a?.answered_at ? fmtHm(a.answered_at) : ''}</small>
          </span>
        </div>
        <div className="surface-rc" data-on={shown >= 2}>
          <span className="surface-rc__ic">✓</span>
          <span>已寫入決策帳本</span>
        </div>
        {money && (
          <div className="surface-rc" data-on={shown >= 2} data-tone={money.tone}>
            <span className="surface-rc__ic">{money.icon}</span>
            <span>{money.text}</span>
          </div>
        )}
        <div className="surface-rc" data-on={shown >= 3} data-tone={handoff.tone}>
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
        {execEvents.filter((event) => event.line).length > 0 && (
          <div className="surface-exec">
            {execEvents
              .filter((event) => event.line)
              .map((event, index) => (
                <div key={index} data-fail={event.status === 'fail'}>
                  {event.line}
                </div>
              ))}
          </div>
        )}
      </div>
      <MoneyActions ipc={ipc} question={question} onUpdated={onAdopt} focusAfter={nextRef} />
      {a?.report && (
        <section className="surface-feedback" aria-label="AI 回饋">
          <div className="surface-feedback__head">AI 回饋 · {a.report.status === 'done' ? '已完成' : a.report.status === 'failed' ? '失敗' : '被擋下'}</div>
          <RichContent onOpenLink={openLink}>{a.report.summary_md}</RichContent>
        </section>
      )}
      <footer className="surface-foot">
        <span className="surface-muted">{isLast ? '全部處理完了' : '可以前往下一題'}</span>
        <button ref={nextRef} type="button" className="surface-send" data-surface-submit="true" onClick={onNext}>
          {isLast ? '完成' : '下一題'} <kbd>⏎</kbd>
        </button>
      </footer>
    </>
  )
}

/**
 * 被其他通道結案的題（AC-DS-16）：就地唯讀——說明怎麼結案的，沒有選項、輸入框與送出，只有「下一題／完成」。
 * 一般題與題組共用。
 */
export function ClosedElsewhere({
  ipc,
  question,
  withWhy,
  isLast,
  onNext,
  nextRef,
  review = false,
  onUpdated,
}: {
  ipc: KunsuIpc
  question: Question
  withWhy: boolean
  isLast: boolean
  onNext: () => void
  nextRef?: React.RefObject<HTMLButtonElement | null>
  /** 回看錢題時的對帳／重新授權送出後的新題目。 */
  onUpdated?: (question: Question) => void
  /** 一打開就已結案的題（歷史紀錄回看，AC-DS-17）：狀態依實際終態（已過期、AI 取消了這題、被新題取代…）。 */
  review?: boolean
}) {
  const at = question.answer?.answered_at ?? question.rejected?.at
  const status = review ? closedStatus(question) : { label: '已在其他地方結案', line: closedElsewhereLine(question) }
  const openLink = useOpenLink(ipc)
  return (
    <div className="surface-decision">
      {withWhy && <WhyCard ipc={ipc} question={question} />}
      <div className="surface-ask-label">{status.label}</div>
      <h2 className="surface-ask">
        <RichContent inline onOpenLink={openLink}>
          {question.question_line}
        </RichContent>
      </h2>
      <div className="surface-receipt" aria-live="polite">
        <div className="surface-rc" data-on="true" data-tone="muted">
          <span className="surface-rc__ic">{review ? '·' : '✓'}</span>
          <span>
            {status.line}
            {at && <small>{fmtHm(at)}</small>}
          </span>
        </div>
      </div>
      {!review && <p className="surface-muted surface-closed__note">這一題不用再回答，你在這裡還沒送出的內容不會送出。</p>}
      {/* 回看錢題（歷史紀錄）時也能人工對帳或重新授權（AC-MA-18／19）。 */}
      {review && <MoneyActions ipc={ipc} question={question} onUpdated={onUpdated} focusAfter={nextRef} />}
      <footer className="surface-foot">
        <span className="surface-muted">{isLast ? '全部處理完了' : '可以前往下一題'}</span>
        <button ref={nextRef} type="button" className="surface-send" data-surface-submit="true" onClick={onNext}>
          {isLast ? '完成' : '下一題'} <kbd>⏎</kbd>
        </button>
      </footer>
    </div>
  )
}

function legacyHandoff(question: Question): { text: string; done: boolean; tone: 'ok' | 'muted' | 'danger' } {
  if (question.state === 'cancelled') {
    return rejectedByRay(question) ? { text: '已退回出題者重擬', done: true, tone: 'ok' } : { text: '出題的 AI 已取消這題', done: true, tone: 'muted' }
  }
  if (question.answer?.executed_at) return { text: 'AI 已接手處理完成', done: true, tone: 'ok' }
  if (question.wake_on_answer.mode === 'none') return { text: '答案已寫入決策帳本，等下一輪排程收割', done: true, tone: 'muted' }
  return { text: 'AI 接手中…', done: false, tone: 'muted' }
}
