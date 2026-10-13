// 決策面板（PRD §23、BDD decision-surface）：瀏海張開成面板；左欄脈絡與證據、右欄只放這一題的決定。
// 取代舊的左側粒子 actor＋墨水轉場。題目在本次 flow 內形成穩定 snapshot（append-only，不覆寫答案）。

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { CSSProperties } from 'react'
import type { KunsuIpc, OverlayTransition } from '../ipc'
import type { Question } from '../types'
import { isTerminal } from '../types'
import { fmtHm } from '../format'
import { AgentMarkIcon } from '../components/AgentMarkIcon'
import { RichContent } from '../components/RichContent'
import { mergeQuestionSession } from '../components/overlaySession'
import { CLOSE_FALLBACK_MS, closeProgress, morphFrame, notchFromOrigin, OPEN_FALLBACK_MS, openProgress, panelRadius } from './geometry'
import type { MorphFrame, NotchShape } from './geometry'
import { evidenceMarkdown, hasEvidence } from './content'
import { DecisionColumn } from './DecisionColumn'
import { BatchReview } from './BatchColumn'
import { WhyCard } from './WhyCard'
import { DraftAssets } from './DraftAssets'
import { dotLabel, escapeAction, forgetDueAgain, holdsReceipt, isTextEntry, laterTargets, openOutside, runLater, wrapFocus } from './decision'
import type { LaterDone } from './decision'
import { reducedMotion } from '../motion'
import { OpenLinkContext, useOpenLink } from './openLink'
import { CanvasFrame } from '../canvas/CanvasFrame'
import { useEvidence } from '../canvas/useEvidence'
import { DEFAULT_CYCLE_MIN, laterHint, readCycleMinutes } from './laterHint'

export interface DecisionSurfaceProps {
  ipc: KunsuIpc
  questions: Question[]
  startIndex?: number
  /** 原生殼送來的轉場（瀏海圖示中心）；沒有時（主視窗內）直接淡入。 */
  transition?: OverlayTransition | null
  /**
   * 最新的完整清單（含終態，蓋板傳入）：更新 session 裡的題（被其他通道結案的就地唯讀）、接上 session 開著時才變成
   * 強制的題（PRD §5.8）。沒給就用 `questions`。
   */
  latest?: Question[]
  /** 收起完成後呼叫；回傳 Promise 時（蓋板要等視窗隱藏）往外開會等它完成才打開目標（AC-DS-13）。 */
  onExit: () => void | Promise<unknown>
  /** 面板當下是否正顯示這次 session 在面板裡送出的回執：蓋板據此保留回執直到使用者收起（AC-OP-07、AC-DS-09）。 */
  onHoldingReceipt?: (holding: boolean) => void
  /**
   * 面板有作答正在送出（送出到結果回來）：蓋板據此不在作答結果回來前收起（AC-DS-09、AC-OP-07(c)）。決策欄與題組在呼叫
   * answer_question 前同步回報，結果寫進 session 後才回報結束。
   */
  onAnswering?: (answering: boolean) => void
  /**
   * 從歷史紀錄回看已結案的題（AC-DS-17）：頁首沒有「稍後再問」（沒有東西可以再問），右上改成「關閉 Esc」。
   * Esc 與「關閉」走同一條收起路徑（已結案的題不記 dismissal）。
   */
  review?: boolean
}

type Stage = 'opening' | 'open' | 'closing'

/** 減少動態時的淡入淡出長度（DESIGN「Reduced motion」）。 */
const FADE_MS = 160

/** 面板裡可以用 Tab 停留的元素（roving tabindex 的 -1 不算）。 */
const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]'

function focusables(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.tabIndex >= 0 && el.getClientRects().length > 0)
}

function screenSize() {
  return { w: window.innerWidth, h: window.innerHeight }
}

export function DecisionSurface({ ipc, questions, startIndex = 0, transition, latest, onExit, onHoldingReceipt, onAnswering, review = false }: DecisionSurfaceProps) {
  const [session, setSession] = useState<Question[]>(questions)
  const [index, setIndex] = useState(Math.min(startIndex, Math.max(0, questions.length - 1)))
  const [fade, setFade] = useState(false)
  // 切題後新內容掛上時淡入（AC-DS-10）；第一次張開不帶（張開有自己的依序淡入）。
  const [entering, setEntering] = useState(false)
  // 這次 session 在面板裡結案的題（送出或退回）；以及被其他通道結案的題（AC-DS-16）。
  const answeredHere = useRef(new Set<string>())
  const [closedElsewhere, setClosedElsewhere] = useState<ReadonlySet<string>>(() => new Set())
  const sessionRef = useRef(session)
  sessionRef.current = session
  const feed = latest ?? questions
  useEffect(() => {
    const current = sessionRef.current
    const merged = mergeQuestionSession(current, feed)
    const closed = merged
      .filter((item) => {
        const before = current.find((c) => c.id === item.id)
        return before !== undefined && !isTerminal(before.state) && isTerminal(item.state) && !answeredHere.current.has(item.id)
      })
      .map((item) => item.id)
    setSession(merged)
    if (closed.length > 0) setClosedElsewhere((previous) => new Set([...previous, ...closed]))
  }, [feed])
  const q = session[index]
  useEffect(() => {
    if (!q && session.length > 0) setIndex(session.length - 1)
  }, [q, session.length])

  // 「N 分鐘後再問」：掛載時讀目前生效的週期長度（AC-DS-04）；讀到之前與讀不到時是 15。
  const [cycleMin, setCycleMin] = useState(DEFAULT_CYCLE_MIN)
  useEffect(() => {
    let alive = true
    void readCycleMinutes(ipc).then((minutes) => alive && setCycleMin(minutes))
    return () => {
      alive = false
    }
  }, [ipc])

  // ---- 張開／收起（AC-DS-01）------------------------------------------------
  const reduced = useMemo(reducedMotion, [])
  const animated = Boolean(transition) && !reduced
  const inOverlay = Boolean(transition)
  const [size, setSize] = useState(screenSize)
  useEffect(() => {
    const onResize = () => setSize(screenSize())
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [])
  const notch: NotchShape = useMemo(
    () => notchFromOrigin({ x: transition?.origin_x ?? 16, y: transition?.origin_y ?? 120 }, size),
    [transition?.origin_x, transition?.origin_y, size],
  )
  const [stage, setStage] = useState<Stage>(animated ? 'opening' : 'open')
  const [progress, setProgress] = useState(() => (animated ? { pA: 0, pB: 0 } : { pA: 1, pB: 1 }))
  const [contentVisible, setContentVisible] = useState(!animated)
  const exitRef = useRef(onExit)
  exitRef.current = onExit
  const stageRef = useRef(stage)
  stageRef.current = stage
  const progressRef = useRef(progress)
  progressRef.current = progress
  const closeFrom = useRef({ pA: 1, pB: 1 })
  const exited = useRef(false)
  // 等收起完成（含蓋板視窗隱藏）的人：往外開要等這一刻才打開目標（AC-DS-13）。
  const exitWaiters = useRef<(() => void)[]>([])
  const exitOnce = useCallback(() => {
    if (exited.current) return
    exited.current = true
    let done: unknown
    try {
      done = exitRef.current()
    } catch (error) {
      console.error('收起蓋板失敗', error)
    }
    void Promise.resolve(done)
      .catch((error) => console.error('收起蓋板失敗', error))
      .finally(() => exitWaiters.current.splice(0).forEach((resolve) => resolve()))
  }, [])

  useEffect(() => {
    if (stage === 'open') return
    if (!animated) {
      // 減少動態（或主視窗）：CSS 做 160 ms 淡出，時間到就離開。
      if (stage !== 'closing') return
      const timer = window.setTimeout(exitOnce, FADE_MS)
      return () => window.clearTimeout(timer)
    }
    let raf = 0
    const t0 = performance.now()
    const settle = () => {
      setProgress({ pA: 1, pB: 1 })
      setContentVisible(true)
      setStage('open')
    }
    const step = (now: number) => {
      const t = (now - t0) / 1000
      if (stage === 'opening') {
        const p = openProgress(t)
        setProgress({ pA: p.pA, pB: p.pB })
        if (p.contentVisible) setContentVisible(true)
        if (p.pA > 0.999 && p.pB > 0.999) {
          settle()
          return
        }
      } else {
        // 收起從當下的形狀倒放：內容先淡出，形狀往內收、再沿邊縮回瀏海。
        const p = closeProgress(t, closeFrom.current)
        setProgress(p)
        if (p.pA < 0.002 && p.pB < 0.002) {
          exitOnce()
          return
        }
      }
      raf = requestAnimationFrame(step)
    }
    raf = requestAnimationFrame(step)
    // 動畫迴圈沒跑（例如視窗還在背景）時的計時後備：不會卡在半開或收不起來。
    const fallback = window.setTimeout(stage === 'opening' ? settle : exitOnce, stage === 'opening' ? OPEN_FALLBACK_MS : CLOSE_FALLBACK_MS)
    return () => {
      cancelAnimationFrame(raf)
      window.clearTimeout(fallback)
    }
  }, [animated, exitOnce, stage])

  const close = useCallback(() => {
    if (stageRef.current === 'closing') return
    closeFrom.current = progressRef.current
    // 有形狀時內容先消失、形狀倒放；沒有形狀時整個面板（含內容）一起 160 ms 淡出。
    if (animated) setContentVisible(false)
    setStage('closing')
  }, [animated])

  /** 收起並等到收起完成（蓋板視窗隱藏）。 */
  const closeAndWait = useCallback(
    () =>
      new Promise<void>((resolve) => {
        if (exited.current) {
          resolve()
          return
        }
        exitWaiters.current.push(resolve)
        close()
      }),
    [close],
  )

  const frame: MorphFrame = useMemo(() => morphFrame(notch, progress.pA, progress.pB, size), [notch, progress, size])
  const panel = useMemo(() => morphFrame(notch, 1, 1, size).content, [notch, size])

  // ---- 佇列（AC-DS-10）-------------------------------------------------------
  const goto = useCallback(
    (next: number) => {
      if (next < 0 || next >= session.length || next === index) return
      if (reduced) {
        setIndex(next)
        return
      }
      setFade(true)
      window.setTimeout(() => {
        setIndex(next)
        setFade(false)
        setEntering(true)
      }, 140)
    },
    [index, reduced, session.length],
  )
  const nextUnanswered = useCallback(() => {
    for (let k = 1; k <= session.length; k++) {
      const i = (index + k) % session.length
      if (!isTerminal(session[i].state)) return i
    }
    return -1
  }, [index, session])
  const advance = useCallback(() => {
    const next = nextUnanswered()
    if (next === -1) close()
    else goto(next)
  }, [close, goto, nextUnanswered])

  const updateQuestion = useCallback((updated: Question) => {
    setSession((current) => current.map((item) => (item.id === updated.id ? updated : item)))
    if (isTerminal(updated.state)) {
      // 面板裡送出（或退回）的：不是「被其他通道結案」，回執照常顯示。
      answeredHere.current.add(updated.id)
      setClosedElsewhere((previous) => {
        if (!previous.has(updated.id)) return previous
        const next = new Set(previous)
        next.delete(updated.id)
        return next
      })
    }
  }, [])

  // 保留回執只在面板正顯示這次在面板裡送出的回執時成立（AC-OP-07）；切去別題就不算。
  const holding = holdsReceipt(q, answeredHere.current)
  const holdingRef = useRef(onHoldingReceipt)
  holdingRef.current = onHoldingReceipt
  useEffect(() => holdingRef.current?.(holding), [holding])

  // 作答進行中（AC-DS-09、AC-OP-07(c)）：決策欄與題組同步回報，蓋板在結果回來前不收起。卸載時還在送出的不算。
  const answeringRef = useRef(onAnswering)
  answeringRef.current = onAnswering
  const inFlight = useRef(0)
  const reportSending = useCallback((sending: boolean) => {
    inFlight.current = Math.max(0, inFlight.current + (sending ? 1 : -1))
    answeringRef.current?.(inFlight.current > 0)
  }, [])
  useEffect(
    () => () => {
      if (inFlight.current > 0) answeringRef.current?.(false)
    },
    [],
  )

  // 原生 Esc 後備的 dismissal 範圍包含目前題目（AC-OP-06、AC-EC-09b）：蓋板裡每次換題都回報。
  const currentId = q?.id ?? null
  useEffect(() => {
    if (!inOverlay) return
    void ipc.overlayCurrent(currentId).catch(() => undefined)
  }, [currentId, inOverlay, ipc])

  // 稍後再問（AC-OP-06）：目前這題和所有已 forced 的題（含 session 中途加入的）都開始新週期，否則另一題留在 forced、
  // 視窗卻已隱藏。任何一題失敗就不收起、顯示錯誤，可以再試；全部成功才通知原生（Esc）並收起。回傳是否已讓開。
  // `laterDone` 記住這次已成功的題，重試只做還沒成功的；記過之後又到期的題照樣記（2026-10-09 QA 第 3 輪）。
  const [laterError, setLaterError] = useState<string | null>(null)
  const laterBusy = useRef(false)
  const laterDone = useRef<LaterDone>(new Map())
  const later = useCallback(
    async (ack?: () => void): Promise<boolean> => {
      if (!q || laterBusy.current) return false
      laterBusy.current = true
      setLaterError(null)
      laterDone.current = forgetDueAgain(laterDone.current, session)
      const ids = laterTargets(q, session)
      const result = await runLater({ ids, dismiss: (id) => ipc.dismissQuestion(id), ack, close: closeAndWait, done: laterDone.current })
      laterBusy.current = false
      if (!result.ok) setLaterError(result.message)
      return result.ok
    },
    [closeAndWait, ipc, q, session],
  )

  // 往外開的唯一入口（AC-DS-13）：蓋板裡先讓開（未答＝稍後再問、已答＝關閉），讓開完成才打開，目標才不會開在蓋板後面。
  const openLink = useMemo(
    () => openOutside({ inOverlay, stepAside: () => later(), open: (href) => void ipc.openDeepLink(href) }),
    [inOverlay, later, ipc],
  )

  // Esc（AC-DS-01、AC-OP-06）：從面板掛載那一刻起就有效，不等張開動畫；只有真的處理了才通知原生看門狗。
  // capture 階段先看到：焦點在行內輸入框裡（data-escape="local"）由它自己收起，這裡只回報網頁已處理；稍後再問等
  // dismissal 成功才通知原生（失敗就交給原生 2 秒後備）。
  const laterRef = useRef(later)
  laterRef.current = later
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      const target = event.target as HTMLElement | null
      const action = escapeAction({
        stage: stageRef.current,
        composing: event.isComposing || event.keyCode === 229,
        local: Boolean(target?.closest?.('[data-escape="local"]')),
        field: isTextEntry(target),
      })
      const ack = action.ack && inOverlay ? () => void ipc.overlayEscapeAck().catch(() => undefined) : undefined
      if (action.later) {
        event.preventDefault()
        void laterRef.current(ack)
      } else {
        ack?.()
      }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [inOverlay, ipc])

  // 面板是對話框（AC-DS-07）：Tab／Shift+Tab 在面板內循環，不會跑到面板後面。
  const panelRef = useRef<HTMLElement>(null)
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== 'Tab' || event.defaultPrevented) return
      const panel = panelRef.current
      if (!panel || stageRef.current !== 'open') return
      const items = focusables(panel)
      const active = document.activeElement as HTMLElement | null
      const index = active ? items.indexOf(active) : -1
      // 面板內其他瀏覽器自己會停的地方（可捲動的區塊）交給瀏覽器；只有焦點真的在面板外才拉回來。
      if (index < 0 && active && panel.contains(active)) return
      const target = wrapFocus(items.length, index, event.shiftKey)
      if (target === null) return
      event.preventDefault()
      items[target].focus()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  useEffect(() => {
    const previous = document.body.style.overflow
    document.body.style.overflow = 'hidden'
    return () => {
      document.body.style.overflow = previous
    }
  }, [])

  if (!q) return null
  // 題組（PRD §25）永遠兩欄：左欄跟著選中的項目顯示原文。
  const batch = q.kind === 'batch'
  const split = batch || hasEvidence(q)
  const allAnswered = session.every((item) => isTerminal(item.state))
  const isLast = nextUnanswered() === -1 || (allAnswered && index === session.length - 1)
  const panelStyle: CSSProperties = {
    left: panel.x,
    top: panel.y,
    width: panel.w,
    height: panel.h,
    // 沒有形狀時面板自己是黑底：貼齊瀏海所在的邊，只有朝螢幕內的角是圓角。
    ...(animated ? {} : { borderRadius: panelRadius(notch.anchor) }),
  }

  return (
    <OpenLinkContext.Provider value={openLink}>
    <div className="surface-stage" data-stage={stage} data-animated={animated ? 'true' : 'false'}>
      {animated && (
        <svg className="surface-shape" viewBox={`0 0 ${size.w} ${size.h}`} aria-hidden="true">
          {frame.paths.map((d, i) => (
            <path key={i} d={d} style={{ fill: 'var(--s-panel)', stroke: 'var(--s-hair-2)', strokeWidth: 1 }} />
          ))}
        </svg>
      )}
      <FlyingMark transition={transition ?? null} agent={q.source.agent} active={animated && stage === 'opening'} />
      <section
        ref={panelRef}
        className="surface-panel"
        data-solid={animated ? 'false' : 'true'}
        data-open={contentVisible ? 'true' : 'false'}
        style={panelStyle}
        aria-label="Kunsu 決策面板"
        role="dialog"
        aria-modal="true"
        // 形狀定案前不可操作（點不到、Tab 不進去）；Esc 由上面的 window 監聽處理，不受影響。
        inert={stage !== 'open'}
      >
        <SurfaceHeader
          question={q}
          index={index}
          session={session}
          closedElsewhere={closedElsewhere}
          onPrevious={() => goto(index - 1)}
          onNext={() => goto(index + 1)}
          onGoto={goto}
          onLater={() => void later()}
          laterText={laterHint(cycleMin)}
          review={review}
        />
        {/* 頁首下方的流內區塊（面板 grid 第 2 列）：把內容往下推，不蓋住「要你決定」與題目（AC-OP-06）。 */}
        {laterError && (
          <div role="alert" className="surface-alert surface-later-error">
            {laterError}
          </div>
        )}
        <div
          className="surface-body"
          data-layout={split ? 'split' : 'focus'}
          data-fading={fade ? 'true' : 'false'}
          data-enter={entering ? 'true' : undefined}
          key={q.id}
        >
          <DraftAssets ipc={ipc} question={q}>
            {batch ? (
              <BatchReview
                ipc={ipc}
                question={q}
                why={<WhyCard ipc={ipc} question={q} />}
                evidence={<EvidenceBlocks ipc={ipc} question={q} />}
                isLast={isLast}
                active={stage === 'open'}
                closedElsewhere={closedElsewhere.has(q.id)}
                onUpdated={updateQuestion}
                onNext={advance}
                onSending={reportSending}
              />
            ) : (
              <>
                {split && (
                  <div className="surface-evidence" aria-label="脈絡與證據">
                    <WhyCard ipc={ipc} question={q} />
                    <EvidenceBlocks ipc={ipc} question={q} />
                  </div>
                )}
                <DecisionColumn
                  ipc={ipc}
                  question={q}
                  withWhy={!split}
                  isLast={isLast}
                  active={stage === 'open'}
                  closedElsewhere={closedElsewhere.has(q.id)}
                  onUpdated={updateQuestion}
                  onNext={advance}
                  onSending={reportSending}
                />
              </>
            )}
          </DraftAssets>
        </div>
      </section>
    </div>
    </OpenLinkContext.Provider>
  )
}

/** 頁首（AC-DS-04/11/12/17）：來源身分、佇列、稍後再問（寫出後果「N 分鐘後再問」）；回看時改成「關閉」。不顯示倒數。 */
function SurfaceHeader({
  question,
  index,
  session,
  closedElsewhere,
  onPrevious,
  onNext,
  onGoto,
  onLater,
  laterText,
  review,
}: {
  question: Question
  index: number
  session: Question[]
  closedElsewhere: ReadonlySet<string>
  onPrevious: () => void
  onNext: () => void
  onGoto: (index: number) => void
  onLater: () => void
  laterText: string
  review: boolean
}) {
  const agentName = question.source.agent === 'claude' ? 'Claude' : question.source.agent === 'codex' ? 'Codex' : (question.product ?? 'AI Agent')
  const task = question.source.routine_name ?? question.product ?? '決策'
  const trigger = question.source.trigger === 'scheduled' || (!question.source.trigger && question.source.routine_id) ? '排程' : '互動'
  return (
    <header className="surface-head">
      <div className="surface-src">
        <span className="surface-mark" data-surface-mark="true">
          <AgentMarkIcon agent={question.source.agent} size={26} />
        </span>
        <span className="surface-src__name">{agentName}</span>
        <span className="surface-chip">
          {task} · {trigger}
        </span>
        <span className="surface-muted tabular">{fmtHm(question.created_at)} 提出</span>
      </div>
      {session.length > 1 && (
        <nav className="surface-queue" aria-label="題目佇列">
          <button type="button" className="surface-icon-btn" aria-label="上一題" disabled={index === 0} onClick={onPrevious}>
            ‹
          </button>
          <span className="tabular">
            第 {index + 1} 題，共 {session.length} 題
          </span>
          <span className="surface-dots">
            {session.map((item, i) => (
              <button
                key={item.id}
                type="button"
                aria-label={dotLabel(i, item, closedElsewhere.has(item.id))}
                data-on={i === index ? 'true' : 'false'}
                data-done={isTerminal(item.state) ? 'true' : 'false'}
                onClick={() => onGoto(i)}
              />
            ))}
          </span>
          <button type="button" className="surface-icon-btn" aria-label="下一題" disabled={index >= session.length - 1} onClick={onNext}>
            ›
          </button>
        </nav>
      )}
      <div className="surface-head__right">
        {review ? (
          <button type="button" className="surface-later" onClick={onLater} aria-keyshortcuts="Escape">
            關閉 <kbd>Esc</kbd>
          </button>
        ) : (
          <button type="button" className="surface-later" onClick={onLater} aria-keyshortcuts="Escape">
            稍後再問 <span className="surface-later__hint">{laterText}</span> <kbd>Esc</kbd>
          </button>
        )}
      </div>
    </header>
  )
}

/** 左欄證據：AI 畫布（沙盒）＋Markdown 內文（表格、圖片、清單）＋結構化證據行。 */
function EvidenceBlocks({ ipc, question }: { ipc: KunsuIpc; question: Question }) {
  const openLink = useOpenLink(ipc)
  const body = evidenceMarkdown(question)
  const evidence = useEvidence(ipc, question)
  const assets = evidence.status === 'ready' ? evidence.bundle.assets : undefined
  return (
    <>
      {evidence.status === 'ready' && evidence.bundle.canvas && (
        <CanvasFrame
          question={question}
          bundle={evidence.bundle}
          ipc={ipc}
          onFailure={(reason) => void ipc.canvasFailure(question.id, question.draft?.revision ?? 0, reason).catch(() => {})}
        />
      )}
      {evidence.status === 'ready' && evidence.bundle.canvas_disabled && (
        <section className="surface-card surface-canvas" data-phase="failed">
          <div className="surface-canvas__caption">AI 提供的資料</div>
          <p className="surface-canvas__failed" role="status">
            圖表載入失敗：{evidence.bundle.canvas_disabled}
          </p>
        </section>
      )}
      {evidence.status === 'error' && question.draft?.canvas && (
        <section className="surface-card surface-canvas" data-phase="failed">
          <div className="surface-canvas__caption">AI 提供的資料</div>
          <p className="surface-canvas__failed" role="status">
            圖表載入失敗：{evidence.message}
          </p>
        </section>
      )}
      {body.trim() !== '' && (
        <section className="surface-card surface-md" aria-label="證據">
          <RichContent assets={assets} onOpenLink={openLink}>
            {body}
          </RichContent>
        </section>
      )}
      {question.evidence.length > 0 && (
        <section className="surface-card" aria-label="證據摘要">
          <ul className="surface-evidence-lines">
            {question.evidence.map((line, i) => (
              <li key={i}>
                <RichContent inline onOpenLink={openLink}>
                  {line.text}
                </RichContent>
                <span className="surface-chip surface-chip--source" title={line.source}>
                  {line.source || '缺身世'}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
    </>
  )
}

/** 來源圖示從瀏海飛到頁首（共享元素）。 */
function FlyingMark({ transition, agent, active }: { transition: OverlayTransition | null; agent: Question['source']['agent']; active: boolean }) {
  const ref = useRef<HTMLDivElement>(null)
  const [target, setTarget] = useState<{ x: number; y: number } | null>(null)
  const [done, setDone] = useState(false)
  useLayoutEffect(() => {
    if (!active) return
    const slot = document.querySelector<HTMLElement>('[data-surface-mark="true"]')
    if (!slot) return
    const rect = slot.getBoundingClientRect()
    slot.style.visibility = 'hidden'
    const id = window.requestAnimationFrame(() =>
      setTarget({
        x: rect.left + rect.width / 2,
        y: rect.top + rect.height / 2,
      }),
    )
    const timer = window.setTimeout(() => {
      slot.style.visibility = 'visible'
      setDone(true)
    }, 560)
    return () => {
      window.cancelAnimationFrame(id)
      window.clearTimeout(timer)
      slot.style.visibility = 'visible'
    }
  }, [active])
  if (!transition || done || !active) return null
  const from = { x: transition.origin_x, y: transition.origin_y }
  const at = target ?? from
  const size = target ? 26 : Math.max(14, Math.min(26, transition.origin_size || 18))
  return (
    <div
      ref={ref}
      className="surface-fly"
      style={{
        transform: `translate(${at.x - size / 2}px, ${at.y - size / 2}px)`,
        width: size,
        height: size,
      }}
      aria-hidden="true"
    >
      <AgentMarkIcon agent={agent} size={size} />
    </div>
  )
}
